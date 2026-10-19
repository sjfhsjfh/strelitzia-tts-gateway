use sqlx::{FromRow, QueryBuilder, Sqlite, SqlitePool};
use strelitzia_core::{Channel, ChannelKind};

use crate::error::StoreError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewChannel {
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

/// None leaves a column unchanged; the nested option on nullable columns clears them with Some(None).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChannelUpdate {
    pub name: Option<String>,
    pub kind: Option<ChannelKind>,
    pub base_url: Option<Option<String>>,
    pub api_key: Option<Option<String>>,
    pub models: Option<Vec<String>>,
    pub voices: Option<Vec<String>>,
    pub voice_namespace: Option<Option<String>>,
    pub priority: Option<i64>,
    pub weight: Option<u32>,
    pub enabled: Option<bool>,
}

#[derive(Debug, Clone)]
pub struct Channels {
    pool: SqlitePool,
}

impl Channels {
    pub(crate) fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn list(&self, enabled_only: bool) -> Result<Vec<Channel>, StoreError> {
        let sql = if enabled_only {
            "SELECT id, name, kind, base_url, api_key, models, voices, voice_namespace, priority, weight, enabled \
             FROM channels WHERE enabled = 1 ORDER BY priority DESC, id"
        } else {
            "SELECT id, name, kind, base_url, api_key, models, voices, voice_namespace, priority, weight, enabled \
             FROM channels ORDER BY priority DESC, id"
        };
        let rows = sqlx::query_as::<_, ChannelRow>(sql)
            .fetch_all(&self.pool)
            .await?;
        rows.into_iter().map(Channel::try_from).collect()
    }

    pub async fn get(&self, id: i64) -> Result<Option<Channel>, StoreError> {
        let row = sqlx::query_as::<_, ChannelRow>(
            "SELECT id, name, kind, base_url, api_key, models, voices, voice_namespace, priority, weight, enabled \
             FROM channels WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        row.map(Channel::try_from).transpose()
    }

    pub async fn create(&self, new: NewChannel) -> Result<Channel, StoreError> {
        let models = encode_models(&new.models)?;
        let result = sqlx::query(
            "INSERT INTO channels \
             (name, kind, base_url, api_key, models, voices, voice_namespace, priority, weight, enabled) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&new.name)
        .bind(kind_as_str(new.kind))
        .bind(&new.base_url)
        .bind(&new.api_key)
        .bind(&models)
        .bind(encode_voices(&new.voices)?)
        .bind(&new.voice_namespace)
        .bind(new.priority)
        .bind(i64::from(new.weight))
        .bind(new.enabled)
        .execute(&self.pool)
        .await?;

        let id = result.last_insert_rowid();
        self.get(id).await?.ok_or(StoreError::NotFound(id))
    }

    pub async fn update(&self, id: i64, patch: ChannelUpdate) -> Result<Channel, StoreError> {
        let mut query =
            QueryBuilder::<Sqlite>::new("UPDATE channels SET updated_at = datetime('now')");

        if let Some(name) = &patch.name {
            query.push(", name = ").push_bind(name);
        }
        if let Some(kind) = patch.kind {
            query.push(", kind = ").push_bind(kind_as_str(kind));
        }
        if let Some(base_url) = &patch.base_url {
            query.push(", base_url = ").push_bind(base_url.as_deref());
        }
        if let Some(api_key) = &patch.api_key {
            query.push(", api_key = ").push_bind(api_key.as_deref());
        }
        if let Some(models) = &patch.models {
            query.push(", models = ").push_bind(encode_models(models)?);
        }
        if let Some(voices) = &patch.voices {
            query.push(", voices = ").push_bind(encode_voices(voices)?);
        }
        if let Some(voice_namespace) = &patch.voice_namespace {
            query
                .push(", voice_namespace = ")
                .push_bind(voice_namespace.as_deref());
        }
        if let Some(priority) = patch.priority {
            query.push(", priority = ").push_bind(priority);
        }
        if let Some(weight) = patch.weight {
            query.push(", weight = ").push_bind(i64::from(weight));
        }
        if let Some(enabled) = patch.enabled {
            query.push(", enabled = ").push_bind(enabled);
        }

        query.push(" WHERE id = ").push_bind(id);
        let result = query.build().execute(&self.pool).await?;
        if result.rows_affected() == 0 {
            return Err(StoreError::NotFound(id));
        }

        self.get(id).await?.ok_or(StoreError::NotFound(id))
    }

    pub async fn delete(&self, id: i64) -> Result<bool, StoreError> {
        let result = sqlx::query("DELETE FROM channels WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn set_enabled(&self, id: i64, enabled: bool) -> Result<bool, StoreError> {
        let result = sqlx::query(
            "UPDATE channels SET enabled = ?, updated_at = datetime('now') WHERE id = ?",
        )
        .bind(enabled)
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }
}

#[derive(Debug, FromRow)]
struct ChannelRow {
    id: i64,
    name: String,
    kind: String,
    base_url: Option<String>,
    api_key: Option<String>,
    models: String,
    voices: String,
    voice_namespace: Option<String>,
    priority: i64,
    weight: i64,
    enabled: bool,
}

impl TryFrom<ChannelRow> for Channel {
    type Error = StoreError;

    fn try_from(row: ChannelRow) -> Result<Self, Self::Error> {
        let kind = kind_from_str(&row.kind).ok_or_else(|| {
            StoreError::InvalidData(format!(
                "channel {} has unknown kind {:?}",
                row.id, row.kind
            ))
        })?;
        let models = serde_json::from_str(&row.models).map_err(|error| {
            StoreError::InvalidData(format!(
                "channel {} has invalid models JSON: {error}",
                row.id
            ))
        })?;
        let voices = decode_voices(row.id, &row.voices)?;
        let weight = u32::try_from(row.weight).map_err(|_| {
            StoreError::InvalidData(format!(
                "channel {} has invalid weight {}",
                row.id, row.weight
            ))
        })?;

        Ok(Self {
            id: row.id,
            name: row.name,
            kind,
            base_url: row.base_url,
            api_key: row.api_key,
            models,
            voices,
            voice_namespace: row.voice_namespace,
            priority: row.priority,
            weight,
            enabled: row.enabled,
        })
    }
}

fn encode_models(models: &[String]) -> Result<String, StoreError> {
    serde_json::to_string(models)
        .map_err(|error| StoreError::InvalidData(format!("cannot encode models: {error}")))
}

fn encode_voices(voices: &[String]) -> Result<String, StoreError> {
    serde_json::to_string(voices)
        .map_err(|error| StoreError::InvalidData(format!("cannot encode voices: {error}")))
}

fn decode_voices(channel_id: i64, voices: &str) -> Result<Vec<String>, StoreError> {
    let voices = voices.trim();
    if voices.is_empty() || voices == "null" {
        return Ok(Vec::new());
    }
    serde_json::from_str(voices).map_err(|error| {
        StoreError::InvalidData(format!(
            "channel {channel_id} has invalid voices JSON: {error}"
        ))
    })
}

const fn kind_as_str(kind: ChannelKind) -> &'static str {
    match kind {
        ChannelKind::OpenAiCompatible => "openai_compatible",
        ChannelKind::EdgeTts => "edge_tts",
        ChannelKind::DoubaoTts => "doubao_tts",
    }
}

fn kind_from_str(kind: &str) -> Option<ChannelKind> {
    match kind {
        "openai_compatible" => Some(ChannelKind::OpenAiCompatible),
        "edge_tts" => Some(ChannelKind::EdgeTts),
        "doubao_tts" => Some(ChannelKind::DoubaoTts),
        _ => None,
    }
}
