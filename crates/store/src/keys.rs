use chrono::{DateTime, Utc};
use sqlx::{FromRow, SqlitePool};

use crate::error::{StoreError, map_unique_violation};

// Hashing happens before storage; this layer never sees or returns the plaintext secret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewApiKey {
    pub name: String,
    pub key_hash: String,
    pub key_prefix: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiKey {
    pub id: i64,
    pub name: String,
    pub key_prefix: String,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct ApiKeys {
    pool: SqlitePool,
}

impl ApiKeys {
    pub(crate) fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn create(&self, new: NewApiKey) -> Result<ApiKey, StoreError> {
        let result =
            sqlx::query("INSERT INTO api_keys (name, key_hash, key_prefix) VALUES (?, ?, ?)")
                .bind(&new.name)
                .bind(&new.key_hash)
                .bind(&new.key_prefix)
                .execute(&self.pool)
                .await
                .map_err(map_unique_violation)?;

        self.get(result.last_insert_rowid())
            .await?
            .ok_or(StoreError::NotFound(result.last_insert_rowid()))
    }

    pub async fn find_enabled_by_hash(&self, key_hash: &str) -> Result<Option<ApiKey>, StoreError> {
        let row = sqlx::query_as::<_, ApiKeyRow>(
            "SELECT id, name, key_prefix, enabled, created_at, last_used_at \
             FROM api_keys WHERE key_hash = ? AND enabled = 1",
        )
        .bind(key_hash)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(ApiKey::from))
    }

    pub async fn list(&self) -> Result<Vec<ApiKey>, StoreError> {
        let rows = sqlx::query_as::<_, ApiKeyRow>(
            "SELECT id, name, key_prefix, enabled, created_at, last_used_at \
             FROM api_keys ORDER BY id DESC",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(ApiKey::from).collect())
    }

    pub async fn revoke(&self, id: i64) -> Result<bool, StoreError> {
        let result = sqlx::query("UPDATE api_keys SET enabled = 0 WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn touch_last_used(&self, id: i64) -> Result<bool, StoreError> {
        let result = sqlx::query("UPDATE api_keys SET last_used_at = datetime('now') WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    async fn get(&self, id: i64) -> Result<Option<ApiKey>, StoreError> {
        let row = sqlx::query_as::<_, ApiKeyRow>(
            "SELECT id, name, key_prefix, enabled, created_at, last_used_at \
             FROM api_keys WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(ApiKey::from))
    }
}

#[derive(Debug, FromRow)]
struct ApiKeyRow {
    id: i64,
    name: String,
    key_prefix: String,
    enabled: bool,
    created_at: DateTime<Utc>,
    last_used_at: Option<DateTime<Utc>>,
}

impl From<ApiKeyRow> for ApiKey {
    fn from(row: ApiKeyRow) -> Self {
        Self {
            id: row.id,
            name: row.name,
            key_prefix: row.key_prefix,
            enabled: row.enabled,
            created_at: row.created_at,
            last_used_at: row.last_used_at,
        }
    }
}
