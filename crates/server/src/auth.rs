//! Only the SHA-256 hash of a key is stored; the plaintext is never persisted.

use axum::extract::FromRequestParts;
use axum::http::HeaderMap;
use axum::http::header::AUTHORIZATION;
use axum::http::request::Parts;

use crate::api::ApiError;
use crate::state::AppState;
use strelitzia_core::GatewayError;
use strelitzia_core::keys::{KEY_PREFIX, hash_key};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthKey {
    pub id: i64,
    pub name: String,
}

fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    let value = headers.get(AUTHORIZATION)?.to_str().ok()?.trim();
    let (scheme, token) = value.split_once(' ')?;
    if !scheme.eq_ignore_ascii_case("bearer") {
        return None;
    }
    let token = token.trim();
    (!token.is_empty()).then_some(token)
}

impl FromRequestParts<AppState> for AuthKey {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let token = bearer_token(&parts.headers).ok_or_else(|| {
            ApiError::from(GatewayError::Authentication {
                message: "missing or malformed API key".to_owned(),
            })
        })?;

        let keys = state.db().api_keys();
        let mut found = keys.find_enabled_by_hash(&hash_key(token)).await;
        // Generated keys include the `sk-` marker, but accept a bare key body
        // as well so hand-configured clients that omit the prefix still work.
        if matches!(found, Ok(None)) && !token.starts_with(KEY_PREFIX) {
            found = keys
                .find_enabled_by_hash(&hash_key(&format!("{KEY_PREFIX}{token}")))
                .await;
        }

        let key = found
            .map_err(|error| ApiError::from(GatewayError::from(error)))?
            .ok_or_else(|| {
                ApiError::from(GatewayError::Authentication {
                    message: "invalid API key".to_owned(),
                })
            })?;

        if let Err(error) = keys.touch_last_used(key.id).await {
            tracing::warn!(api_key_id = key.id, %error, "failed to update API key usage");
        }

        let auth = Self {
            id: key.id,
            name: key.name,
        };
        parts.extensions.insert(auth.clone());
        Ok(auth)
    }
}
