use chrono::{DateTime, Utc};
use sqlx::{FromRow, SqlitePool};

use crate::error::{StoreError, map_unique_violation};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewRequestLog {
    pub request_id: String,
    pub api_key_id: Option<i64>,
    pub model: String,
    pub voice: String,
    pub provider: Option<String>,
    pub channel_id: Option<i64>,
    pub chars: i64,
    pub status_code: i64,
    pub error: Option<String>,
    pub latency_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewAttempt {
    pub channel_id: Option<i64>,
    pub attempt_no: i64,
    pub outcome: String,
    pub error: Option<String>,
    pub latency_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestLog {
    pub id: i64,
    pub request_id: String,
    pub api_key_id: Option<i64>,
    pub model: String,
    pub voice: String,
    pub provider: Option<String>,
    pub channel_id: Option<i64>,
    pub chars: i64,
    pub status_code: i64,
    pub error: Option<String>,
    pub latency_ms: i64,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestAttempt {
    pub id: i64,
    pub request_log_id: i64,
    pub channel_id: Option<i64>,
    pub attempt_no: i64,
    pub outcome: String,
    pub error: Option<String>,
    pub latency_ms: i64,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestLogDetail {
    pub log: RequestLog,
    pub attempts: Vec<RequestAttempt>,
}

#[derive(Debug, Clone)]
pub struct Logs {
    pool: SqlitePool,
}

impl Logs {
    pub(crate) fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    // The log and its attempts are written in one transaction.
    pub async fn record(
        &self,
        log: NewRequestLog,
        attempts: &[NewAttempt],
    ) -> Result<i64, StoreError> {
        let mut transaction = self.pool.begin().await?;

        let result = sqlx::query(
            "INSERT INTO request_logs \
             (request_id, api_key_id, model, voice, provider, channel_id, chars, status_code, error, latency_ms) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&log.request_id)
        .bind(log.api_key_id)
        .bind(&log.model)
        .bind(&log.voice)
        .bind(&log.provider)
        .bind(log.channel_id)
        .bind(log.chars)
        .bind(log.status_code)
        .bind(&log.error)
        .bind(log.latency_ms)
        .execute(&mut *transaction)
        .await
        .map_err(map_unique_violation)?;
        let log_id = result.last_insert_rowid();

        for attempt in attempts {
            sqlx::query(
                "INSERT INTO request_attempts \
                 (request_log_id, channel_id, attempt_no, outcome, error, latency_ms) \
                 VALUES (?, ?, ?, ?, ?, ?)",
            )
            .bind(log_id)
            .bind(attempt.channel_id)
            .bind(attempt.attempt_no)
            .bind(&attempt.outcome)
            .bind(&attempt.error)
            .bind(attempt.latency_ms)
            .execute(&mut *transaction)
            .await?;
        }

        transaction.commit().await?;
        Ok(log_id)
    }

    pub async fn list_recent(
        &self,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<RequestLog>, StoreError> {
        let rows = sqlx::query_as::<_, RequestLogRow>(
            "SELECT id, request_id, api_key_id, model, voice, provider, channel_id, chars, \
             status_code, error, latency_ms, created_at \
             FROM request_logs ORDER BY id DESC LIMIT ? OFFSET ?",
        )
        .bind(i64::from(limit))
        .bind(i64::from(offset))
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(RequestLog::from).collect())
    }

    pub async fn get_by_request_id(
        &self,
        request_id: &str,
    ) -> Result<Option<RequestLogDetail>, StoreError> {
        let row = sqlx::query_as::<_, RequestLogRow>(
            "SELECT id, request_id, api_key_id, model, voice, provider, channel_id, chars, \
             status_code, error, latency_ms, created_at \
             FROM request_logs WHERE request_id = ?",
        )
        .bind(request_id)
        .fetch_optional(&self.pool)
        .await?;

        let Some(row) = row else {
            return Ok(None);
        };
        let log = RequestLog::from(row);

        let rows = sqlx::query_as::<_, RequestAttemptRow>(
            "SELECT id, request_log_id, channel_id, attempt_no, outcome, error, latency_ms, created_at \
             FROM request_attempts WHERE request_log_id = ? ORDER BY attempt_no, id",
        )
        .bind(log.id)
        .fetch_all(&self.pool)
        .await?;

        Ok(Some(RequestLogDetail {
            log,
            attempts: rows.into_iter().map(RequestAttempt::from).collect(),
        }))
    }
}

#[derive(Debug, FromRow)]
struct RequestLogRow {
    id: i64,
    request_id: String,
    api_key_id: Option<i64>,
    model: String,
    voice: String,
    provider: Option<String>,
    channel_id: Option<i64>,
    chars: i64,
    status_code: i64,
    error: Option<String>,
    latency_ms: i64,
    created_at: DateTime<Utc>,
}

impl From<RequestLogRow> for RequestLog {
    fn from(row: RequestLogRow) -> Self {
        Self {
            id: row.id,
            request_id: row.request_id,
            api_key_id: row.api_key_id,
            model: row.model,
            voice: row.voice,
            provider: row.provider,
            channel_id: row.channel_id,
            chars: row.chars,
            status_code: row.status_code,
            error: row.error,
            latency_ms: row.latency_ms,
            created_at: row.created_at,
        }
    }
}

#[derive(Debug, FromRow)]
struct RequestAttemptRow {
    id: i64,
    request_log_id: i64,
    channel_id: Option<i64>,
    attempt_no: i64,
    outcome: String,
    error: Option<String>,
    latency_ms: i64,
    created_at: DateTime<Utc>,
}

impl From<RequestAttemptRow> for RequestAttempt {
    fn from(row: RequestAttemptRow) -> Self {
        Self {
            id: row.id,
            request_log_id: row.request_log_id,
            channel_id: row.channel_id,
            attempt_no: row.attempt_no,
            outcome: row.outcome,
            error: row.error,
            latency_ms: row.latency_ms,
            created_at: row.created_at,
        }
    }
}
