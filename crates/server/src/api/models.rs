use std::collections::BTreeSet;

use axum::Json;
use axum::extract::State;
use serde::Serialize;
use utoipa::ToSchema;

use crate::api::ApiError;
use crate::auth::AuthKey;
use crate::state::AppState;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct ModelObject {
    pub id: String,

    pub object: String,

    pub created: i64,

    pub owned_by: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct ModelList {
    pub object: String,

    pub data: Vec<ModelObject>,
}

#[utoipa::path(
    get,
    path = "/v1/models",
    responses(
        (status = 200, description = "Available models", body = ModelList),
        (status = 401, description = "Invalid API key", body = strelitzia_core::ErrorEnvelope)
    ),
    tag = "models",
    security(("bearer_auth" = []))
)]
pub async fn list_models(
    State(state): State<AppState>,
    AuthKey { .. }: AuthKey,
) -> Result<Json<ModelList>, ApiError> {
    let channels = state
        .db()
        .channels()
        .list(true)
        .await
        .map_err(ApiError::from)?;

    let mut ids = BTreeSet::new();
    for channel in channels {
        ids.extend(channel.models);
    }

    let created = chrono::Utc::now().timestamp();
    let data = ids
        .into_iter()
        .map(|id| ModelObject {
            id,
            object: "model".to_owned(),
            created,
            owned_by: "strelitzia".to_owned(),
        })
        .collect();

    Ok(Json(ModelList {
        object: "list".to_owned(),
        data,
    }))
}
