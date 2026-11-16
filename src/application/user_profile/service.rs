use std::str::FromStr;

use chrono::Utc;
use tracing::instrument;
use uuid::Uuid;

use crate::application::{
    currency::{normalize_currency, DEFAULT_CURRENCY},
    error::ApplicationError,
    inbound::repositories::{UserProfileRepository, UserProfileUpdate, UserProfileValues},
};
use crate::model::user_profile::UserProfile;

/// Timezone assumed until the user sets one.
pub const DEFAULT_TIMEZONE: &str = "UTC";

pub struct UserProfileService<P>
where
    P: UserProfileRepository,
{
    profiles: P,
}

impl<P> UserProfileService<P>
where
    P: UserProfileRepository,
{
    pub fn new(profiles: P) -> Self {
        Self { profiles }
    }

    /// Returns the stored profile, or a non-persisted default for a user that
    /// has never saved one.
    #[instrument(skip_all, err(level = "warn"), fields(user_id = %user_id))]
    pub async fn get(&self, user_id: Uuid) -> Result<UserProfile, ApplicationError> {
        match self.profiles.get(user_id).await? {
            Some(profile) => Ok(profile),
            None => Ok(default_profile(user_id)),
        }
    }

    /// Partial update: any argument left as `None` keeps its current value.
    /// The effective values are validated before the single upsert.
    #[instrument(skip_all, err(level = "warn"), fields(user_id = %user_id))]
    pub async fn update(
        &self,
        user_id: Uuid,
        update: UserProfileUpdate,
    ) -> Result<UserProfile, ApplicationError> {
        let current = self.get(user_id).await?;

        let timezone = match update.timezone {
            Some(tz) => validate_timezone(&tz)?,
            None => current.timezone,
        };
        let base_currency = match update.base_currency {
            Some(c) => normalize_currency(&c)?,
            None => current.base_currency,
        };
        let locale = match update.locale {
            Some(l) => normalize_locale(&l)?,
            // Cannot clear a locale back to NULL through this endpoint.
            None => current.locale,
        };

        self.profiles
            .upsert(
                user_id,
                UserProfileValues {
                    timezone,
                    base_currency,
                    locale,
                },
            )
            .await
    }
}

fn default_profile(user_id: Uuid) -> UserProfile {
    let now = Utc::now();
    UserProfile {
        user_id,
        timezone: DEFAULT_TIMEZONE.to_string(),
        base_currency: DEFAULT_CURRENCY.to_string(),
        locale: None,
        created_at: now,
        updated_at: now,
    }
}

/// Validates an IANA timezone against the embedded tz database and returns the
/// canonical name. Lookup is case-insensitive so `europe/madrid` behaves like
/// PostgreSQL, which accepts timezone names case-insensitively.
pub fn validate_timezone(timezone: &str) -> Result<String, ApplicationError> {
    let raw = timezone.trim();

    if let Ok(parsed) = chrono_tz::Tz::from_str(raw) {
        return Ok(parsed.to_string());
    }
    if let Some(found) = chrono_tz::TZ_VARIANTS
        .iter()
        .find(|tz| tz.name().eq_ignore_ascii_case(raw))
    {
        return Ok(found.to_string());
    }

    Err(ApplicationError::Validation(
        "timezone must be a valid IANA name, e.g. 'Europe/Madrid'".into(),
    ))
}

/// Parses a stored IANA timezone.
///
/// The value is validated on write, so a failure here means a corrupt profile
/// rather than bad user input -- reported as an internal error rather than
/// silently falling back to UTC, which would put every period boundary and
/// every day bucket on the wrong day without saying so.
pub fn parse_timezone(timezone: &str) -> Result<chrono_tz::Tz, ApplicationError> {
    let raw = timezone.trim();

    if let Ok(parsed) = chrono_tz::Tz::from_str(raw) {
        return Ok(parsed);
    }
    chrono_tz::TZ_VARIANTS
        .iter()
        .find(|tz| tz.name().eq_ignore_ascii_case(raw))
        .copied()
        .ok_or_else(|| {
            ApplicationError::Internal(format!(
                "stored timezone {timezone:?} is not a valid IANA name"
            ))
        })
}

/// Today's date in a timezone.
///
/// Every period boundary and every day bucket in this app belongs to the
/// user's calendar, not the server's (D25). A budget period, a burn-rate day
/// and a runway day are all read from here so they agree with each other.
pub fn today_in(timezone: &str) -> Result<chrono::NaiveDate, ApplicationError> {
    let tz = parse_timezone(timezone)?;
    Ok(chrono::Utc::now().with_timezone(&tz).date_naive())
}

/// The UTC instant at which a calendar day begins in a timezone.
///
/// A period that starts on 1 March in Europe/Madrid starts at 23:00 UTC on
/// 28 February, not at UTC midnight. Using UTC midnight for a local date would
/// drop the first hour of every day from the period it belongs to.
///
/// `shortcut:` a zone whose clocks jump past midnight has no such instant, and
/// the fallback is UTC midnight -- a few hours out, once a year, in a handful
/// of zones. Every zone with a normal midnight is exact.
pub fn day_start(
    date: chrono::NaiveDate,
    timezone: &str,
) -> Result<chrono::DateTime<chrono::Utc>, ApplicationError> {
    let tz = parse_timezone(timezone)?;
    let local = date.and_time(chrono::NaiveTime::MIN).and_local_timezone(tz);
    Ok(local
        .earliest()
        .map(|dt| dt.to_utc())
        .unwrap_or_else(|| date.and_time(chrono::NaiveTime::MIN).and_utc()))
}

