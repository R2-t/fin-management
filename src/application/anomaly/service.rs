use base64::Engine;
use chrono::{DateTime, SecondsFormat, Utc};
use tracing::instrument;
use uuid::Uuid;

use crate::application::error::ApplicationError;
use crate::application::inbound::repositories::{AnomalyRepository, Result};
use crate::model::anomaly::{Anomaly, AnomalyStatus};

const DEFAULT_LIMIT: i64 = 50;
const MAX_LIMIT: i64 = 200;
const MAX_REASON_CHARS: usize = 500;

#[derive(Debug)]
pub struct AnomalyPage {
    pub items: Vec<Anomaly>,
    pub next_cursor: Option<String>,
}

/// Reads and dismisses findings.
///
/// Detectors (4.2–4.4) call [`AnomalyRepository::create`] directly; this service
/// is the user-facing side: a page, a dismissal, and its undo.
pub struct AnomalyService<R>
where
    R: AnomalyRepository,
{
    anomalies: R,
}

impl<R> AnomalyService<R>
where
    R: AnomalyRepository,
{
    pub fn new(anomalies: R) -> Self {
        Self { anomalies }
    }

    /// Open findings by default, newest first. A page of `limit` items carries a
    /// cursor for the next page; the last page carries none.
    #[instrument(skip_all, err(level = "warn"))]
    pub async fn list(
        &self,
        user_id: Uuid,
        kind: Option<String>,
        state: Option<AnomalyStatus>,
        limit: Option<i64>,
        cursor: Option<String>,
    ) -> Result<AnomalyPage> {
        let limit = limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
        let before = cursor.as_deref().map(decode_cursor).transpose()?;
        let kind = kind.as_deref().filter(|k| !k.trim().is_empty());

        let mut items = self
            .anomalies
            .list(
                user_id,
                kind,
                Some(state.unwrap_or(AnomalyStatus::Open)),
                limit + 1,
                before,
            )
            .await?;

        let next_cursor = if items.len() as i64 > limit {
            items.truncate(limit as usize);
            items.last().map(encode_cursor)
        } else {
            None
        };

        Ok(AnomalyPage { items, next_cursor })
    }

    #[instrument(skip_all, err(level = "warn"), fields(anomaly_id = %id))]
    pub async fn dismiss(
        &self,
        user_id: Uuid,
        id: Uuid,
        reason: Option<String>,
    ) -> Result<Anomaly> {
        self.anomalies
            .dismiss(user_id, id, validated_reason(reason)?)
            .await
    }

    #[instrument(skip_all, err(level = "warn"), fields(anomaly_id = %id))]
    pub async fn undismiss(&self, user_id: Uuid, id: Uuid) -> Result<Anomaly> {
        self.anomalies.undismiss(user_id, id).await
    }
}

/// A reason is a short label the user reads, not free storage.
fn validated_reason(reason: Option<String>) -> Result<Option<String>> {
    let reason = reason
        .map(|r| r.trim().to_string())
        .filter(|r| !r.is_empty());
    if let Some(reason) = &reason {
        if reason.chars().count() > MAX_REASON_CHARS {
            return Err(ApplicationError::Validation(format!(
                "a dismissal reason cannot be longer than {MAX_REASON_CHARS} characters"
            )));
        }
    }
    Ok(reason)
}

fn encode_cursor(anomaly: &Anomaly) -> String {
    let at = anomaly
        .detected_at
        .to_rfc3339_opts(SecondsFormat::Nanos, true);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(format!("{at}|{}", anomaly.id))
}

fn decode_cursor(cursor: &str) -> Result<(DateTime<Utc>, Uuid)> {
    let invalid = || ApplicationError::Validation("invalid cursor".into());
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(cursor)
        .map_err(|_| invalid())?;
    let text = String::from_utf8(bytes).map_err(|_| invalid())?;
    let (at, id) = text.split_once('|').ok_or_else(invalid)?;
    let at = DateTime::parse_from_rfc3339(at)
        .map_err(|_| invalid())?
        .with_timezone(&Utc);
    let id = Uuid::parse_str(id).map_err(|_| invalid())?;
    Ok((at, id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::anomaly::Anomaly;

    fn finding(at: &str) -> Anomaly {
        Anomaly {
            id: Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(),
            user_id: Uuid::nil(),
            kind: "duplicate".into(),
            subject_type: "purchase".into(),
            subject_id: None,
            title: "Two of the same".into(),
            detail: serde_json::json!({"amount": 1200}),
            severity: "notice".into(),
            detected_at: DateTime::parse_from_rfc3339(at).unwrap().with_timezone(&Utc),
            key: "1200|2026-01-01".into(),
            dismissed_at: None,
            dismissed_reason: None,
        }
    }

    #[test]
    fn a_long_reason_is_refused() {
        assert!(matches!(
            validated_reason(Some("x".repeat(MAX_REASON_CHARS + 1))),
            Err(ApplicationError::Validation(_))
        ));
        assert!(validated_reason(Some("x".repeat(MAX_REASON_CHARS))).is_ok());
    }

    #[test]
    fn a_blank_reason_is_stored_as_none() {
        assert_eq!(validated_reason(Some("   ".into())).unwrap(), None);
    }

    #[test]
    fn a_cursor_round_trips_its_pair() {
        let anomaly = finding("2026-01-01T10:00:00.123456Z");
        let (at, id) = decode_cursor(&encode_cursor(&anomaly)).unwrap();
        assert_eq!(at, anomaly.detected_at);
        assert_eq!(id, anomaly.id);
    }

    #[test]
    fn a_malformed_cursor_is_a_validation_error() {
        assert!(matches!(
            decode_cursor("not-a-cursor"),
            Err(ApplicationError::Validation(_))
        ));
    }
}
