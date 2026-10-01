use std::str::FromStr;

use chrono::Utc;
use tracing::instrument;
use uuid::Uuid;

use crate::application::{
    currency::{normalize_currency, DEFAULT_CURRENCY},
    error::ApplicationError,
    inbound::repositories::UserProfileRepository,
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
        timezone: Option<String>,
        base_currency: Option<String>,
        locale: Option<String>,
    ) -> Result<UserProfile, ApplicationError> {
        let current = self.get(user_id).await?;

        let timezone = match timezone {
            Some(tz) => validate_timezone(&tz)?,
            None => current.timezone,
        };
        let base_currency = match base_currency {
            Some(c) => normalize_currency(&c)?,
            None => current.base_currency,
        };
        let locale = match locale {
            Some(l) => normalize_locale(&l)?,
            // Cannot clear a locale back to NULL through this endpoint.
            None => current.locale,
        };

        self.profiles
            .upsert(user_id, &timezone, &base_currency, locale.as_deref())
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
            timezone: &str,
            base_currency: &str,
            locale: Option<&str>,
        ) -> Result<UserProfile> {
            let now = Utc::now();
            let profile = UserProfile {
                user_id,
                timezone: timezone.to_string(),
                base_currency: base_currency.to_string(),
                locale: locale.map(str::to_string),
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
            Some("Europe/Madrid".into()),
            Some("eur".into()),
            Some("es-ES".into()),
        )
        .await
        .unwrap();

        let updated = svc.update(user, None, None, None).await.unwrap();
        assert_eq!(updated.timezone, "Europe/Madrid");
        assert_eq!(updated.base_currency, "EUR");
        assert_eq!(updated.locale.as_deref(), Some("es-ES"));

        let only_locale = svc
            .update(user, None, None, Some("en-GB".into()))
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
            .update(user, Some("Nowhere".into()), None, None)
            .await
            .is_err());
        assert!(svc
            .update(user, None, Some("EURO".into()), None)
            .await
            .is_err());
        assert!(svc
            .update(user, None, None, Some("  ".into()))
            .await
            .is_err());
    }
}