fn normalize_locale(locale: &str) -> Result<Option<String>, ApplicationError> {
    let locale = locale.trim();
    if locale.is_empty() {
        return Err(ApplicationError::Validation(
            "locale cannot be empty".into(),
        ));
    }
    if locale.len() > 35
        || !locale
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(ApplicationError::Validation(
            "locale must be a BCP-47 tag, e.g. 'en-US'".into(),
        ));
    }
    Ok(Some(locale.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::inbound::repositories::Result;
    use std::sync::Mutex;

    #[derive(Default)]
    struct MockProfiles {
        stored: Mutex<Option<UserProfile>>,
    }

    impl UserProfileRepository for MockProfiles {
        async fn get(&self, _user_id: Uuid) -> Result<Option<UserProfile>> {
            Ok(self.stored.lock().unwrap().clone())
        }
        async fn upsert(
            &self,
            user_id: Uuid,
            values: UserProfileValues,
        ) -> Result<UserProfile> {
            let now = Utc::now();
            let profile = UserProfile {
                user_id,
                timezone: values.timezone,
                base_currency: values.base_currency,
                locale: values.locale,
                created_at: now,
                updated_at: now,
            };
            *self.stored.lock().unwrap() = Some(profile.clone());
            Ok(profile)
        }
    }

    fn service() -> (UserProfileService<MockProfiles>, Uuid) {
        (
            UserProfileService::new(MockProfiles::default()),
            Uuid::new_v4(),
        )
    }

    #[test]
    fn accepts_iana_names_and_canonicalises_case() {
        assert_eq!(validate_timezone("Europe/Madrid").unwrap(), "Europe/Madrid");
        assert_eq!(validate_timezone("  UTC ").unwrap(), "UTC");
        assert_eq!(validate_timezone("europe/madrid").unwrap(), "Europe/Madrid");
    }

    #[test]
    fn rejects_unknown_and_offset_timezones() {
        for bad in ["Not/AZone", "Mars/Olympus", "", "+05:30"] {
            assert!(
                matches!(validate_timezone(bad), Err(ApplicationError::Validation(_))),
                "{bad:?} should be rejected"
            );
        }
    }

    #[tokio::test]
    async fn unknown_user_gets_defaults_without_persisting() {
        let (svc, user) = service();
        let profile = svc.get(user).await.unwrap();
        assert_eq!(profile.timezone, "UTC");
        assert_eq!(profile.base_currency, "USD");
        assert!(profile.locale.is_none());
        assert!(svc.profiles.stored.lock().unwrap().is_none());
    }

    #[tokio::test]
    async fn partial_update_keeps_untouched_fields() {
        let (svc, user) = service();
        svc.update(
            user,
            UserProfileUpdate {
                timezone: Some("Europe/Madrid".into()),
                base_currency: Some("eur".into()),
                locale: Some("es-ES".into()),
            },
        )
        .await
        .unwrap();

        let updated = svc.update(user, UserProfileUpdate::default()).await.unwrap();
        assert_eq!(updated.timezone, "Europe/Madrid");
        assert_eq!(updated.base_currency, "EUR");
        assert_eq!(updated.locale.as_deref(), Some("es-ES"));

        let only_locale = svc
            .update(
                user,
                UserProfileUpdate {
                    locale: Some("en-GB".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(only_locale.timezone, "Europe/Madrid");
        assert_eq!(only_locale.base_currency, "EUR");
        assert_eq!(only_locale.locale.as_deref(), Some("en-GB"));
    }

    #[tokio::test]
    async fn rejects_invalid_timezone_and_currency() {
        let (svc, user) = service();
        assert!(svc
            .update(
                user,
                UserProfileUpdate {
                    timezone: Some("Nowhere".into()),
                    ..Default::default()
                },
            )
            .await
            .is_err());
        assert!(svc
            .update(
                user,
                UserProfileUpdate {
                    base_currency: Some("EURO".into()),
                    ..Default::default()
                },
            )
            .await
            .is_err());
        assert!(svc
            .update(
                user,
                UserProfileUpdate {
                    locale: Some("  ".into()),
                    ..Default::default()
                },
            )
            .await
            .is_err());
    }

    /// A period that starts on 1 March in Madrid starts at 23:00 UTC on
    /// 28 February. Using UTC midnight for a local date would drop the first
    /// hour of the day from the period it belongs to.
    #[test]
    fn day_start_is_local_midnight_not_utc_midnight() {
        let date = chrono::NaiveDate::from_ymd_opt(2026, 3, 1).unwrap();

        let madrid = day_start(date, "Europe/Madrid").unwrap();
        assert_eq!(
            madrid,
            chrono::NaiveDate::from_ymd_opt(2026, 2, 28)
                .unwrap()
                .and_hms_opt(23, 0, 0)
                .unwrap()
                .and_utc(),
            "Madrid is UTC+1 in March"
        );

        let utc = day_start(date, "UTC").unwrap();
        assert_eq!(utc, date.and_hms_opt(0, 0, 0).unwrap().and_utc());
    }

    /// A timezone stored with different casing still parses, because the
    /// profile accepts it case-insensitively on write.
    #[test]
    fn parse_timezone_accepts_the_name_as_stored() {
        assert!(parse_timezone("Europe/Madrid").is_ok());
        assert!(parse_timezone("europe/madrid").is_ok());
        assert!(parse_timezone("  UTC  ").is_ok());
    }

    /// A stored timezone that is not a real IANA name is reported rather than
    /// silently falling back to UTC, which would put every period boundary on
    /// the wrong day without saying so.
    #[test]
    fn parse_timezone_rejects_a_corrupt_value() {
        assert!(matches!(
            parse_timezone("Mars/Olympus"),
            Err(ApplicationError::Internal(_))
        ));
    }

    #[test]
    fn today_in_is_a_date_for_a_real_zone() {
        assert!(today_in("Europe/Madrid").is_ok());
        assert!(today_in("UTC").is_ok());
    }
}
