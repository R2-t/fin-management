use crate::application::error::ApplicationError;

/// Fallback reporting/holding currency used when a caller does not supply one.
///
/// TODO(0.3): default from `user_profile.base_currency` once that table exists.
pub const DEFAULT_CURRENCY: &str = "USD";

const CURRENCY_CODE_LEN: usize = 3;

/// Scale used for exchange rates: `rate_scaled = round(rate * RATE_SCALE)`.
pub const RATE_SCALE: i64 = 1_000_000_000;

/// Converts a decimal rate (as supplied by a provider or a client override)
/// into its scaled integer form. Rejects non-finite and non-positive rates.
pub fn rate_to_scaled(rate: f64) -> Result<i64, ApplicationError> {
    if !rate.is_finite() || rate <= 0.0 {
        return Err(ApplicationError::Validation(
            "exchange rate must be a positive number".into(),
        ));
    }
    Ok((rate * RATE_SCALE as f64).round() as i64)
}

/// Display-only conversion. Never use the result for storage or aggregation.
pub fn scaled_to_rate(rate_scaled: i64) -> f64 {
    rate_scaled as f64 / RATE_SCALE as f64
}

/// Converts a minor-unit amount with a scaled rate, rounding half away from
/// zero. Computed in `i128` because `i64 * i64` can overflow.
pub fn convert_minor(amount_minor: i64, rate_scaled: i64) -> i64 {
    let numerator = amount_minor as i128 * rate_scaled as i128;
    let scale = RATE_SCALE as i128;
    let half = scale / 2;
    let rounded = if numerator >= 0 {
        (numerator + half) / scale
    } else {
        (numerator - half) / scale
    };
    rounded as i64
}

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
    fn converts_minor_amounts_half_away_from_zero() {
        let rate = 850_000_000; // 0.85
        assert_eq!(convert_minor(1050, rate), 893); // 892.5 -> 893
        assert_eq!(convert_minor(1000, rate), 850);
        assert_eq!(convert_minor(1, rate), 1);
        assert_eq!(convert_minor(0, rate), 0);
        assert_eq!(convert_minor(-1050, rate), -893);
        assert_eq!(convert_minor(1050, RATE_SCALE), 1050);
    }

    #[test]
    fn round_trips_rates_and_rejects_bad_ones() {
        assert_eq!(rate_to_scaled(0.85).unwrap(), 850_000_000);
        assert_eq!(scaled_to_rate(850_000_000), 0.85);
        for bad in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert!(matches!(
                rate_to_scaled(bad),
                Err(ApplicationError::Validation(_))
            ));
        }
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
