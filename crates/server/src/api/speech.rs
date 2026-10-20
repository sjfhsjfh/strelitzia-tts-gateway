use axum::Json;
use axum::extract::State;
use axum::response::{IntoResponse, Response};
use strelitzia_core::{Channel, GatewayError, SpeechRequest, StreamFormat, split_voice_reference};

use crate::api::{
    ApiError, AttemptRun, BodyRejection, RequestId, audio_response, execute_attempts,
    filter_voice_candidates, record_request, sse_response,
};
use crate::auth::AuthKey;
use crate::state::AppState;

#[utoipa::path(
    post,
    path = "/v1/audio/speech",
    request_body = SpeechRequest,
    responses(
        (
            status = 200,
            description = "Encoded audio, or `text/event-stream` frames when `stream_format` is `sse`",
            content(
                (inline(crate::openapi::BinaryAudio) = "audio/mpeg"),
                (inline(crate::openapi::BinaryAudio) = "audio/opus"),
                (inline(crate::openapi::BinaryAudio) = "audio/aac"),
                (inline(crate::openapi::BinaryAudio) = "audio/flac"),
                (inline(crate::openapi::BinaryAudio) = "audio/wav"),
                (inline(crate::openapi::BinaryAudio) = "audio/pcm"),
                (inline(crate::openapi::SseStream) = "text/event-stream")
            )
        ),
        (status = 400, description = "Invalid request", body = strelitzia_core::ErrorEnvelope),
        (status = 401, description = "Invalid API key", body = strelitzia_core::ErrorEnvelope),
        (status = 404, description = "Unknown model", body = strelitzia_core::ErrorEnvelope),
        (status = 413, description = "Request body too large", body = strelitzia_core::ErrorEnvelope),
        (status = 429, description = "Upstream rate limit", body = strelitzia_core::ErrorEnvelope),
        (status = 502, description = "Upstream failure", body = strelitzia_core::ErrorEnvelope),
        (status = 503, description = "Upstream unavailable", body = strelitzia_core::ErrorEnvelope)
    ),
    tag = "speech",
    security(("bearer_auth" = []))
)]
pub async fn create_speech(
    State(state): State<AppState>,
    AuthKey { id: api_key_id, .. }: AuthKey,
    request_id: RequestId,
    payload: Result<Json<SpeechRequest>, BodyRejection>,
) -> Response {
    let request = match payload {
        Ok(Json(request)) => request,
        Err(rejection) => {
            let status = rejection.status();
            let gateway_error = GatewayError::invalid_request(rejection.body_text(), None);
            record_request(
                &state,
                &request_id.0,
                api_key_id,
                "",
                "",
                0,
                &AttemptRun::failed(gateway_error.clone()),
            )
            .await;
            return ApiError::with_status(status, gateway_error).into_response();
        }
    };

    let chars = request.input.chars().count();

    if let Err(error) = request.validate() {
        record_request(
            &state,
            &request_id.0,
            api_key_id,
            &request.model,
            &request.voice,
            chars,
            &AttemptRun::failed(error.clone()),
        )
        .await;
        return ApiError::from(error).into_response();
    }

    let channels = match state.db().channels().list(true).await {
        Ok(channels) => channels,
        Err(error) => {
            let error = GatewayError::from(error);
            record_request(
                &state,
                &request_id.0,
                api_key_id,
                &request.model,
                &request.voice,
                chars,
                &AttemptRun::failed(error.clone()),
            )
            .await;
            return ApiError::from(error).into_response();
        }
    };

    let candidates: Vec<Channel> = channels
        .into_iter()
        .filter(|channel| channel.models.iter().any(|model| model == &request.model))
        .collect();

    let candidates = match filter_voice_candidates(candidates, &request.voice) {
        Ok(candidates) => candidates,
        Err(error) => {
            record_request(
                &state,
                &request_id.0,
                api_key_id,
                &request.model,
                &request.voice,
                chars,
                &AttemptRun::failed(error.clone()),
            )
            .await;
            return ApiError::from(error).into_response();
        }
    };

    if candidates.is_empty() {
        let error = GatewayError::NotFound {
            message: format!(
                "no enabled channel serves model `{}` and voice `{}`",
                request.model, request.voice
            ),
        };
        record_request(
            &state,
            &request_id.0,
            api_key_id,
            &request.model,
            &request.voice,
            chars,
            &AttemptRun::failed(error.clone()),
        )
        .await;
        return ApiError::from(error).into_response();
    }

    let mut upstream = request.clone();
    upstream.voice = split_voice_reference(&request.voice).1.to_owned();
    let mut run = execute_attempts(&state, candidates, move |_channel| upstream.clone()).await;
    if let Some(audio) = &run.audio {
        run.provider = Some(audio.provider.clone());
    }

    record_request(
        &state,
        &request_id.0,
        api_key_id,
        &request.model,
        &request.voice,
        chars,
        &run,
    )
    .await;

    match run.audio {
        Some(audio) => {
            let provider = audio.provider.clone();
            let voice = audio.voice.clone();
            let audio_chars = audio.chars;
            match request.stream_format {
                StreamFormat::Audio => {
                    audio_response(audio, &request_id, &provider, &voice, audio_chars)
                }
                StreamFormat::Sse => {
                    sse_response(audio, &request_id, &provider, &voice, audio_chars)
                }
            }
        }
        None => ApiError::from(run.error.unwrap_or(GatewayError::Internal {
            message: "speech synthesis failed without an error".to_owned(),
        }))
        .into_response(),
    }
}
