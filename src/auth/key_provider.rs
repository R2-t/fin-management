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

/// In-memory key provider for local development / tests.
///
/// Replace with a real client to the external key service (which owns key
/// material and performs rotation) before production.
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
