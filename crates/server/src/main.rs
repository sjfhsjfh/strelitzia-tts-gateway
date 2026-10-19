use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use clap::Parser;
use rand::RngExt;
use strelitzia_providers::{
    AdapterRegistry, DoubaoTtsAdapter, EdgeTtsAdapter, OpenAiCompatibleAdapter,
};
use strelitzia_routing::RetryPolicy;
use strelitzia_server::cli::{Cli, Command, ConfigCommand};
use strelitzia_server::config::Config;
use strelitzia_server::config_generate;
use strelitzia_server::state::AppState;
use strelitzia_server::{openapi, router};
use tokio::net::TcpListener;
use tracing_subscriber::EnvFilter;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();

    match run(cli).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

async fn run(cli: Cli) -> Result<(), Box<dyn std::error::Error>> {
    match cli.command_or_default() {
        Command::Serve => {
            let config = Config::load(cli.config.as_deref(), cli.listen)?;
            serve(config).await
        }
        Command::Migrate => {
            let config = Config::load(cli.config.as_deref(), cli.listen)?;
            let db = strelitzia_store::Db::connect(&config.database).await?;
            db.migrate().await?;
            println!("migrations applied to {}", config.database.display());
            Ok(())
        }
        Command::Config { command } => match command {
            ConfigCommand::Generate { output, force } => {
                config_generate::generate(&output, force)?;
                Ok(())
            }
        },
        Command::Openapi { check } => {
            Config::load(cli.config.as_deref(), cli.listen)?;
            openapi::write_or_check(check)?;
            Ok(())
        }
    }
}

async fn serve(config: Config) -> Result<(), Box<dyn std::error::Error>> {
    init_tracing(&config.log_level);

    // reqwest and tokio-tungstenite ship both rustls providers; install ring before any TLS client or rustls panics.
    if rustls::crypto::ring::default_provider()
        .install_default()
        .is_err()
    {
        tracing::debug!("rustls crypto provider was already installed");
    }

    let db = strelitzia_store::Db::connect(&config.database).await?;

    let client = reqwest::Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(Duration::from_secs(config.upstream_timeout_secs))
        .build()?;

    let mut adapters = AdapterRegistry::new();
    let _ = adapters.register(Arc::new(OpenAiCompatibleAdapter::new(client.clone())));
    let _ = adapters.register(Arc::new(DoubaoTtsAdapter::new(client.clone())));
    match EdgeTtsAdapter::with_default_timeouts() {
        Ok(adapter) => {
            let _ = adapters.register(Arc::new(adapter));
        }
        Err(error) => {
            tracing::warn!(%error, "Edge TTS adapter unavailable; continuing without it");
        }
    }

    let state = AppState::new(
        db.clone(),
        adapters,
        client,
        RetryPolicy {
            max_attempts: config.max_attempts,
        },
        Duration::from_secs(config.upstream_timeout_secs),
    );

    let mut app = router(state);
    match config
        .admin_token
        .as_deref()
        .map(str::trim)
        .filter(|token| !token.is_empty())
    {
        Some(token) => {
            let session_secret = match config
                .session_secret
                .as_deref()
                .map(str::trim)
                .filter(|secret| !secret.is_empty())
            {
                Some(secret) => secret.to_owned(),
                None => {
                    tracing::warn!(
                        "STRELITZIA_SESSION_SECRET not set; generating an ephemeral per-process session secret"
                    );
                    let mut bytes = [0u8; 32];
                    rand::rng().fill(&mut bytes);
                    hex::encode(bytes)
                }
            };
            let admin_state = strelitzia_admin::AdminState::new(db, token);
            app = app.nest(
                "/admin",
                strelitzia_admin::router_with_session_secret(admin_state, &session_secret),
            );
        }
        None => tracing::warn!("admin disabled: STRELITZIA_ADMIN_TOKEN not set"),
    }

    let listener = TcpListener::bind(config.listen).await?;
    tracing::info!(address = %config.listen, database = %config.database.display(), "listening");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
}

fn init_tracing(level: &str) {
    let filter = EnvFilter::try_new(level).unwrap_or_else(|error| {
        eprintln!("warning: invalid log level `{level}` ({error}); using `info`");
        EnvFilter::new("info")
    });

    if let Err(error) = tracing_subscriber::fmt().with_env_filter(filter).try_init() {
        eprintln!("warning: failed to initialize tracing: {error}");
    }
}

async fn shutdown_signal() {
    let ctrl_c = async {
        if let Err(error) = tokio::signal::ctrl_c().await {
            tracing::error!(%error, "failed to listen for Ctrl+C");
        }
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(error) => tracing::error!(%error, "failed to listen for SIGTERM"),
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }

    tracing::info!("shutdown signal received");
}
