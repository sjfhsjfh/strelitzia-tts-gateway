mod health;
mod legado;
mod models;
mod speech;

use std::any::Any;
use std::convert::Infallible;
use std::panic::AssertUnwindSafe;
use std::time::Instant;

use axum::body::Body;
use axum::extract::{DefaultBodyLimit, FromRequestParts, Request};
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE, RETRY_AFTER};
use axum::http::request::Parts;
use axum::http::{HeaderName, HeaderValue, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router, middleware};
use futures_util::FutureExt;
use strelitzia_core::{
    AudioResponse, Channel, ChannelKind, GatewayError, SpeechRequest, split_voice_reference,
};
use strelitzia_routing::select_channel;
use strelitzia_store::{NewAttempt, NewRequestLog};
use tower_http::catch_panic::CatchPanicLayer;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::state::AppState;

pub const MAX_BODY_BYTES: usize = 64 * 1024;

pub const REQUEST_ID_HEADER: &str = "x-request-id";

#[derive(Debug, Clone)]
pub struct RequestId(pub String);

impl<S> FromRequestParts<S> for RequestId
where
    S: Send + Sync,
{
    type Rejection = Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Infallible> {
        Ok(parts
            .extensions
            .get::<Self>()
            .cloned()
            .unwrap_or_else(|| Self(uuid::Uuid::now_v7().to_string())))
    }
}

async fn request_id_layer(mut request: Request, next: Next) -> Response {
    let id = request
        .headers()
        .get(REQUEST_ID_HEADER)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| uuid::Uuid::now_v7().to_string());
    let id = RequestId(id);
    request.extensions_mut().insert(id.clone());

    let mut response = next.run(request).await;
    response.headers_mut().insert(
        HeaderName::from_static(REQUEST_ID_HEADER),
        header_value(&id.0),
    );
    response
}

#[derive(Debug)]
pub struct ApiError {
    status: StatusCode,
    error: GatewayError,
}

impl ApiError {
    pub fn new(error: GatewayError) -> Self {
        let status =
            StatusCode::from_u16(error.status_code()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        Self { status, error }
    }

    pub fn with_status(status: StatusCode, error: GatewayError) -> Self {
        Self { status, error }
    }
}

impl From<GatewayError> for ApiError {
    fn from(error: GatewayError) -> Self {
        Self::new(error)
    }
}

impl From<strelitzia_store::StoreError> for ApiError {
    fn from(error: strelitzia_store::StoreError) -> Self {
        Self::new(GatewayError::from(error))
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let mut response = (self.status, Json(self.error.envelope())).into_response();
        if let Some(seconds) = self.error.retry_after() {
            response
                .headers_mut()
                .insert(RETRY_AFTER, header_value(&seconds.to_string()));
        }
        response
    }
}

pub type BodyRejection = axum::extract::rejection::JsonRejection;

const FAVICON_SVG: &str = include_str!("../../../../assets/icon.svg");
const FAVICON_ICO: &[u8] = include_bytes!("../../../../assets/icon.ico");
const FAVICON_MAX_AGE: &str = "public, max-age=86400";

async fn favicon_svg() -> Response {
    (
        [
            (CONTENT_TYPE, "image/svg+xml"),
            (CACHE_CONTROL, FAVICON_MAX_AGE),
        ],
        FAVICON_SVG,
    )
        .into_response()
}

async fn favicon_ico() -> Response {
    (
        [
            (CONTENT_TYPE, "image/x-icon"),
            (CACHE_CONTROL, FAVICON_MAX_AGE),
        ],
        FAVICON_ICO,
    )
        .into_response()
}

pub(crate) fn openapi_router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(health::healthz))
        .routes(routes!(health::readyz))
        .routes(routes!(health::openapi_json))
        .routes(routes!(models::list_models))
        .routes(routes!(speech::create_speech))
        .routes(routes!(legado::speech_stream))
}

pub fn router(state: AppState) -> Router {
    let router: Router = openapi_router()
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
        .layer(middleware::from_fn(request_id_layer))
        .with_state(state)
        .into();

    router
        .route("/favicon.svg", get(favicon_svg))
        .route("/favicon.ico", get(favicon_ico))
        .layer(CatchPanicLayer::new())
}

