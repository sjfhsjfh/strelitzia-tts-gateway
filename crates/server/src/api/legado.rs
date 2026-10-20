use axum::Json;
use axum::extract::State;
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use serde_json::Map;
use strelitzia_core::{
    Channel, ChannelKind, GatewayError, ResponseFormat, SpeechRequest, StreamFormat,
    split_voice_reference,
};
use utoipa::ToSchema;

use crate::api::{
    ApiError, AttemptRun, BodyRejection, RequestId, audio_response, execute_attempts,
    filter_voice_candidates, record_request,
};
use crate::auth::AuthKey;
use crate::state::AppState;

#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
pub struct LegadoSpeechRequest {
    pub text: String,

    pub voice: String,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,

    /// Rates use `N+M%` or `N-M%`, where N is the old Android TTS scale (10-20) and M a percentage adjustment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rate: Option<String>,
}

#[utoipa::path(
    post,
    path = "/speech/stream",
    request_body = LegadoSpeechRequest,
    responses(
        (
            status = 200,
            description = "MP3 audio",
            content((inline(crate::openapi::BinaryAudio) = "audio/mpeg"))
        ),
        (status = 400, description = "Invalid request", body = strelitzia_core::ErrorEnvelope),
        (status = 401, description = "Invalid API key", body = strelitzia_core::ErrorEnvelope),
        (status = 404, description = "No matching channel", body = strelitzia_core::ErrorEnvelope),
        (status = 413, description = "Request body too large", body = strelitzia_core::ErrorEnvelope),
        (status = 429, description = "Upstream rate limit", body = strelitzia_core::ErrorEnvelope),
        (status = 502, description = "Upstream failure", body = strelitzia_core::ErrorEnvelope),
        (status = 503, description = "Upstream unavailable", body = strelitzia_core::ErrorEnvelope)
    ),
    tag = "speech",
    security(("bearer_auth" = []))
)]
pub async fn speech_stream(
    State(state): State<AppState>,
    AuthKey { id: api_key_id, .. }: AuthKey,
    request_id: RequestId,
    payload: Result<Json<LegadoSpeechRequest>, BodyRejection>,
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
                "legado",
                "",
                0,
                &AttemptRun::failed(gateway_error.clone()),
            )
            .await;
            return ApiError::with_status(status, gateway_error).into_response();
        }
    };

    let chars = request.text.chars().count();

    let speed = match parse_rate(request.rate.as_deref()) {
        Ok(speed) => speed,
        Err(error) => {
            record_request(
                &state,
                &request_id.0,
                api_key_id,
                "legado",
                &request.voice,
                chars,
                &AttemptRun::failed(error.clone()),
            )
            .await;
            return ApiError::from(error).into_response();
        }
    };

    let template = SpeechRequest {
        model: "legado".to_owned(),
        input: request.text.clone(),
        voice: request.voice.clone(),
        response_format: ResponseFormat::Mp3,
        speed,
        instructions: None,
        stream_format: StreamFormat::Audio,
        extra: Map::new(),
    };
    if let Err(error) = template.validate() {
        record_request(
            &state,
            &request_id.0,
            api_key_id,
            "legado",
            &request.voice,
            chars,
            &AttemptRun::failed(error.clone()),
        )
        .await;
        return ApiError::from(error).into_response();
    }

    let kind = match request.provider.as_deref() {
        Some(provider) => match parse_provider(provider) {
            Ok(kind) => Some(kind),
            Err(error) => {
                record_request(
                    &state,
                    &request_id.0,
                    api_key_id,
                    "legado",
                    &request.voice,
                    chars,
                    &AttemptRun::failed(error.clone()),
                )
                .await;
                return ApiError::from(error).into_response();
            }
        },
        None => None,
    };

    let channels = match state.db().channels().list(true).await {
        Ok(channels) => channels,
        Err(error) => {
            let error = GatewayError::from(error);
            record_request(
                &state,
                &request_id.0,
                api_key_id,
                "legado",
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
        .filter(|channel| kind.is_none_or(|kind| channel.kind == kind))
        .collect();

    let candidates = match filter_voice_candidates(candidates, &request.voice) {
        Ok(candidates) => candidates,
        Err(error) => {
            record_request(
                &state,
                &request_id.0,
                api_key_id,
                "legado",
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
            message: "no enabled channel matches the requested provider and voice".to_owned(),
        };
        record_request(
            &state,
            &request_id.0,
            api_key_id,
            "legado",
            &request.voice,
            chars,
            &AttemptRun::failed(error.clone()),
        )
        .await;
        return ApiError::from(error).into_response();
    }

    let text = request.text.clone();
    let voice = split_voice_reference(&request.voice).1.to_owned();
    let mut run = execute_attempts(&state, candidates, move |channel| SpeechRequest {
        model: channel.models.first().cloned().unwrap_or_default(),
        input: text.clone(),
        voice: voice.clone(),
        response_format: ResponseFormat::Mp3,
        speed,
        instructions: None,
        stream_format: StreamFormat::Audio,
        extra: Map::new(),
    })
    .await;
    if let Some(audio) = &run.audio {
        run.provider = Some(audio.provider.clone());
    }

    record_request(
        &state,
        &request_id.0,
        api_key_id,
        "legado",
        &request.voice,
        chars,
        &run,
    )
    .await;

    match run.audio {
        Some(mut audio) => {
            audio.content_type = "audio/mpeg".to_owned();
            let provider = audio.provider.clone();
            let voice = audio.voice.clone();
            let audio_chars = audio.chars;
            audio_response(audio, &request_id, &provider, &voice, audio_chars)
        }
        None => ApiError::from(run.error.unwrap_or(GatewayError::Internal {
            message: "speech synthesis failed without an error".to_owned(),
        }))
        .into_response(),
    }
}

fn parse_rate(rate: Option<&str>) -> Result<f32, GatewayError> {
    match rate {
        None => Ok(1.0),
        Some(rate) => parse_legado_rate(rate).ok_or_else(|| {
            GatewayError::invalid_request(
                format!("`rate` must use the form `N+M%` or `N-M%`, got `{rate}`"),
                Some("rate"),
            )
        }),
    }
}

// percentage = (N - 15) / 15 * 100 + M, clamped to [-90, 200]; speed clamped to [0.25, 4.0].
fn parse_legado_rate(rate: &str) -> Option<f32> {
    let rate = rate.trim().strip_suffix('%')?;
    let sign_index = rate.find(['+', '-'])?;
    if sign_index == 0 {
        return None;
    }

    let (base, adjustment) = rate.split_at(sign_index);
    let base: f32 = base.trim().parse().ok()?;
    let sign = if adjustment.starts_with('+') {
        1.0
    } else {
        -1.0
    };
    let offset: f32 = adjustment[1..].trim().parse().ok()?;

    let percent = (base - 15.0) / 15.0 * 100.0 + sign * offset;
    let percent = percent.clamp(-90.0, 200.0);

    Some((1.0 + percent / 100.0).clamp(0.25, 4.0))
}

fn parse_provider(provider: &str) -> Result<ChannelKind, GatewayError> {
    match provider.trim().to_ascii_lowercase().as_str() {
        "edge" | "edge_tts" => Ok(ChannelKind::EdgeTts),
        "openai" | "openai_compatible" => Ok(ChannelKind::OpenAiCompatible),
        "doubao" | "doubao_tts" => Ok(ChannelKind::DoubaoTts),
        _ => Err(GatewayError::invalid_request(
            format!("unsupported provider `{provider}`"),
            Some("provider"),
        )),
    }
}
