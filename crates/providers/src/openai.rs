use async_trait::async_trait;
use futures_util::StreamExt;
use reqwest::StatusCode;
use reqwest::header::{CONTENT_TYPE, HeaderMap, RETRY_AFTER};
use strelitzia_core::ErrorEnvelope;
use strelitzia_core::adapter::{Capabilities, TtsAdapter};
use strelitzia_core::domain::{AudioResponse, Channel, ChannelKind, ResponseFormat, SpeechRequest};
use strelitzia_core::error::GatewayError;

const FORMATS: &[ResponseFormat] = &[
    ResponseFormat::Mp3,
    ResponseFormat::Opus,
    ResponseFormat::Aac,
    ResponseFormat::Flac,
    ResponseFormat::Wav,
    ResponseFormat::Pcm,
];

const MAX_BODY_BYTES: usize = 20 * 1024 * 1024;

const SPEECH_PATH: &str = "/v1/audio/speech";

#[derive(Debug, Clone)]
pub struct OpenAiCompatibleAdapter {
    client: reqwest::Client,
}

impl OpenAiCompatibleAdapter {
    pub fn new(client: reqwest::Client) -> Self {
        Self { client }
    }
}

#[async_trait]
impl TtsAdapter for OpenAiCompatibleAdapter {
    fn kind(&self) -> ChannelKind {
        ChannelKind::OpenAiCompatible
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            formats: FORMATS,
            supports_instructions: true,
        }
    }

    async fn synthesize(
        &self,
        channel: &Channel,
        req: &SpeechRequest,
    ) -> Result<AudioResponse, GatewayError> {
        let url = speech_url(channel.base_url.as_deref())?;
        let body = serde_json::to_vec(req).map_err(|error| GatewayError::Internal {
            message: format!("failed to serialize speech request: {error}"),
        })?;

        let mut request = self
            .client
            .post(url)
            .header(CONTENT_TYPE, "application/json")
            .body(body);
        if let Some(api_key) = channel
            .api_key
            .as_deref()
            .map(str::trim)
            .filter(|key| !key.is_empty())
        {
            request = request.bearer_auth(api_key);
        }

        let response = request
            .send()
            .await
            .map_err(|error| GatewayError::UpstreamUnavailable {
                message: sanitize(
                    format!("upstream request failed: {error}"),
                    channel.api_key.as_deref(),
                ),
            })?;

        let status = response.status();
        if !status.is_success() {
            let retry_after = parse_retry_after(response.headers());
            let body = read_body(response).await.unwrap_or_default();
            return Err(map_error(
                status,
                retry_after,
                &body,
                channel.api_key.as_deref(),
            ));
        }

        let header = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let content_type = audio_content_type(header.as_deref(), req.response_format)?;
        if response
            .content_length()
            .is_some_and(|length| length > MAX_BODY_BYTES as u64)
        {
            return Err(body_too_large());
        }

        let api_key = channel.api_key.clone();
        let stream = response.bytes_stream().map(move |chunk| match chunk {
            Ok(bytes) => Ok(bytes),
            Err(error) => Err(GatewayError::UpstreamUnavailable {
                message: sanitize(
                    format!("failed to read upstream response: {error}"),
                    api_key.as_deref(),
                ),
            }),
        });

        Ok(AudioResponse {
            stream: Box::pin(stream),
            content_type,
            provider: "openai".to_owned(),
            voice: req.voice.clone(),
            chars: req.input.chars().count(),
        })
    }
}

fn speech_url(base_url: Option<&str>) -> Result<String, GatewayError> {
    let base_url = base_url.map(str::trim).unwrap_or_default();
    let base_url = base_url.trim_end_matches('/');
    if base_url.is_empty() {
        return Err(GatewayError::invalid_request(
            "channel `base_url` is required for OpenAI-compatible channels",
            Some("base_url"),
        ));
    }

    if base_url.ends_with(SPEECH_PATH) {
        Ok(base_url.to_owned())
    } else {
        Ok(format!("{base_url}{SPEECH_PATH}"))
    }
}