pub(crate) fn filter_voice_candidates(
    channels: Vec<Channel>,
    requested: &str,
) -> Result<Vec<Channel>, GatewayError> {
    let (namespace, voice) = split_voice_reference(requested);
    if namespace.is_some() {
        return Ok(channels
            .into_iter()
            .filter(|channel| channel.accepts_voice(requested))
            .collect());
    }

    let explicit: Vec<Channel> = channels
        .iter()
        .filter(|channel| channel.contains_voice(voice))
        .cloned()
        .collect();
    if explicit.is_empty() {
        return Ok(channels
            .into_iter()
            .filter(Channel::is_voice_wildcard)
            .collect());
    }

    let mut namespaces: Vec<String> = explicit
        .iter()
        .map(|channel| {
            let namespace = channel.namespace().to_ascii_lowercase();
            if namespace == "doubao" {
                "volcengine".to_owned()
            } else {
                namespace
            }
        })
        .collect();
    namespaces.sort_unstable();
    namespaces.dedup();
    if namespaces.len() > 1 {
        return Err(GatewayError::invalid_request(
            format!("voice `{requested}` is ambiguous; use `provider:voice`"),
            Some("voice"),
        ));
    }

    Ok(explicit)
}

pub(crate) struct AttemptRun {
    pub audio: Option<AudioResponse>,

    pub error: Option<GatewayError>,

    pub attempts: Vec<NewAttempt>,

    pub channel_id: Option<i64>,

    pub provider: Option<String>,

    pub latency_ms: i64,
}

impl AttemptRun {
    pub fn failed(error: GatewayError) -> Self {
        Self {
            audio: None,
            error: Some(error),
            attempts: Vec::new(),
            channel_id: None,
            provider: None,
            latency_ms: 0,
        }
    }

    pub fn status_code(&self) -> u16 {
        match (&self.audio, &self.error) {
            (Some(_), _) => 200,
            (None, Some(error)) => error.status_code(),
            (None, None) => 500,
        }
    }
}

pub(crate) async fn execute_attempts<F>(
    state: &AppState,
    candidates: Vec<Channel>,
    build: F,
) -> AttemptRun
where
    F: Fn(&Channel) -> SpeechRequest,
{
    let started = Instant::now();
    let mut remaining = candidates;
    let mut attempts = Vec::new();
    let mut last_error = None;
    let mut last_channel_id = None;
    let mut last_provider = None;
    let mut retry_index = 0usize;

    loop {
        let selected = {
            let mut rng = rand::rng();
            select_channel(&remaining, retry_index, &mut rng).cloned()
        };
        let Some(channel) = selected else {
            if last_error.is_none() {
                last_error = Some(GatewayError::NotFound {
                    message: "no eligible channel is available".to_owned(),
                });
            }
            break;
        };
        remaining.retain(|candidate| candidate.id != channel.id);

        last_channel_id = Some(channel.id);
        last_provider = Some(channel_kind_label(channel.kind).to_owned());

        let attempt_started = Instant::now();
        let request = build(&channel);
        let result = match state.adapters().get(channel.kind) {
            Some(adapter) => {
                let synthesis = AssertUnwindSafe(tokio::time::timeout(
                    state.upstream_timeout(),
                    adapter.synthesize(&channel, &request),
                ))
                .catch_unwind()
                .await;

                match synthesis {
                    Ok(Ok(result)) => result,
                    Ok(Err(_)) => Err(GatewayError::UpstreamUnavailable {
                        message: format!(
                            "upstream timed out after {} seconds",
                            state.upstream_timeout().as_secs()
                        ),
                    }),
                    Err(payload) => {
                        let message = panic_message(payload.as_ref());
                        tracing::error!(
                            channel_id = channel.id,
                            channel_kind = channel_kind_label(channel.kind),
                            panic = %message,
                            "adapter panicked"
                        );
                        Err(GatewayError::Internal {
                            message: format!(
                                "adapter for channel kind `{}` panicked: {message}",
                                channel_kind_label(channel.kind)
                            ),
                        })
                    }
                }
            }
            None => Err(GatewayError::UpstreamUnavailable {
                message: format!(
                    "no adapter is registered for channel kind `{}`",
                    channel_kind_label(channel.kind)
                ),
            }),
        };
        let latency_ms = elapsed_ms(attempt_started);
        let attempt_no = attempts.len() as i64 + 1;

        match result {
            Ok(audio) => {
                attempts.push(NewAttempt {
                    channel_id: Some(channel.id),
                    attempt_no,
                    outcome: "success".to_owned(),
                    error: None,
                    latency_ms,
                });
                return AttemptRun {
                    audio: Some(audio),
                    error: None,
                    attempts,
                    channel_id: Some(channel.id),
                    provider: last_provider,
                    latency_ms: elapsed_ms(started),
                };
            }
            Err(error) => {
                let retryable = state.retry().should_retry(
                    u32::try_from(attempt_no).unwrap_or(u32::MAX),
                    Some(error.status_code()),
                    false,
                );
                attempts.push(NewAttempt {
                    channel_id: Some(channel.id),
                    attempt_no,
                    outcome: "error".to_owned(),
                    error: Some(error.to_string()),
                    latency_ms,
                });
                last_error = Some(error);

                if retryable && !remaining.is_empty() {
                    retry_index += 1;
                    continue;
                }
                break;
            }
        }
    }

    AttemptRun {
        audio: None,
        error: last_error,
        attempts,
        channel_id: last_channel_id,
        provider: last_provider,
        latency_ms: elapsed_ms(started),
    }
}

