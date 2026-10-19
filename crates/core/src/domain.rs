use bytes::Bytes;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use utoipa::ToSchema;

use crate::error::GatewayError;

pub const MAX_INPUT_CHARS: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChannelKind {
    #[serde(rename = "openai_compatible")]
    OpenAiCompatible,
    #[serde(rename = "edge_tts")]
    EdgeTts,
    #[serde(rename = "doubao_tts")]
    DoubaoTts,
}

impl ChannelKind {
    pub const fn default_voice_namespace(self) -> &'static str {
        match self {
            Self::OpenAiCompatible => "openai",
            Self::EdgeTts => "edge",
            Self::DoubaoTts => "volcengine",
        }
    }

    pub fn accepts_namespace(self, channel_namespace: &str, requested: &str) -> bool {
        if channel_namespace.eq_ignore_ascii_case(requested) {
            return true;
        }
        self == Self::DoubaoTts
            && ((channel_namespace.eq_ignore_ascii_case("volcengine")
                && requested.eq_ignore_ascii_case("doubao"))
                || (channel_namespace.eq_ignore_ascii_case("doubao")
                    && requested.eq_ignore_ascii_case("volcengine")))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Channel {
    pub id: i64,
    pub name: String,
    pub kind: ChannelKind,
    pub base_url: Option<String>,
    pub api_key: Option<String>,
    pub models: Vec<String>,
    pub voices: Vec<String>,
    pub voice_namespace: Option<String>,
    pub priority: i64,
    pub weight: u32,
    pub enabled: bool,
}

pub fn split_voice_reference(input: &str) -> (Option<&str>, &str) {
    let Some(index) = input.find([':', '/']) else {
        return (None, input);
    };
    let (namespace, rest) = input.split_at(index);
    let voice = &rest[1..];
    if namespace.is_empty() || voice.is_empty() {
        (None, input)
    } else {
        (Some(namespace), voice)
    }
}

impl Channel {
    pub fn namespace(&self) -> &str {
        self.voice_namespace
            .as_deref()
            .filter(|namespace| !namespace.is_empty())
            .unwrap_or_else(|| self.kind.default_voice_namespace())
    }

    pub fn contains_voice(&self, bare: &str) -> bool {
        self.voices.iter().any(|voice| voice == bare)
    }

    pub fn is_voice_wildcard(&self) -> bool {
        self.voices.is_empty()
    }

    pub fn accepts_voice(&self, requested: &str) -> bool {
        let (namespace, voice) = split_voice_reference(requested);
        if let Some(namespace) = namespace
            && !self.kind.accepts_namespace(self.namespace(), namespace)
        {
            return false;
        }
        self.is_voice_wildcard() || self.contains_voice(voice)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum ResponseFormat {
    #[default]
    Mp3,
    /// Opus is reported as audio/opus, not the Ogg container type.
    Opus,
    Aac,
    Flac,
    Wav,
    Pcm,
}

impl ResponseFormat {
    pub fn content_type(self) -> &'static str {
        match self {
            Self::Mp3 => "audio/mpeg",
            Self::Opus => "audio/opus",
            Self::Aac => "audio/aac",
            Self::Flac => "audio/flac",
            Self::Wav => "audio/wav",
            Self::Pcm => "audio/pcm",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SpeechRequest {
    pub model: String,
    pub input: String,
    pub voice: String,
    #[serde(default)]
    pub response_format: ResponseFormat,
    #[serde(default = "default_speed")]
    pub speed: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
    #[serde(flatten)]
    #[schema(value_type = Object)]
    pub extra: Map<String, Value>,
}

const fn default_speed() -> f32 {
    1.0
}

impl SpeechRequest {
    pub fn validate(&self) -> Result<(), GatewayError> {
        if self.model.trim().is_empty() {
            return Err(GatewayError::invalid_request(
                "`model` must not be empty",
                Some("model"),
            ));
        }
        if self.voice.trim().is_empty() {
            return Err(GatewayError::invalid_request(
                "`voice` must not be empty",
                Some("voice"),
            ));
        }

        let chars = self.input.chars().count();
        if chars > MAX_INPUT_CHARS {
            return Err(GatewayError::invalid_request(
                format!("`input` must be at most {MAX_INPUT_CHARS} characters, got {chars}"),
                Some("input"),
            ));
        }

        if !(0.25..=4.0).contains(&self.speed) {
            return Err(GatewayError::invalid_request(
                "`speed` must be between 0.25 and 4.0",
                Some("speed"),
            ));
        }

        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct AudioResponse {
    pub bytes: Bytes,
    pub content_type: String,
    pub provider: String,
    pub voice: String,
    pub chars: usize,
}
