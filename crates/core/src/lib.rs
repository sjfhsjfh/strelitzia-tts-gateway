pub mod adapter;
pub mod domain;
pub mod error;
pub mod keys;

pub use adapter::{Capabilities, TtsAdapter};
pub use domain::{
    AudioResponse, AudioStream, Channel, ChannelKind, MAX_INPUT_CHARS, ResponseFormat,
    SpeechRequest, StreamFormat, split_voice_reference,
};
pub use error::{ErrorBody, ErrorEnvelope, GatewayError};
pub use keys::{GeneratedKey, generate_key, hash_key};
