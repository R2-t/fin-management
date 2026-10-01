use crate::application::error::ApplicationError;

/// Fallback reporting/holding currency used when a caller does not supply one.
///
/// TODO(0.3): default from `user_profile.base_currency` once that table exists.
pub const DEFAULT_CURRENCY: &str = "USD";

const CURRENCY_CODE_LEN: usize = 3;

/// Normalises and validates an ISO-4217 alpha code (`usd` -> `USD`).
///
/// Only the shape is validated (three ASCII letters); the code is not checked
/// against the ISO-4217 list, which would need a reference table.
pub fn normalize_currency(raw: &str) -> Result<String, ApplicationError> {
    let currency = raw.trim().to_uppercase();
    if currency.len() != CURRENCY_CODE_LEN || !currency.chars().all(|c| c.is_ascii_alphabetic()) {
        return Err(ApplicationError::Validation(
            "currency must be a 3-letter ISO 4217 code".into(),
        ));
    }
    Ok(currency)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_case_and_whitespace() {
        assert_eq!(normalize_currency(" usd ").unwrap(), "USD");
        assert_eq!(normalize_currency("eur").unwrap(), "EUR");
    }

    #[test]
    fn rejects_wrong_shape() {
        for bad in ["", "US", "USDD", "12", "U$D", "  "] {
            assert!(
                matches!(
                    normalize_currency(bad),
                    Err(ApplicationError::Validation(_))
                ),
                "{bad:?} should be rejected"
            );
        }
    }
}
