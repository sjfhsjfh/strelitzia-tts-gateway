use serde::{Deserialize, Serialize};
use thiserror::Error;
use utoipa::ToSchema;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum GatewayError {
    #[error("{message}")]
    InvalidRequest {
        message: String,
        param: Option<String>,
    },
    #[error("{message}")]
    Authentication { message: String },
    #[error("{message}")]
    NotFound { message: String },
    #[error("{message}")]
    RateLimit {
        message: String,
        retry_after: Option<u64>,
    },
    #[error("{message}")]
    Upstream { message: String },
    #[error("{message}")]
    UpstreamUnavailable { message: String },
    #[error("{message}")]
    Internal { message: String },
}

impl GatewayError {
    pub fn invalid_request(message: impl Into<String>, param: Option<&str>) -> Self {
        Self::InvalidRequest {
            message: message.into(),
            param: param.map(str::to_owned),
        }
    }

    pub fn status_code(&self) -> u16 {
        match self {
            Self::InvalidRequest { .. } => 400,
            Self::Authentication { .. } => 401,
            Self::NotFound { .. } => 404,
            Self::RateLimit { .. } => 429,
            Self::Upstream { .. } => 502,
            Self::UpstreamUnavailable { .. } => 503,
            Self::Internal { .. } => 500,
        }
    }

    pub fn error_type(&self) -> &'static str {
        match self {
            Self::InvalidRequest { .. } | Self::NotFound { .. } => "invalid_request_error",
            Self::Authentication { .. } => "authentication_error",
            Self::RateLimit { .. } => "rate_limit_error",
            Self::Upstream { .. } | Self::UpstreamUnavailable { .. } | Self::Internal { .. } => {
                "server_error"
            }
        }
    }

    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidRequest { .. } => "invalid_request",
            Self::Authentication { .. } => "invalid_api_key",
            Self::NotFound { .. } => "not_found",
            Self::RateLimit { .. } => "rate_limit_exceeded",
            Self::Upstream { .. } => "upstream_error",
            Self::UpstreamUnavailable { .. } => "upstream_unavailable",
            Self::Internal { .. } => "internal_error",
        }
    }

    pub fn param(&self) -> Option<&str> {
        match self {
            Self::InvalidRequest { param, .. } => param.as_deref(),
            _ => None,
        }
    }

    pub fn retry_after(&self) -> Option<u64> {
        match self {
            Self::RateLimit { retry_after, .. } => *retry_after,
            _ => None,
        }
    }

    pub fn envelope(&self) -> ErrorEnvelope {
        ErrorEnvelope {
            error: ErrorBody {
                message: self.to_string(),
                r#type: self.error_type().to_owned(),
                param: self.param().map(str::to_owned),
                code: Some(self.code().to_owned()),
            },
        }
    }
}

impl From<GatewayError> for ErrorEnvelope {
    fn from(error: GatewayError) -> Self {
        error.envelope()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ErrorEnvelope {
    pub error: ErrorBody,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ErrorBody {
    pub message: String,
    #[serde(rename = "type")]
    pub r#type: String,
    pub param: Option<String>,
    pub code: Option<String>,
}
