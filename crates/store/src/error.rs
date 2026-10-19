use strelitzia_core::GatewayError;
use thiserror::Error;

// SQLx errors become strings so the driver does not appear in this crate's public API.
#[derive(Debug, Error)]
pub enum StoreError {
    #[error("database error: {0}")]
    Database(String),

    #[error("migration error: {0}")]
    Migration(String),

    #[error("filesystem error: {0}")]
    Io(#[from] std::io::Error),

    #[error("invalid stored data: {0}")]
    InvalidData(String),

    #[error("record {0} not found")]
    NotFound(i64),

    #[error("conflict: {0}")]
    Conflict(String),
}

impl From<sqlx::Error> for StoreError {
    fn from(error: sqlx::Error) -> Self {
        Self::Database(error.to_string())
    }
}

impl From<sqlx::migrate::MigrateError> for StoreError {
    fn from(error: sqlx::migrate::MigrateError) -> Self {
        Self::Migration(error.to_string())
    }
}

impl From<StoreError> for GatewayError {
    fn from(error: StoreError) -> Self {
        match error {
            StoreError::NotFound(id) => Self::NotFound {
                message: format!("record {id} not found"),
            },
            other => Self::Internal {
                message: other.to_string(),
            },
        }
    }
}

pub(crate) fn map_unique_violation(error: sqlx::Error) -> StoreError {
    match error {
        sqlx::Error::Database(database) if database.is_unique_violation() => {
            StoreError::Conflict(database.message().to_owned())
        }
        other => StoreError::from(other),
    }
}