async fn read_body(mut response: reqwest::Response) -> Result<Vec<u8>, GatewayError> {
    if response
        .content_length()
        .is_some_and(|length| length > MAX_BODY_BYTES as u64)
    {
        return Err(body_too_large());
    }

    let mut body = Vec::new();
    while let Some(chunk) =
        response
            .chunk()
            .await
            .map_err(|error| GatewayError::UpstreamUnavailable {
                message: format!("failed to read upstream response: {error}"),
            })?
    {
        if body.len().saturating_add(chunk.len()) > MAX_BODY_BYTES {
            return Err(body_too_large());
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

fn body_too_large() -> GatewayError {
    GatewayError::Upstream {
        message: format!("upstream response exceeded {MAX_BODY_BYTES} bytes"),
    }
}

fn parse_retry_after(headers: &HeaderMap) -> Option<u64> {
    headers.get(RETRY_AFTER)?.to_str().ok()?.trim().parse().ok()
}

// application/octet-stream is a transport wrapper; return the requested format's MIME type instead.
fn audio_content_type(
    header: Option<&str>,
    requested: ResponseFormat,
) -> Result<String, GatewayError> {
    let Some(header) = header else {
        return Err(GatewayError::Upstream {
            message: "upstream response is missing a Content-Type header".to_owned(),
        });
    };
    let mime = header.split(';').next().unwrap_or(header).trim();

    if mime.eq_ignore_ascii_case("application/octet-stream") {
        return Ok(requested.content_type().to_owned());
    }
    if mime
        .as_bytes()
        .get(..6)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(b"audio/"))
    {
        return Ok(mime.to_owned());
    }

    Err(GatewayError::Upstream {
        message: format!("upstream returned unsupported content type `{mime}`"),
    })
}

// 401/403 always mean authentication failure regardless of body; other messages are surfaced after key redaction.
fn map_error(
    status: StatusCode,
    retry_after: Option<u64>,
    body: &[u8],
    api_key: Option<&str>,
) -> GatewayError {
    let parsed = serde_json::from_slice::<ErrorEnvelope>(body)
        .ok()
        .map(|envelope| envelope.error);
    let upstream_message = parsed
        .as_ref()
        .map(|error| error.message.trim())
        .filter(|message| !message.is_empty());
    let param = parsed
        .as_ref()
        .and_then(|error| error.param.as_deref())
        .filter(|param| !param.is_empty());

    let message =
        |fallback: &str| sanitize(upstream_message.unwrap_or(fallback).to_owned(), api_key);

    match status.as_u16() {
        400 => GatewayError::InvalidRequest {
            message: message(&format!("upstream rejected the request (HTTP {status})")),
            param: param.map(str::to_owned),
        },
        // Never forward the provider's authentication message: providers may
        // echo (part of) the key in it.
        401 | 403 => GatewayError::Authentication {
            message: sanitize(
                format!("upstream authentication failed (HTTP {status})"),
                api_key,
            ),
        },
        404 => GatewayError::NotFound {
            message: message(&format!("upstream endpoint was not found (HTTP {status})")),
        },
        429 => GatewayError::RateLimit {
            message: message(&format!("upstream rate limit exceeded (HTTP {status})")),
            retry_after,
        },
        503 => GatewayError::UpstreamUnavailable {
            message: message(&format!("upstream is unavailable (HTTP {status})")),
        },
        _ if status.is_server_error() => GatewayError::Upstream {
            message: message(&format!("upstream failed (HTTP {status})")),
        },
        _ => GatewayError::Upstream {
            message: message(&format!("upstream returned HTTP {status}")),
        },
    }
}

fn sanitize(mut message: String, api_key: Option<&str>) -> String {
    if let Some(api_key) = api_key.map(str::trim).filter(|key| !key.is_empty()) {
        message = message.replace(api_key, "[redacted]");
    }
    message
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};
    use strelitzia_core::TtsAdapter;
    use strelitzia_core::domain::{Channel, ChannelKind, SpeechRequest};
    use strelitzia_core::error::GatewayError;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::OpenAiCompatibleAdapter;

    async fn collect(mut stream: strelitzia_core::AudioStream) -> Vec<u8> {
        use futures_util::StreamExt;

        let mut body = Vec::new();
        while let Some(chunk) = stream.next().await {
            body.extend_from_slice(&chunk.unwrap());
        }
        body
    }

    fn channel(base_url: impl Into<String>, api_key: Option<&str>) -> Channel {
        Channel {
            id: 1,
            name: "openai".to_owned(),
            kind: ChannelKind::OpenAiCompatible,
            base_url: Some(base_url.into()),
            api_key: api_key.map(str::to_owned),
            models: vec!["tts-1".to_owned()],
            voices: Vec::new(),
            voice_namespace: None,
            priority: 0,
            weight: 1,
            enabled: true,
        }
    }

    fn request_with_extra() -> SpeechRequest {
        serde_json::from_value(json!({
            "model": "tts-1",
            "input": "hello",
            "voice": "alloy",
            "response_format": "mp3",
            "speed": 1.25,
            "stream_format": "sse",
            "custom_field": { "nested": true },
            "numeric_field": 7
        }))
        .unwrap()
    }

    async fn synthesize_error(response: ResponseTemplate) -> GatewayError {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/audio/speech"))
            .respond_with(response)
            .mount(&server)
            .await;

        let adapter = OpenAiCompatibleAdapter::new(reqwest::Client::new());
        adapter
            .synthesize(&channel(server.uri(), None), &request_with_extra())
            .await
            .unwrap_err()
    }

    #[tokio::test]
    async fn forwards_request_maps_binary_response_and_upstream_errors() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/audio/speech"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("Content-Type", "audio/mpeg")
                    .set_body_bytes(b"ID3-audio".to_vec()),
            )
            .expect(1)
            .mount(&server)
            .await;

        let adapter = OpenAiCompatibleAdapter::new(reqwest::Client::new());
        let response = adapter
            .synthesize(
                &channel(server.uri(), Some("sk-secret")),
                &request_with_extra(),
            )
            .await
            .unwrap();

        assert_eq!(collect(response.stream).await, b"ID3-audio".to_vec());
        assert_eq!(response.content_type, "audio/mpeg");
        assert_eq!(response.provider, "openai");
        assert_eq!(response.voice, "alloy");
        assert_eq!(response.chars, 5);

        let requests = server.received_requests().await.unwrap();
        let request = &requests[0];
        assert_eq!(request.url.path(), "/v1/audio/speech");
        assert_eq!(
            request
                .headers
                .get("authorization")
                .unwrap()
                .to_str()
                .unwrap(),
            "Bearer sk-secret"
        );
        assert_eq!(
            request
                .headers
                .get("content-type")
                .unwrap()
                .to_str()
                .unwrap(),
            "application/json"
        );

        let body: Value = serde_json::from_slice(&request.body).unwrap();
        assert_eq!(
            body,
            json!({
                "model": "tts-1",
                "input": "hello",
                "voice": "alloy",
                "response_format": "mp3",
                "speed": 1.25,
                "custom_field": { "nested": true },
                "numeric_field": 7
            })
        );
        assert!(body.get("instructions").is_none());
        assert!(body.get("stream_format").is_none());

        let error = synthesize_error(
            ResponseTemplate::new(429)
                .insert_header("Retry-After", "7")
                .set_body_json(json!({
                    "error": {
                        "message": "slow down",
                        "type": "rate_limit_error",
                        "param": null,
                        "code": "rate_limit_exceeded"
                    }
                })),
        )
        .await;
        match error {
            GatewayError::RateLimit {
                message,
                retry_after,
            } => {
                assert_eq!(message, "slow down");
                assert_eq!(retry_after, Some(7));
            }
            other => panic!("unexpected error: {other:?}"),
        }

        let error = synthesize_error(ResponseTemplate::new(400).set_body_json(json!({
            "error": {
                "message": "unsupported voice",
                "type": "invalid_request_error",
                "param": "voice",
                "code": "invalid_value"
            }
        })))
        .await;
        assert_eq!(
            error,
            GatewayError::InvalidRequest {
                message: "unsupported voice".to_owned(),
                param: Some("voice".to_owned()),
            }
        );

        let error = synthesize_error(ResponseTemplate::new(500).set_body_json(json!({
            "error": {
                "message": "upstream exploded",
                "type": "server_error",
                "param": null,
                "code": null
            }
        })))
        .await;
        match error {
            GatewayError::Upstream { message } => assert_eq!(message, "upstream exploded"),
            other => panic!("unexpected error: {other:?}"),
        }
    }
}
