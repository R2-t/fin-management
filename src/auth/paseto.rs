use std::sync::Arc;

use chrono::{DateTime, Utc};
use pasetors::{
    claims::{Claims, ClaimsValidationRules},
    footer::Footer,
    keys::SymmetricKey,
    local,
    token::{TrustedToken, UntrustedToken},
    version4::V4,
    Local,
};
use uuid::Uuid;

use crate::config::auth::AuthConfig;

use super::{
    key_provider::{KeyProvider, KeyProviderError},
    verifier::{TokenVerifier, VerifyError},
    AuthPrincipal,
};

/// Claims a token must carry, decoded.
#[derive(Debug)]
pub struct VerifiedClaims {
    pub claims: Claims,
    pub user_id: Uuid,
}

/// Verifies a PASETO v4.local token and returns its claims.
///
/// Free function rather than a method: both the bearer verifier (a user's
/// machine token) and the service-token path (analytics calls and job rows)
/// need exactly this, with different rules about which claims they require.
pub async fn decrypt_and_validate<K: KeyProvider>(
    key_provider: &K,
    token: &str,
    config: &AuthConfig,
) -> Result<VerifiedClaims, VerifyError> {
    // 1. Structural validation. The token is still untrusted.
    let untrusted =
        UntrustedToken::<Local, V4>::try_from(token).map_err(|_| VerifyError::InvalidToken)?;

    // 2. The `kid` travels in the footer, so a rotated key is picked up without
    // a redeploy.
    let mut footer = Footer::new();
    footer
        .parse_bytes(untrusted.untrusted_footer())
        .map_err(|_| VerifyError::InvalidToken)?;
    let key_id = footer
        .get_claim("kid")
        .and_then(|v| v.as_str())
        .ok_or(VerifyError::InvalidToken)?;

    let resolved = key_provider.get_key(key_id).await.map_err(|e| match e {
        KeyProviderError::NotFound(_) => VerifyError::KeyNotFound,
        other => VerifyError::KeyProvider(other),
    })?;

    let key = SymmetricKey::<V4>::from(&resolved.bytes)
        .map_err(|_| VerifyError::Internal("invalid key material".into()))?;

    // 3. Explicit rules, not `::new()` left implicit.
    //
    // `new()` already refuses a token with no `exp` (`allow_non_expiring` is
    // off by default) and requires `iat` and `nbf`, which is the expiry rule
    // this service wants. What it does *not* check is who the token is for and
    // who issued it -- so `iss` and `aud` are pinned here.
    let mut rules = ClaimsValidationRules::new();
    rules.validate_issuer_with(&config.paseto_issuer);
    rules.validate_audience_with(&config.paseto_audience);

    let trusted: TrustedToken = local::decrypt(&key, &untrusted, &rules, Some(&footer), None)
        .map_err(|_| VerifyError::InvalidToken)?;

    let claims = trusted
        .payload_claims()
        .cloned()
        .ok_or(VerifyError::InvalidToken)?;

    let user_id = claims
        .get_claim("user_id")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse::<Uuid>().ok())
        .ok_or(VerifyError::InvalidToken)?;

    Ok(VerifiedClaims { claims, user_id })
}

/// PASETO (v4.local) token verifier for machine clients.
///
/// The token's footer carries a `kid`; the key comes from an external
/// [`KeyProvider`], which owns key material and performs rotation.
pub struct PasetoVerifier<K> {
    key_provider: Arc<K>,
    config: AuthConfig,
}

impl<K> PasetoVerifier<K>
where
    K: KeyProvider,
{
    pub fn new(key_provider: Arc<K>, config: AuthConfig) -> Self {
        Self {
            key_provider,
            config,
        }
    }
}

#[async_trait::async_trait]
impl<K> TokenVerifier for PasetoVerifier<K>
where
    K: KeyProvider,
{
    async fn verify(&self, token: &str) -> Result<AuthPrincipal, VerifyError> {
        let verified = decrypt_and_validate(self.key_provider.as_ref(), token, &self.config).await?;
        Ok(AuthPrincipal {
            user_id: verified.user_id,
        })
    }
}

/// Mints the tokens this service issues: service-to-service calls and job rows.
///
/// Service tokens use their **own key** (D76). Sharing the human bearer key
/// would let a compromised sidecar mint a principal for any user, which is the
/// opposite of what the token is for.
pub struct ServiceTokenMinter<K> {
    key_provider: Arc<K>,
    key_id: String,
    config: AuthConfig,
}

