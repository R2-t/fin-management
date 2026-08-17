use std::sync::Arc;

use pasetors::{
    claims::ClaimsValidationRules,
    footer::Footer,
    keys::SymmetricKey,
    local,
    token::UntrustedToken,
    version4::V4,
    Local,
};

use super::{
    key_provider::KeyProvider,
    verifier::{TokenVerifier, VerifyError},
    AuthPrincipal,
};

/// PASETO (v4.local) token verifier.
///
/// The token's footer carries a `kid` (key id). The key id is used to fetch the
/// symmetric key from an external [`KeyProvider`], which owns key material and
/// performs key rotation. This keeps this service decoupled from key management.
pub struct PasetoVerifier<K> {
    key_provider: Arc<K>,
}

impl<K> PasetoVerifier<K>
where
    K: KeyProvider,
{
    pub fn new(key_provider: Arc<K>) -> Self {
        Self { key_provider }
    }
}

#[async_trait::async_trait]
impl<K> TokenVerifier for PasetoVerifier<K>
where
    K: KeyProvider,
{
    async fn verify(&self, token: &str) -> Result<AuthPrincipal, VerifyError> {
        // 1. Parse the untrusted token (structural validation).
        let untrusted = UntrustedToken::<Local, V4>::try_from(token)
            .map_err(|_| VerifyError::InvalidToken)?;

        // 2. Read the `kid` from the footer.
        let mut footer = Footer::new();
        footer
            .parse_bytes(untrusted.untrusted_footer())
            .map_err(|_| VerifyError::InvalidToken)?;

        let key_id = footer
            .get_claim("kid")
            .and_then(|v| v.as_str())
            .ok_or(VerifyError::InvalidToken)?;

        // 3. Fetch the key from the external key service (handles rotation).
        let resolved = self
            .key_provider
            .get_key(key_id)
            .await
            .map_err(|e| match e {
                super::key_provider::KeyProviderError::NotFound(_) => VerifyError::KeyNotFound,
                other => VerifyError::KeyProvider(other),
            })?;

        let key = SymmetricKey::<V4>::from(&resolved.bytes)
            .map_err(|_| VerifyError::Internal("invalid key material".into()))?;

        // 4. Decrypt and validate claims.
        let validation_rules = ClaimsValidationRules::new();
        let trusted = local::decrypt(
            &key,
            &untrusted,
            &validation_rules,
            Some(&footer),
            None,
        )
        .map_err(|_| VerifyError::InvalidToken)?;

        // 5. Extract the user id from the verified claims.
        let user_id = trusted
            .payload_claims()
            .and_then(|c| c.get_claim("user_id"))
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<uuid::Uuid>().ok())
            .ok_or(VerifyError::InvalidToken)?;

        Ok(AuthPrincipal { user_id })
    }
}
