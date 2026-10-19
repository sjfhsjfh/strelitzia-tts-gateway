use std::path::Path;
use std::time::Duration;

use sqlx::SqlitePool;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};

use crate::channels::Channels;
use crate::error::StoreError;
use crate::keys::ApiKeys;
use crate::logs::Logs;

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!();

#[derive(Debug, Clone)]
pub struct Db {
    pool: SqlitePool,
}

impl Db {
    pub async fn connect(path: &Path) -> Result<Self, StoreError> {
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)?;
        }

        let options = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .foreign_keys(true)
            .busy_timeout(Duration::from_millis(5_000))
            .synchronous(SqliteSynchronous::Normal);
        let pool = SqlitePoolOptions::new().connect_with(options).await?;

        let db = Self { pool };
        db.migrate().await?;
        Ok(db)
    }

    pub async fn migrate(&self) -> Result<(), StoreError> {
        MIGRATOR.run(&self.pool).await?;
        Ok(())
    }

    pub fn channels(&self) -> Channels {
        Channels::new(self.pool.clone())
    }

    pub fn api_keys(&self) -> ApiKeys {
        ApiKeys::new(self.pool.clone())
    }

    pub fn logs(&self) -> Logs {
        Logs::new(self.pool.clone())
    }
}
