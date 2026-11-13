use bb8_postgres::{bb8::Pool, PostgresConnectionManager};
use thiserror::Error;
use tokio_postgres::NoTls;

pub mod analytics;
pub mod category;
pub mod item;
pub mod ledger;
pub mod purchase;
pub mod purchase_history;
pub mod rate_cache;
pub mod transfer;
pub mod user_profile;
pub mod wallet;

pub type PgPool = Pool<PostgresConnectionManager<NoTls>>;

#[derive(Debug, Error)]
pub enum RepositoryError {
    #[error("Database error: {0}")]
    DbError(#[from] tokio_postgres::Error),
    #[error("Pool error: {0}")]
    PoolError(#[from] bb8_postgres::bb8::RunError<tokio_postgres::Error>),
    #[error("Row not found")]
    NotFound,
    #[error("Version conflict")]
    VersionConflict,
    #[error("Serialization error: {0}")]
    Serde(#[from] serde_json::Error),
}