pub(crate) async fn record_request(
    state: &AppState,
    request_id: &str,
    api_key_id: i64,
    model: &str,
    voice: &str,
    chars: usize,
    run: &AttemptRun,
) {
    let log = NewRequestLog {
        request_id: request_id.to_owned(),
        api_key_id: Some(api_key_id),
        model: model.to_owned(),
        voice: voice.to_owned(),
        provider: run.provider.clone(),
        channel_id: run.channel_id,
        chars: i64::try_from(chars).unwrap_or(i64::MAX),
        status_code: i64::from(run.status_code()),
        error: run.error.as_ref().map(ToString::to_string),
        latency_ms: run.latency_ms,
    };

    if let Err(error) = state.db().logs().record(log, &run.attempts).await {
        tracing::warn!(%error, request_id, "failed to persist request log");
    }

    tracing::info!(
        request_id,
        status_code = run.status_code(),
        latency_ms = run.latency_ms,
        attempts = run.attempts.len(),
        "request completed"
    );
}

pub(crate) fn audio_response(
    audio: AudioResponse,
    request_id: &RequestId,
    provider: &str,
    voice: &str,
    chars: usize,
) -> Response {
    let content_type = if audio.content_type.is_empty() {
        "application/octet-stream"
    } else {
        audio.content_type.as_str()
    };
    let mut response = Body::from(audio.bytes).into_response();
    *response.status_mut() = StatusCode::OK;

    let headers = response.headers_mut();
    headers.insert(CONTENT_TYPE, header_value(content_type));
    headers.insert(
        HeaderName::from_static("x-provider"),
        header_value(provider),
    );
    headers.insert(HeaderName::from_static("x-voice"), header_value(voice));
    headers.insert(
        HeaderName::from_static("x-chars"),
        header_value(&chars.to_string()),
    );
    headers.insert(
        HeaderName::from_static(REQUEST_ID_HEADER),
        header_value(&request_id.0),
    );
    response
}

fn channel_kind_label(kind: ChannelKind) -> &'static str {
    match kind {
        ChannelKind::OpenAiCompatible => "openai_compatible",
        ChannelKind::EdgeTts => "edge_tts",
        ChannelKind::DoubaoTts => "doubao_tts",
    }
}

fn header_value(value: &str) -> HeaderValue {
    HeaderValue::from_str(value).unwrap_or_else(|_| HeaderValue::from_static("invalid"))
}

fn panic_message(payload: &(dyn Any + Send)) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_owned()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "non-string panic payload".to_owned()
    }
}

fn elapsed_ms(started: Instant) -> i64 {
    i64::try_from(started.elapsed().as_millis()).unwrap_or(i64::MAX)
}
