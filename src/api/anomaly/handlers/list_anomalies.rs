use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::{
    api::ApiError,
    auth::AuthPrincipal,
    model::anomaly::{Anomaly, AnomalyStatus},
    AppState,
};

#[derive(Debug, Deserialize)]
pub struct QueryParams {
    pub kind: Option<String>,
    pub state: Option<AnomalyStatus>,
    pub limit: Option<i64>,
    pub cursor: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct AnomalyResponse {
    pub id: uuid::Uuid,
    pub kind: String,
    pub subject_type: String,
    pub subject_id: Option<uuid::Uuid>,
    pub title: String,
    pub detail: serde_json::Value,
    pub severity: String,
    pub detected_at: chrono::DateTime<chrono::Utc>,
    pub dismissed_at: Option<chrono::DateTime<chrono::Utc>>,
    pub dismissed_reason: Option<String>,
}

impl From<Anomaly> for AnomalyResponse {
    fn from(a: Anomaly) -> Self {
        Self {
            id: a.id,
            kind: a.kind,
            subject_type: a.subject_type,
            subject_id: a.subject_id,
            title: a.title,
            detail: a.detail,
            severity: a.severity,
            detected_at: a.detected_at,
            dismissed_at: a.dismissed_at,
            dismissed_reason: a.dismissed_reason,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct ListResponse {
    pub items: Vec<AnomalyResponse>,
    pub next_cursor: Option<String>,
}

pub async fn handler(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<AuthPrincipal>,
    Query(params): Query<QueryParams>,
) -> Result<impl IntoResponse, ApiError> {
    let page = state
        .anomaly_service
        .list(
            principal.user_id,
            params.kind,
            params.state,
            params.limit,
            params.cursor,
        )
        .await?;

    let body = ListResponse {
        items: page.items.into_iter().map(AnomalyResponse::from).collect(),
        next_cursor: page.next_cursor,
    };
    Ok((StatusCode::OK, Json(body)).into_response())
}
