use async_trait::async_trait;
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

#[derive(Debug, Clone, Copy, Default)]
pub struct MockAdapter;

#[async_trait]
impl TtsAdapter for MockAdapter {
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
        _channel: &Channel,
        req: &SpeechRequest,
    ) -> Result<AudioResponse, GatewayError> {
        let body = format!("mock:{}:{}:{}", req.model, req.voice, req.input);

        Ok(AudioResponse::once(
            body.into_bytes(),
            req.response_format.content_type(),
            "mock",
            req.voice.clone(),
            req.input.chars().count(),
        ))
    }
}
