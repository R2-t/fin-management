//! Bridging the PASETO minter to the analytics client's trait.
//!
//! The application layer depends on [`ServiceTokenSource`](crate::application::outbound::analytics::ServiceTokenSource),
//! not on the token library or the key material, so the HTTP client can be
//! tested with a fake that returns a fixed string.

use std::sync::Arc;

use uuid::Uuid;

use crate::application::outbound::analytics::ServiceTokenSource;
use crate::auth::key_provider::{EnvKeyProvider, KeyProvider};
use crate::auth::paseto::ServiceTokenMinter;

/// Mints a fresh token for each call, naming the tenant being served.
pub struct PasetoServiceTokenSource<K> {
    minter: Arc<ServiceTokenMinter<K>>,
}

impl<K> PasetoServiceTokenSource<K>
where
    K: KeyProvider,
{
    pub fn new(minter: Arc<ServiceTokenMinter<K>>) -> Self {
        Self { minter }
    }
}

#[async_trait::async_trait]
impl<K> ServiceTokenSource for PasetoServiceTokenSource<K>
where
    K: KeyProvider + 'static,
{
    async fn token_for(&self, user_id: Uuid) -> Result<String, String> {
        self.minter
            .mint_for_call(user_id)
            .await
            .map_err(|e| e.to_string())
    }
}

/// The production wiring: the service key from the environment.
///
/// Returns `None` when no service key is configured, so a deployment without one
/// serves unauthenticated calls the sidecar refuses loudly, rather than failing
/// to start for a feature that is not required.
pub fn service_token_source(
    config: crate::config::auth::AuthConfig,
) -> Option<Arc<dyn ServiceTokenSource>> {
    let key_provider = EnvKeyProvider::from_env().ok()?;
    let key_id = crate::config::auth::AuthConfig::service_key_id()?;
    let minter = Arc::new(ServiceTokenMinter::new(
        Arc::new(key_provider),
        key_id,
        config,
    ));
    Some(Arc::new(PasetoServiceTokenSource::new(minter)))
}
