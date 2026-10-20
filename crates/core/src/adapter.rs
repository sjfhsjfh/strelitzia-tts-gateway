use async_trait::async_trait;

use crate::domain::{AudioResponse, Channel, ChannelKind, ResponseFormat, SpeechRequest};
use crate::error::GatewayError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capabilities {
    pub formats: &'static [ResponseFormat],
    pub supports_instructions: bool,
}

#[async_trait]
pub trait TtsAdapter: Send + Sync {
    fn kind(&self) -> ChannelKind;

    fn capabilities(&self) -> Capabilities;

    /// Start synthesis and return a stream of encoded audio chunks.
    ///
    /// `Ok` means the upstream accepted the request and is about to emit
    /// audio; `Err` before `Ok` is eligible for retry and channel failover.
    /// Adapters may prefetch up to the first audio chunk so pre-audio failures
    /// surface as `Err`. Once `Ok` is returned the attempt can no longer be
    /// retried or failed over.
    ///
    /// The returned stream drives the upstream connection and must be polled
    /// to make progress; dropping it cancels the upstream request. An `Err`
    /// item is terminal: the stream yields no items after it. `content_type`
    /// must be a concrete `audio/*` MIME type. Implementations must not leave
    /// detached background tasks behind.
    async fn synthesize(
        &self,
        channel: &Channel,
        req: &SpeechRequest,
    ) -> Result<AudioResponse, GatewayError>;
}
