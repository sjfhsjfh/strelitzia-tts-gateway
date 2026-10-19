use std::time::Duration;

use async_trait::async_trait;
use edge_tts_rust::{EdgeTtsClient, Error as EdgeError, SpeakOptions};
use strelitzia_core::adapter::{Capabilities, TtsAdapter};
use strelitzia_core::domain::{AudioResponse, Channel, ChannelKind, ResponseFormat, SpeechRequest};
use strelitzia_core::error::GatewayError;

const FORMATS: &[ResponseFormat] = &[ResponseFormat::Mp3];

const OUTPUT_FORMAT: &str = "audio-24khz-48kbitrate-mono-mp3";

pub const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

pub const DEFAULT_RECEIVE_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Debug, Clone)]
pub struct EdgeTtsAdapter {
    client: EdgeTtsClient,
}

impl EdgeTtsAdapter {
    pub fn new(connect_timeout: Duration, receive_timeout: Duration) -> Result<Self, GatewayError> {
        let client = EdgeTtsClient::builder()
            .connect_timeout(connect_timeout)
            .receive_timeout(receive_timeout)
            .default_output_format(OUTPUT_FORMAT)
            // Connections are opened on demand: pre-warming would require a
            // Tokio runtime at construction time and starts live network
            // traffic before the first request.
            .ws_warmup(false)
            .build()
            .map_err(|error| GatewayError::Internal {
                message: format!("failed to create the Edge TTS client: {error}"),
            })?;

        Ok(Self { client })
    }

    pub fn with_default_timeouts() -> Result<Self, GatewayError> {
        Self::new(DEFAULT_CONNECT_TIMEOUT, DEFAULT_RECEIVE_TIMEOUT)
    }
}

#[async_trait]
impl TtsAdapter for EdgeTtsAdapter {
    fn kind(&self) -> ChannelKind {
        ChannelKind::EdgeTts
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
        _channel: &Channel,
        req: &SpeechRequest,
    ) -> Result<AudioResponse, GatewayError> {
        if req.response_format != ResponseFormat::Mp3 {
            return Err(GatewayError::invalid_request(
                "Edge TTS only supports the `mp3` response format",
                Some("response_format"),
            ));
        }

        let options = SpeakOptions {
            voice: req.voice.clone(),
            rate: edge_rate(req.speed),
            output_format: OUTPUT_FORMAT.to_owned(),
            ..SpeakOptions::default()
        };

        let result = self
            .client
            .synthesize(req.input.clone(), options)
            .await
            .map_err(map_synthesis_error)?;

        Ok(AudioResponse {
            bytes: result.audio.into(),
            content_type: ResponseFormat::Mp3.content_type().to_owned(),
            provider: "edge_tts".to_owned(),
            voice: req.voice.clone(),
            chars: req.input.chars().count(),
        })
    }
}

// Round half away from zero; unity renders as +0% rather than -0%.
fn edge_rate(speed: f32) -> String {
    let percent = ((speed - 1.0) * 100.0).round() as i64;
    if percent >= 0 {
        format!("+{percent}%")
    } else {
        format!("{percent}%")
    }
}

fn map_synthesis_error(error: EdgeError) -> GatewayError {
    match error {
        EdgeError::InvalidRate(_)
        | EdgeError::InvalidVolume(_)
        | EdgeError::InvalidPitch(_)
        | EdgeError::InvalidVoice(_)
        | EdgeError::InvalidOutputFormat(_)
        | EdgeError::InvalidChunkSize
        | EdgeError::InvalidSplitPoint => {
            GatewayError::invalid_request(format!("Edge TTS rejected the request: {error}"), None)
        }
        EdgeError::Http(_) | EdgeError::WebSocket(_) | EdgeError::Io(_) => {
            GatewayError::UpstreamUnavailable {
                message: format!("Edge TTS synthesis failed: {error}"),
            }
        }
        EdgeError::HttpRequest(_) => GatewayError::Internal {
            message: format!("Edge TTS request construction failed: {error}"),
        },
        _ => GatewayError::Upstream {
            message: format!("Edge TTS synthesis failed: {error}"),
        },
    }
}
