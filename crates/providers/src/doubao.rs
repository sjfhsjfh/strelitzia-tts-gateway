//! Adapter for Volcengine Doubao (seed-tts-2.0) HTTP chunked synthesis.
//!
//! The endpoint streams newline-delimited JSON objects. Each object either
//! carries a base64 MP3 chunk (`code == 0`), reports normal completion
//! (`code == 20000000`), or reports a business error. Some deployments wrap
//! each object in an SSE `data:` prefix, which is accepted as well.

use async_trait::async_trait;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use reqwest::StatusCode;
use reqwest::header::{CONTENT_TYPE, HeaderMap, RETRY_AFTER};
use serde::Deserialize;
use serde_json::json;
use strelitzia_core::adapter::{Capabilities, TtsAdapter};
use strelitzia_core::domain::{AudioResponse, Channel, ChannelKind, ResponseFormat, SpeechRequest};
use strelitzia_core::error::GatewayError;
use uuid::Uuid;

const FORMATS: &[ResponseFormat] = &[ResponseFormat::Mp3];

const SPEECH_PATH: &str = "/api/v3/tts/unidirectional";
const RESOURCE_ID: &str = "seed-tts-2.0";
const SAMPLE_RATE: u32 = 24_000;
const COMPLETION_CODE: i64 = 20_000_000;
const LOG_ID_HEADER: &str = "x-tt-logid";

/// Maximum upstream error body retained for diagnostics.
const MAX_ERROR_BODY_BYTES: usize = 8 * 1024;

/// Adapter for Doubao text-to-speech.
#[derive(Debug, Clone)]
pub struct DoubaoTtsAdapter {
    client: reqwest::Client,
}

impl DoubaoTtsAdapter {
    /// Use a caller-configured client, including its timeouts and TLS settings.
    pub fn new(client: reqwest::Client) -> Self {
        Self { client }
    }
}

#[async_trait]
impl TtsAdapter for DoubaoTtsAdapter {
    fn kind(&self) -> ChannelKind {
        ChannelKind::DoubaoTts
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            formats: FORMATS,
            supports_instructions: false,
            supports_stream: false,
        }
    }

    async fn synthesize(
        &self,
        channel: &Channel,
        req: &SpeechRequest,
    ) -> Result<AudioResponse, GatewayError> {
        if req.response_format != ResponseFormat::Mp3 {
            return Err(GatewayError::invalid_request(
                "Doubao TTS only supports the `mp3` response format",
                Some("response_format"),
            ));
        }

        let api_key = channel
            .api_key
            .as_deref()
            .map(str::trim)
            .filter(|key| !key.is_empty())
            .ok_or_else(|| {
                GatewayError::invalid_request("Doubao TTS requires an API key", Some("api_key"))
            })?;

        let payload = json!({
            "req_params": {
                "text": &req.input,
                "speaker": &req.voice,
                "audio_params": {
                    "format": "mp3",
                    "sample_rate": SAMPLE_RATE,
                    "speech_rate": speech_rate(req.speed),
                },
                "section_id": Uuid::now_v7().to_string(),
            }
        });
        let body = serde_json::to_vec(&payload).map_err(|error| GatewayError::Internal {
            message: format!("failed to serialize Doubao request: {error}"),
        })?;

        let response = self
            .client
            .post(speech_url(channel.base_url.as_deref())?)
            .header(CONTENT_TYPE, "application/json")
            .header("X-Api-Key", api_key)
            .header("X-Api-Resource-Id", RESOURCE_ID)
            .header("X-Api-Request-Id", Uuid::now_v7().to_string())
            .body(body)
            .send()
            .await
            .map_err(|error| GatewayError::UpstreamUnavailable {
                message: sanitize(format!("Doubao request failed: {error}"), Some(api_key)),
            })?;

        let status = response.status();
        let log_id = response
            .headers()
            .get(LOG_ID_HEADER)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);

        if !status.is_success() {
            let retry_after = parse_retry_after(response.headers());
            let body = read_limited(response, MAX_ERROR_BODY_BYTES)
                .await
                .unwrap_or_default();
            return Err(map_http_error(
                status,
                retry_after,
                &body,
                api_key,
                log_id.as_deref(),
            ));
        }

        let audio = read_events(response, api_key, log_id.as_deref()).await?;

        Ok(AudioResponse {
            bytes: audio.into(),
            content_type: ResponseFormat::Mp3.content_type().to_owned(),
            provider: "doubao_tts".to_owned(),
            voice: req.voice.clone(),
            chars: req.input.chars().count(),
        })
    }
}

