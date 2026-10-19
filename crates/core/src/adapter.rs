use async_trait::async_trait;

use crate::domain::{AudioResponse, Channel, ChannelKind, ResponseFormat, SpeechRequest};
use crate::error::GatewayError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capabilities {
    pub formats: &'static [ResponseFormat],
    pub supports_instructions: bool,
    pub supports_stream: bool,
}

#[async_trait]
pub trait TtsAdapter: Send + Sync {
    fn kind(&self) -> ChannelKind;

    fn capabilities(&self) -> Capabilities;

    async fn synthesize(
        &self,
        channel: &Channel,
        req: &SpeechRequest,
    ) -> Result<AudioResponse, GatewayError>;
}
