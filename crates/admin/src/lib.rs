mod handlers;
mod pages;

use axum::extract::Request;
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::{Router, middleware};
use rand::RngExt;
use sha2::{Digest, Sha256};
use strelitzia_store::Db;
use subtle::ConstantTimeEq;
use tower_sessions::cookie::Key;
use tower_sessions::{MemoryStore, Session, SessionManagerLayer};

pub(crate) const AUTH_KEY: &str = "authenticated";
pub(crate) const CSRF_KEY: &str = "csrf_token";
const SESSION_COOKIE: &str = "strelitzia_admin_session";

#[derive(Clone)]
pub struct AdminState {
    pub db: Db,
    pub token_hash: [u8; 32],
}

impl AdminState {
    pub fn new(db: Db, token: &str) -> Self {
        let digest = Sha256::digest(token.as_bytes());
        let mut token_hash = [0u8; 32];
        token_hash.copy_from_slice(&digest);
        Self { db, token_hash }
    }
}

pub fn router(state: AdminState) -> Router {
    let mut secret = [0u8; 32];
    rand::rng().fill(&mut secret);
    router_with_session_secret(state, &hex::encode(secret))
}

pub fn router_with_session_secret(state: AdminState, session_secret: &str) -> Router {
    let session_layer = SessionManagerLayer::new(MemoryStore::default())
        .with_name(SESSION_COOKIE)
        .with_signed(signing_key(session_secret))
        .with_http_only(true)
        .with_same_site(tower_sessions::cookie::SameSite::Lax);

    let protected = Router::new()
        .route("/", get(handlers::dashboard))
        .route("/logout", post(handlers::logout))
        .route(
            "/channels",
            get(handlers::channels).post(handlers::channel_create),
        )
        .route("/channels/new", get(handlers::channel_new))
        .route("/channels/{id}/edit", get(handlers::channel_edit))
        .route("/channels/{id}", post(handlers::channel_update))
        .route("/channels/{id}/delete", post(handlers::channel_delete))
        .route("/channels/{id}/toggle", post(handlers::channel_toggle))
        .route("/keys", get(handlers::keys).post(handlers::key_create))
        .route("/keys/{id}/revoke", post(handlers::key_revoke))
        .route("/logs", get(handlers::logs))
        .route("/logs/{request_id}", get(handlers::log_detail))
        .route_layer(middleware::from_fn(require_auth));

    Router::new()
        .route("/login", get(handlers::login_form).post(handlers::login))
        .route("/static/admin.css", get(styles))
        .merge(protected)
        .layer(session_layer)
        .with_state(state)
}

fn signing_key(secret: &str) -> Key {
    let mut first = Sha256::new();
    first.update(b"strelitzia admin session v1");
    first.update(secret.as_bytes());

    let mut second = Sha256::new();
    second.update(b"strelitzia admin session v2");
    second.update(secret.as_bytes());

    let mut master = [0u8; 64];
    master[..32].copy_from_slice(&first.finalize());
    master[32..].copy_from_slice(&second.finalize());
    Key::from(master.as_slice())
}

async fn require_auth(session: Session, request: Request, next: Next) -> Response {
    let authenticated = session
        .get::<bool>(AUTH_KEY)
        .await
        .ok()
        .flatten()
        .unwrap_or(false);

    if authenticated {
        return next.run(request).await;
    }

    if request.method() == axum::http::Method::GET {
        Redirect::to("/admin/login").into_response()
    } else {
        (StatusCode::UNAUTHORIZED, "unauthorized").into_response()
    }
}

async fn styles() -> Response {
    (
        [(axum::http::header::CONTENT_TYPE, "text/css; charset=utf-8")],
        include_str!("../static/admin.css"),
    )
        .into_response()
}

pub(crate) async fn csrf_token(session: &Session) -> Result<String, AdminError> {
    if let Some(token) = session
        .get::<String>(CSRF_KEY)
        .await
        .map_err(AdminError::session)?
    {
        return Ok(token);
    }

    let mut bytes = [0u8; 32];
    rand::rng().fill(&mut bytes);
    let token = hex::encode(bytes);
    session
        .insert(CSRF_KEY, &token)
        .await
        .map_err(AdminError::session)?;
    Ok(token)
}

pub(crate) async fn verify_csrf(session: &Session, submitted: &str) -> Result<(), AdminError> {
    let expected = session
        .get::<String>(CSRF_KEY)
        .await
        .map_err(AdminError::session)?
        .ok_or(AdminError::Csrf)?;

    if expected.as_bytes().ct_eq(submitted.as_bytes()).into() {
        Ok(())
    } else {
        Err(AdminError::Csrf)
    }
}

// Constant-time to avoid leaking the admin token through response timing.
pub(crate) fn token_matches(state: &AdminState, submitted: &str) -> bool {
    let digest = Sha256::digest(submitted.as_bytes());
    digest.as_slice().ct_eq(state.token_hash.as_slice()).into()
}

pub(crate) fn see_other(location: &str) -> Response {
    (
        StatusCode::SEE_OTHER,
        [(axum::http::header::LOCATION, location)],
    )
        .into_response()
}

#[derive(Debug)]
pub(crate) enum AdminError {
    Csrf,
    NotFound,
    Internal(String),
}

impl AdminError {
    fn session(error: tower_sessions::session::Error) -> Self {
        tracing::error!(%error, "admin session error");
        Self::Internal("session error".to_owned())
    }
}

impl From<strelitzia_store::StoreError> for AdminError {
    fn from(error: strelitzia_store::StoreError) -> Self {
        match error {
            strelitzia_store::StoreError::NotFound(_) => Self::NotFound,
            other => {
                tracing::error!(%other, "admin store error");
                Self::Internal("database error".to_owned())
            }
        }
    }
}

impl IntoResponse for AdminError {
    fn into_response(self) -> Response {
        match self {
            Self::Csrf => (StatusCode::BAD_REQUEST, "invalid CSRF token").into_response(),
            Self::NotFound => (StatusCode::NOT_FOUND, "not found").into_response(),
            Self::Internal(message) => (StatusCode::INTERNAL_SERVER_ERROR, message).into_response(),
        }
    }
}

pub(crate) fn render_with_status<T>(status: StatusCode, template: T) -> Response
where
    T: askama::Template + IntoResponse,
{
    let mut response = template.into_response();
    *response.status_mut() = status;
    response
}