/// Resolve the endpoint from the channel's explicitly configured base URL.
fn speech_url(base_url: Option<&str>) -> Result<String, GatewayError> {
    let base_url = base_url.map(str::trim).unwrap_or_default();
    if base_url.is_empty() {
        return Err(GatewayError::invalid_request(
            "Doubao TTS requires a base URL",
            Some("base_url"),
        ));
    }
    let base_url = base_url.trim_end_matches('/');

    if base_url.ends_with(SPEECH_PATH) {
        Ok(base_url.to_owned())
    } else {
        Ok(format!("{base_url}{SPEECH_PATH}"))
    }
}

/// Convert an absolute speed multiplier to Doubao's percentage scale.
fn speech_rate(speed: f32) -> i32 {
    (((speed - 1.0) * 100.0).round() as i32).clamp(-50, 100)
}

#[derive(Debug, Deserialize)]
struct DoubaoEvent {
    #[serde(default)]
    code: i64,
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    data: Option<String>,
}

enum LineOutcome {
    Continue,
    Completed,
}

/// Drain the chunked NDJSON response, decoding and concatenating audio.
async fn read_events(
    mut response: reqwest::Response,
    api_key: &str,
    log_id: Option<&str>,
) -> Result<Vec<u8>, GatewayError> {
    let mut audio = Vec::new();
    let mut pending = Vec::new();
    let mut completed = false;

    while let Some(chunk) =
        response
            .chunk()
            .await
            .map_err(|error| GatewayError::UpstreamUnavailable {
                message: sanitize(
                    with_log_id(format!("Doubao response stream failed: {error}"), log_id),
                    Some(api_key),
                ),
            })?
    {
        pending.extend_from_slice(&chunk);

        while let Some(index) = pending.iter().position(|byte| *byte == b'\n') {
            let line: Vec<u8> = pending.drain(..=index).collect();
            if matches!(
                handle_event_line(&line, &mut audio, api_key, log_id)?,
                LineOutcome::Completed
            ) {
                completed = true;
                break;
            }
        }

        if completed {
            break;
        }
    }

    if !completed && !pending.is_empty() {
        completed = matches!(
            handle_event_line(&pending, &mut audio, api_key, log_id)?,
            LineOutcome::Completed
        );
    }

    if !completed {
        return Err(GatewayError::Upstream {
            message: sanitize(
                with_log_id(
                    "Doubao stream ended without a completion event".to_owned(),
                    log_id,
                ),
                Some(api_key),
            ),
        });
    }
    if audio.is_empty() {
        return Err(GatewayError::Upstream {
            message: sanitize(
                with_log_id("Doubao returned no audio".to_owned(), log_id),
                Some(api_key),
            ),
        });
    }

    Ok(audio)
}

