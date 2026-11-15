use axum::{extract::State, http::StatusCode, response::IntoResponse, Extension, Json};
use serde::Deserialize;
use std::sync::Arc;

use crate::{api::ApiError, auth::AuthPrincipal, model::account_group::AccountGroup, AppState};

#[derive(Debug, Deserialize)]
pub struct RequestBody {
    pub name: String,
}

pub async fn handler(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<AuthPrincipal>,
    Json(body): Json<RequestBody>,
) -> Result<impl IntoResponse, ApiError> {
    let group = state
        .account_group_service
        .create(principal.user_id, body.name)
        .await?;
    Ok((StatusCode::CREATED, Json(group)).into_response())
}

impl serde::Serialize for AccountGroup {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut st = s.serialize_struct("AccountGroup", 3)?;
        st.serialize_field("id", &self.id)?;
        st.serialize_field("name", &self.name)?;
        st.serialize_field("created_at", &self.created_at)?;
        st.end()
    }
}
