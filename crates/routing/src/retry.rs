use strelitzia_core::GatewayError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryPolicy {
    pub max_attempts: u32,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self { max_attempts: 3 }
    }
}

impl RetryPolicy {
    // Only transient statuses retry: 408, 425, 429, 500, 502, 503, 504.
    pub fn is_retryable_status(status: u16) -> bool {
        matches!(status, 408 | 425 | 429 | 500 | 502 | 503 | 504)
    }

    pub fn should_retry(
        &self,
        attempts_made: u32,
        status: Option<u16>,
        audio_started: bool,
    ) -> bool {
        if audio_started || attempts_made >= self.max_attempts {
            return false;
        }

        match status {
            None => true,
            Some(status) => Self::is_retryable_status(status),
        }
    }

    pub fn retry_after_hint(err: &GatewayError) -> Option<u64> {
        match err {
            GatewayError::RateLimit { retry_after, .. } => *retry_after,
            _ => None,
        }
    }
}
