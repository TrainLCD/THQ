use std::{fs, path::PathBuf};

use anyhow::Context;
use clap::Parser;
use serde::Deserialize;

#[derive(Parser, Debug)]
#[command(
    name = "thq-server",
    version,
    about = "Standalone telemetry WebSocket server for THQ",
    author = "TrainLCD"
)]
pub struct Cli {
    /// Host interface to bind
    #[arg(long)]
    pub host: Option<String>,

    /// Port to listen on
    #[arg(long)]
    pub port: Option<u16>,

    /// Path to a TOML config file
    #[arg(long, value_name = "FILE")]
    pub config: Option<PathBuf>,

    /// Ring buffer capacity (number of latest events to keep)
    #[arg(long, value_name = "N")]
    pub ring_size: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub host: String,
    pub port: u16,
    pub ring_size: usize,
}

#[derive(Debug, Deserialize, Default)]
struct FileConfig {
    host: Option<String>,
    port: Option<u16>,
    ring_size: Option<usize>,
}

impl Config {
    pub fn from_cli(cli: Cli) -> anyhow::Result<Self> {
        let mut file_cfg = if let Some(path) = cli.config.as_ref() {
            let raw = fs::read_to_string(path)
                .with_context(|| format!("failed to read config file at {}", path.display()))?;
            toml::from_str::<FileConfig>(&raw)
                .with_context(|| format!("failed to parse config file at {}", path.display()))?
        } else {
            FileConfig::default()
        };

        if let Some(host) = cli.host {
            file_cfg.host = Some(host);
        }
        if let Some(port) = cli.port {
            file_cfg.port = Some(port);
        }
        if let Some(ring_size) = cli.ring_size {
            file_cfg.ring_size = Some(ring_size);
        }

        Ok(Config {
            host: file_cfg.host.unwrap_or_else(|| "0.0.0.0".to_string()),
            port: file_cfg.port.unwrap_or(8080),
            ring_size: file_cfg.ring_size.unwrap_or(1000).max(1),
        })
    }
}
