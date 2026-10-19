use std::net::SocketAddr;
use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "strelitzia", version, about)]
pub struct Cli {
    #[arg(long, global = true, value_name = "FILE")]
    pub config: Option<PathBuf>,

    #[arg(long, global = true, value_name = "ADDR")]
    pub listen: Option<SocketAddr>,

    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Clone, Subcommand)]
pub enum Command {
    Serve,

    Migrate,

    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },

    Openapi {
        #[arg(long)]
        check: bool,
    },
}

#[derive(Debug, Clone, Subcommand)]
pub enum ConfigCommand {
    /// Write a configuration file with the current defaults and fresh secrets.
    Generate {
        /// Destination file.
        #[arg(long, value_name = "FILE", default_value = "config.toml")]
        output: PathBuf,

        /// Overwrite an existing file.
        #[arg(long)]
        force: bool,
    },
}

impl Cli {
    pub fn command_or_default(&self) -> Command {
        self.command.clone().unwrap_or(Command::Serve)
    }
}