fn handle_event_line(
    line: &[u8],
    audio: &mut Vec<u8>,
    api_key: &str,
    log_id: Option<&str>,
) -> Result<LineOutcome, GatewayError> {
    let line = std::str::from_utf8(line).map_err(|error| GatewayError::Upstream {
        message: sanitize(
            with_log_id(
                format!("Doubao returned an invalid UTF-8 line: {error}"),
                log_id,
            ),
            Some(api_key),
        ),
    })?;
    let line = line.trim();
    if line.is_empty()
        || line.starts_with(':')
        || line.starts_with("event:")
        || line.starts_with("id:")
    {
        return Ok(LineOutcome::Continue);
    }

    let payload = line.strip_prefix("data:").map(str::trim).unwrap_or(line);
    if payload.is_empty() || payload == "[DONE]" {
        return Ok(LineOutcome::Continue);
    }

    let event: DoubaoEvent =
        serde_json::from_str(payload).map_err(|error| GatewayError::Upstream {
            message: sanitize(
                with_log_id(format!("Doubao returned invalid JSON: {error}"), log_id),
                Some(api_key),
            ),
        })?;

    match event.code {
        COMPLETION_CODE => Ok(LineOutcome::Completed),
        0 => {
            if let Some(encoded) = event.data.as_deref().filter(|data| !data.is_empty()) {
                let decoded = STANDARD
                    .decode(encoded)
                    .map_err(|error| GatewayError::Upstream {
                        message: sanitize(
                            with_log_id(
                                format!("Doubao returned invalid audio data: {error}"),
                                log_id,
                            ),
                            Some(api_key),
                        ),
                    })?;
                audio.extend_from_slice(&decoded);
            }
            Ok(LineOutcome::Continue)
        }
        code => {
            let message = event
                .message
                .as_deref()
                .map(str::trim)
                .filter(|message| !message.is_empty())
                .unwrap_or("unknown error");
            Err(GatewayError::Upstream {
                message: sanitize(
                    with_log_id(format!("Doubao API error {code}: {message}"), log_id),
                    Some(api_key),
                ),
            })
        }
    }
}

async fn read_limited(
    mut response: reqwest::Response,
    limit: usize,
) -> Result<Vec<u8>, reqwest::Error> {
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        let remaining = limit.saturating_sub(body.len());
        if remaining == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..chunk.len().min(remaining)]);
    }
    Ok(body)
}

fn map_http_error(
    status: StatusCode,
    retry_after: Option<u64>,
    body: &[u8],
    api_key: &str,
    log_id: Option<&str>,
) -> GatewayError {
    let detail = String::from_utf8_lossy(body);
    let detail = detail.trim();
    let detail = if detail.is_empty() {
        String::new()
    } else {
        format!(": {}", detail.chars().take(500).collect::<String>())
    };

    let message = |fallback: &str| {
        sanitize(
            with_log_id(format!("{fallback} (HTTP {status}){detail}"), log_id),
            Some(api_key),
        )
    };

    match status.as_u16() {
        400 => GatewayError::InvalidRequest {
            message: message("Doubao rejected the request"),
            param: None,
        },
        // Never forward the provider's authentication message: it may echo key material.
        401 | 403 => GatewayError::Authentication {
            message: sanitize(
                with_log_id(
                    format!("Doubao authentication failed (HTTP {status})"),
                    log_id,
                ),
                Some(api_key),
            ),
        },
        404 => GatewayError::NotFound {
            message: message("Doubao endpoint was not found"),
        },
        429 => GatewayError::RateLimit {
            message: message("Doubao rate limit exceeded"),
            retry_after,
        },
        503 => GatewayError::UpstreamUnavailable {
            message: message("Doubao is unavailable"),
        },
        _ if status.is_server_error() => GatewayError::Upstream {
            message: message("Doubao failed"),
        },
        _ => GatewayError::Upstream {
            message: message("Doubao returned an unexpected status"),
        },
    }
}

fn parse_retry_after(headers: &HeaderMap) -> Option<u64> {
    headers.get(RETRY_AFTER)?.to_str().ok()?.trim().parse().ok()
}

fn with_log_id(message: String, log_id: Option<&str>) -> String {
    match log_id {
        Some(log_id) if !log_id.is_empty() => format!("{message} (logid {log_id})"),
        _ => message,
    }
}

/// Never surface the configured key in diagnostics.
fn sanitize(mut message: String, api_key: Option<&str>) -> String {
    if let Some(api_key) = api_key.map(str::trim).filter(|key| !key.is_empty()) {
        message = message.replace(api_key, "[redacted]");
    }
    message
}
