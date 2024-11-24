use std::sync::Arc;

use axum::Extension;
use opentelemetry::logs::{LogRecord as _, Logger, Severity};
use opentelemetry_sdk::logs::LogRecord;
use tracing::info;

use crate::AppState;

pub async fn handler(Extension(state): Extension<Arc<AppState>>) -> &'static str {
    let mut log_record = LogRecord::default();
    log_record.set_severity_number(Severity::Info);
    log_record.set_body("list wallets logs".into());
    state.logger.emit(log_record);

    info!("list wallets span");

    "list wallets response"
}
