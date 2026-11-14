use thiserror::Error;

/// Errors that can occur while resolving a key from the external key service.
#[derive(Debug, Error)]
pub enum KeyProviderError {
    /// The key service returned no key for the given id.
    #[error("key not found: {0}")]
    NotFound(String),
    /// The key service could not be reached or returned an error.
    #[error("key service error: {0}")]
    Service(String),
}

/// A resolved symmetric key (raw bytes) for a given key id.
#[derive(Clone, Debug)]
pub struct ResolvedKey {
    pub key_id: String,
    pub bytes: Vec<u8>,
}

/// Abstraction over an external key service.
///
/// The key service owns key material and performs key rotation. This service
/// only fetches the key for a given key id at verification time, so rotated
/// keys are picked up without redeploying or restarting.
#[async_trait::async_trait]
pub trait KeyProvider: Send + Sync {
    /// Fetch the symmetric key bytes for the given key id.
    async fn get_key(&self, key_id: &str) -> Result<ResolvedKey, KeyProviderError>;
}

/// Key provider backed by the process environment.
///
/// Startup fails with a named error when no key is configured, instead of every
/// request failing at verification time with 503 -- a misconfigured deployment
/// should not look like an outage.
pub struct EnvKeyProvider {
    keys: std::collections::HashMap<String, Vec<u8>>,
    active_key_id: String,
}

impl EnvKeyProvider {
    /// Reads `PASETO_KEY_<KID>` for every kid present and requires
    /// `PASETO_ACTIVE_KID` to name one of them.
    pub fn from_env() -> Result<Self, KeyProviderError> {
        let keys = crate::config::auth::AuthConfig::keys_from_env();
        if keys.is_empty() {
            return Err(KeyProviderError::Service(
                "no PASETO keys configured: set PASETO_KEY_<KID> (base64, 32 bytes) \
                 and PASETO_ACTIVE_KID"
                    .into(),
            ));
        }
        let active_key_id = crate::config::auth::AuthConfig::active_key_id().ok_or_else(|| {
            KeyProviderError::Service(
                "PASETO_ACTIVE_KID is not set, so no key can be selected".into(),
            )
        })?;
        if !keys.contains_key(&active_key_id) {
            return Err(KeyProviderError::Service(format!(
                "PASETO_ACTIVE_KID is '{active_key_id}' but no PASETO_KEY_{} is set",
                active_key_id.to_ascii_uppercase()
            )));
        }
        Ok(Self {
            keys,
            active_key_id,
        })
    }

    pub fn active_key_id(&self) -> &str {
        &self.active_key_id
    }
}

#[async_trait::async_trait]
impl KeyProvider for EnvKeyProvider {
    async fn get_key(&self, key_id: &str) -> Result<ResolvedKey, KeyProviderError> {
        self.keys
            .get(key_id)
            .cloned()
            .map(|bytes| ResolvedKey {
                key_id: key_id.to_string(),
                bytes,
            })
            .ok_or_else(|| KeyProviderError::NotFound(key_id.to_string()))
    }
}

/// In-memory key provider for local development / tests.
///
/// Replace with a real client to the external key service (which owns key
/// material and performs rotation) before production.
#[derive(Clone)]
pub struct StaticKeyProvider {
    keys: std::collections::HashMap<String, Vec<u8>>,
}

impl StaticKeyProvider {
    pub fn new(keys: std::collections::HashMap<String, Vec<u8>>) -> Self {
        Self { keys }
    }
}

#[async_trait::async_trait]
impl KeyProvider for StaticKeyProvider {
    async fn get_key(&self, key_id: &str) -> Result<ResolvedKey, KeyProviderError> {
        self.keys
            .get(key_id)
            .cloned()
            .map(|bytes| ResolvedKey {
                key_id: key_id.to_string(),
                bytes,
            })
            .ok_or_else(|| KeyProviderError::NotFound(key_id.to_string()))
    }
}
