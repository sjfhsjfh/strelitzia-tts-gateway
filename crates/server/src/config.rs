use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use figment::Figment;
use figment::providers::{Env, Serialized};
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const ENV_PREFIX: &str = "STRELITZIA_";

/// Precedence: defaults < TOML file < STRELITZIA_* env < `--listen`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct Config {
    pub listen: SocketAddr,
    pub database: PathBuf,
    #[serde(alias = "upstream_timeout_secs")]
    pub upstream_timeout_secs: u64,
    #[serde(alias = "max_attempts")]
    pub max_attempts: u32,
    #[serde(alias = "log_level")]
    pub log_level: String,
    /// Empty or unset disables the admin routes.
    #[serde(alias = "admin_token")]
    pub admin_token: Option<String>,
    /// Empty or unset makes the server generate an ephemeral per-process value.
    #[serde(alias = "session_secret")]
    pub session_secret: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            listen: SocketAddr::from(([127, 0, 0, 1], 8080)),
            database: PathBuf::from("data/strelitzia.db"),
            upstream_timeout_secs: 60,
            max_attempts: 3,
            log_level: "info".to_owned(),
            admin_token: None,
            session_secret: None,
        }
    }
}

impl Config {
    pub fn load(
        config_path: Option<&Path>,
        listen_override: Option<SocketAddr>,
    ) -> Result<Self, ConfigError> {
        let mut figment = Figment::new();

        if let Some(path) = config_path {
            if !path.is_file() {
                return Err(ConfigError::Missing {
                    path: path.to_path_buf(),
                });
            }
            let source = std::fs::read_to_string(path).map_err(|source| ConfigError::Read {
                path: path.to_path_buf(),
                source,
            })?;
            let value =
                toml::from_str(&source).map_err(|error| ConfigError::Parse(error.to_string()))?;
            figment = figment.merge(Serialized::defaults(kebab_keys(value)));
        }

        figment = figment
            .merge(Env::prefixed(ENV_PREFIX).map(|key| key.as_str().replace('_', "-").into()));

        let mut config: Self = figment
            .extract()
            .map_err(|error| ConfigError::Parse(error.to_string()))?;

        if let Some(listen) = listen_override {
            config.listen = listen;
        }

        Ok(config)
    }
}

/// Rewrite table keys to the canonical kebab-case spelling so legacy
/// snake_case files and environment variables do not collide with aliases.
fn kebab_keys(value: toml::Value) -> toml::Value {
    match value {
        toml::Value::Table(table) => toml::Value::Table(
            table
                .into_iter()
                .map(|(key, value)| (key.replace('_', "-"), kebab_keys(value)))
                .collect(),
        ),
        toml::Value::Array(values) => {
            toml::Value::Array(values.into_iter().map(kebab_keys).collect())
        }
        value => value,
    }
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("configuration file `{path}` does not exist")]
    Missing { path: PathBuf },
    #[error("failed to read configuration file `{path}`: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid configuration: {0}")]
    Parse(String),
}