impl<K> ServiceTokenMinter<K>
where
    K: KeyProvider,
{
    pub fn new(key_provider: Arc<K>, key_id: String, config: AuthConfig) -> Self {
        Self {
            key_provider,
            key_id,
            config,
        }
    }

    async fn mint_with(
        &self,
        user_id: Uuid,
        scope: &str,
        ttl_seconds: i64,
        now: DateTime<Utc>,
    ) -> Result<String, VerifyError> {
        let resolved = self
            .key_provider
            .get_key(&self.key_id)
            .await
            .map_err(|e| match e {
                KeyProviderError::NotFound(_) => VerifyError::KeyNotFound,
                other => VerifyError::KeyProvider(other),
            })?;
        let key = SymmetricKey::<V4>::from(&resolved.bytes)
            .map_err(|_| VerifyError::Internal("invalid key material".into()))?;

        let mut claims = Claims::new()
            .map_err(|e| VerifyError::Internal(e.to_string()))?;
        claims
            .issuer(&self.config.paseto_issuer)
            .map_err(|e| VerifyError::Internal(e.to_string()))?;
        claims
            .audience(&self.config.paseto_audience)
            .map_err(|e| VerifyError::Internal(e.to_string()))?;
        claims
            .subject("service:api")
            .map_err(|e| VerifyError::Internal(e.to_string()))?;
        let exp = now + chrono::Duration::seconds(ttl_seconds);
        claims
            .expiration(&exp.to_rfc3339())
            .map_err(|e| VerifyError::Internal(e.to_string()))?;
        claims
            .issued_at(&now.to_rfc3339())
            .map_err(|e| VerifyError::Internal(e.to_string()))?;
        claims
            .not_before(&now.to_rfc3339())
            .map_err(|e| VerifyError::Internal(e.to_string()))?;
        claims
            .add_additional("user_id", user_id.to_string().as_str())
            .map_err(|e| VerifyError::Internal(e.to_string()))?;
        claims
            .add_additional("scope", scope)
            .map_err(|e| VerifyError::Internal(e.to_string()))?;

        let mut footer = Footer::new();
        // `kid` is a registered PASETO footer claim, so `add_additional`
        // refuses it and `Footer::key_id` only accepts a PASERK id derived from
        // the key -- neither is a plain key id. The footer is therefore built
        // from its JSON form, which is what the verifier parses back out.
        footer
            .parse_string(&serde_json::json!({ "kid": self.key_id }).to_string())
            .map_err(|e| VerifyError::Internal(format!("footer: {e}")))?;

        local::encrypt(&key, &claims, Some(&footer), None)
            .map_err(|e| VerifyError::Internal(e.to_string()))
    }

    /// A token for one HTTP call to the analytics sidecar.
    pub async fn mint_for_call(&self, user_id: Uuid) -> Result<String, VerifyError> {
        self.mint_with(
            user_id,
            "analytics",
            self.config.service_token_ttl_seconds,
            Utc::now(),
        )
        .await
    }

    /// A token for one analytics job row (D72). Longer-lived than a call token
    /// because a queued job is not claimed instantly, short enough that a
    /// stolen job token is worth little.
    pub async fn mint_for_job(&self, user_id: Uuid) -> Result<String, VerifyError> {
        self.mint_with(
            user_id,
            "analytics",
            self.config.job_token_ttl_seconds,
            Utc::now(),
        )
        .await
    }

    #[cfg(test)]
    async fn mint_expiring_in(
        &self,
        user_id: Uuid,
        ttl_seconds: i64,
        now: DateTime<Utc>,
    ) -> Result<String, VerifyError> {
        self.mint_with(user_id, "analytics", ttl_seconds, now).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::key_provider::StaticKeyProvider;
    use std::collections::HashMap;

    fn key() -> Vec<u8> {
        // 32 bytes, as v4.local requires.
        (0u8..32).collect()
    }

    fn setup() -> (Arc<StaticKeyProvider>, AuthConfig) {
        let mut keys = HashMap::new();
        keys.insert("service".to_string(), key());
        keys.insert("human".to_string(), (32u8..64).collect::<Vec<u8>>());
        (Arc::new(StaticKeyProvider::new(keys)), AuthConfig::default())
    }

    #[tokio::test]
    async fn a_minted_service_token_verifies() {
        let (provider, config) = setup();
        let minter = ServiceTokenMinter::new(provider.clone(), "service".into(), config.clone());
        let user = Uuid::new_v4();
        let token = minter.mint_for_call(user).await.unwrap();

        let verified = decrypt_and_validate(provider.as_ref(), &token, &config).await.unwrap();
        assert_eq!(verified.user_id, user);
        assert_eq!(
            verified.claims.get_claim("scope").unwrap().as_str(),
            Some("analytics")
        );
    }

    #[tokio::test]
    async fn a_token_signed_with_the_human_key_is_rejected_by_the_human_rules() {
        // The service key is the only one the sidecar holds; a token minted
        // with the human key must not verify against the service key id.
        let (provider, config) = setup();
        let minter = ServiceTokenMinter::new(provider.clone(), "human".into(), config.clone());
        let token = minter.mint_for_call(Uuid::new_v4()).await.unwrap();

        // The footer names "human"; a provider that does not hold it is what a
        // compromised sidecar has.
        let only_service: Arc<StaticKeyProvider> = Arc::new(StaticKeyProvider::new({
            let mut keys = HashMap::new();
            keys.insert("service".to_string(), key());
            keys
        }));
        let err = decrypt_and_validate(only_service.as_ref(), &token, &config)
            .await
            .unwrap_err();
        assert!(matches!(err, VerifyError::KeyNotFound), "{err:?}");
    }

    /// D76, proven at the verifier: the bearer verifier's key set must not
    /// contain the service key, so a token the sidecar could mint (it holds that
    /// key) cannot be presented as a human bearer token.
    #[tokio::test]
    async fn a_service_token_cannot_be_presented_as_a_bearer_token() {
        let (provider, config) = setup();
        let minter = ServiceTokenMinter::new(provider.clone(), "service".into(), config.clone());
        let token = minter.mint_for_call(Uuid::new_v4()).await.unwrap();

        // The bearer verifier holds only the human key, exactly as
        // `EnvKeyProvider::for_bearer` builds it.
        let bearer_keys: Arc<StaticKeyProvider> = Arc::new(StaticKeyProvider::new({
            let mut keys = HashMap::new();
            keys.insert("human".to_string(), (32u8..64).collect::<Vec<u8>>());
            keys
        }));
        let verifier = PasetoVerifier::new(bearer_keys, config);
        let err = verifier.verify(&token).await.unwrap_err();
        assert!(
            matches!(err, VerifyError::KeyNotFound),
            "the sidecar's key must not open the bearer path: {err:?}"
        );
    }

    #[tokio::test]
    async fn an_expired_token_is_rejected() {
        let (provider, config) = setup();
        let minter = ServiceTokenMinter::new(provider.clone(), "service".into(), config.clone());
        let token = minter
            .mint_expiring_in(
                Uuid::new_v4(),
                -60,
                Utc::now() - chrono::Duration::hours(1),
            )
            .await
            .unwrap();

        let err = decrypt_and_validate(provider.as_ref(), &token, &config)
            .await
            .unwrap_err();
        assert!(matches!(err, VerifyError::InvalidToken), "{err:?}");
    }

    #[tokio::test]
    async fn a_token_for_another_audience_is_rejected() {
        let (provider, config) = setup();
        let mut other = config.clone();
        other.paseto_audience = "somewhere-else".into();
        let minter = ServiceTokenMinter::new(provider.clone(), "service".into(), other);
        let token = minter.mint_for_call(Uuid::new_v4()).await.unwrap();

        let err = decrypt_and_validate(provider.as_ref(), &token, &config)
            .await
            .unwrap_err();
        assert!(matches!(err, VerifyError::InvalidToken), "{err:?}");
    }

    #[tokio::test]
    async fn a_token_from_another_issuer_is_rejected() {
        let (provider, config) = setup();
        let mut other = config.clone();
        other.paseto_issuer = "not-us".into();
        let minter = ServiceTokenMinter::new(provider.clone(), "service".into(), other);
        let token = minter.mint_for_call(Uuid::new_v4()).await.unwrap();

        let err = decrypt_and_validate(provider.as_ref(), &token, &config)
            .await
            .unwrap_err();
        assert!(matches!(err, VerifyError::InvalidToken), "{err:?}");
    }
}
