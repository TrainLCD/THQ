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

    /// PostgreSQL connection string (e.g. postgres://user:pass@host:5432/db)
    #[arg(long, env = "DATABASE_URL", value_name = "URL")]
    pub database_url: Option<String>,

    /// Shared secret token required for WebSocket auth (via Sec-WebSocket-Protocol)
    #[arg(long, env = "THQ_WS_AUTH_TOKEN", value_name = "TOKEN")]
    pub ws_auth_token: Option<String>,

    /// Whether WebSocket auth is required (true/false). Defaults to true when a token is supplied.
    #[arg(long, env = "THQ_WS_AUTH_REQUIRED")]
    pub ws_auth_required: Option<bool>,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub host: String,
    pub port: u16,
    pub ring_size: usize,
    pub database_url: Option<String>,
    pub ws_auth_token: Option<String>,
    pub ws_auth_required: bool,
}

#[derive(Debug, Deserialize, Default)]
struct FileConfig {
    host: Option<String>,
    port: Option<u16>,
    ring_size: Option<usize>,
    database_url: Option<String>,
    ws_auth_token: Option<String>,
    ws_auth_required: Option<bool>,
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
        if let Some(database_url) = cli.database_url {
            file_cfg.database_url = Some(database_url);
        }
        if let Some(ws_auth_token) = cli.ws_auth_token {
            file_cfg.ws_auth_token = Some(ws_auth_token);
        }
        if let Some(ws_auth_required) = cli.ws_auth_required {
            file_cfg.ws_auth_required = Some(ws_auth_required);
        }

        let ws_auth_required = match (file_cfg.ws_auth_required, file_cfg.ws_auth_token.as_ref()) {
            (Some(required), _) => required,
            (None, Some(_)) => true,
            (None, None) => false,
        };

        Ok(Config {
            host: file_cfg.host.unwrap_or_else(|| "0.0.0.0".to_string()),
            port: file_cfg.port.unwrap_or(8080),
            ring_size: file_cfg.ring_size.unwrap_or(1000).max(1),
            database_url: file_cfg.database_url,
            ws_auth_token: file_cfg.ws_auth_token,
            ws_auth_required,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use uuid::Uuid;

    fn tmp_path(name: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("{}_{}.toml", name, Uuid::new_v4()));
        p
    }

    #[test]
    fn defaults_are_used_when_no_cli_or_file() {
        let cfg = Config::from_cli(Cli {
            host: None,
            port: None,
            config: None,
            ring_size: None,
            database_url: None,
            ws_auth_token: None,
            ws_auth_required: None,
        })
        .unwrap();

        assert_eq!(cfg.host, "0.0.0.0");
        assert_eq!(cfg.port, 8080);
        assert_eq!(cfg.ring_size, 1000);
        assert!(cfg.ws_auth_token.is_none());
        assert!(!cfg.ws_auth_required);
    }

    #[test]
    fn file_values_are_loaded() {
        let path = tmp_path("config_file_values");
        fs::write(&path, "host = '127.0.0.1'\nport = 9000\nring_size = 50").unwrap();

        let cfg = Config::from_cli(Cli {
            host: None,
            port: None,
            config: Some(path.clone()),
            ring_size: None,
            database_url: None,
            ws_auth_token: None,
            ws_auth_required: None,
        })
        .unwrap();

        assert_eq!(cfg.host, "127.0.0.1");
        assert_eq!(cfg.port, 9000);
        assert_eq!(cfg.ring_size, 50);

        // best-effort cleanup
        let _ = fs::remove_file(path);
    }

    #[test]
    fn cli_overrides_file() {
        let path = tmp_path("config_cli_override");
        fs::write(&path, "host = '0.0.0.0'\nport = 8080\nring_size = 10").unwrap();

        let cfg = Config::from_cli(Cli {
            host: Some("127.0.0.1".into()),
            port: Some(7000),
            config: Some(path.clone()),
            ring_size: Some(5),
            database_url: Some("postgres://cli/override".into()),
            ws_auth_token: Some("cli-token".into()),
            ws_auth_required: Some(false),
        })
        .unwrap();

        assert_eq!(cfg.host, "127.0.0.1");
        assert_eq!(cfg.port, 7000);
        assert_eq!(cfg.ring_size, 5);
        assert_eq!(cfg.database_url.as_deref(), Some("postgres://cli/override"));
        assert_eq!(cfg.ws_auth_token.as_deref(), Some("cli-token"));
        assert!(!cfg.ws_auth_required);

        let _ = fs::remove_file(path);
    }

    #[test]
    fn database_url_loaded_from_file() {
        let path = tmp_path("config_db_url");
        fs::write(&path, "database_url = 'postgres://user:pass@localhost/db'").unwrap();

        let cfg = Config::from_cli(Cli {
            host: None,
            port: None,
            config: Some(path.clone()),
            ring_size: None,
            database_url: None,
            ws_auth_token: None,
            ws_auth_required: None,
        })
        .unwrap();

        assert_eq!(
            cfg.database_url.as_deref(),
            Some("postgres://user:pass@localhost/db")
        );

        let _ = fs::remove_file(path);
    }

    #[test]
    fn ws_auth_defaults_to_required_when_token_present() {
        let cfg = Config::from_cli(Cli {
            host: None,
            port: None,
            config: None,
            ring_size: None,
            database_url: None,
            ws_auth_token: Some("secret".into()),
            ws_auth_required: None,
        })
        .unwrap();

        assert!(cfg.ws_auth_required);
        assert_eq!(cfg.ws_auth_token.as_deref(), Some("secret"));
    }

    #[test]
    fn ws_auth_can_be_disabled_explicitly() {
        let cfg = Config::from_cli(Cli {
            host: None,
            port: None,
            config: None,
            ring_size: None,
            database_url: None,
            ws_auth_token: Some("secret".into()),
            ws_auth_required: Some(false),
        })
        .unwrap();

        assert!(!cfg.ws_auth_required);
        assert_eq!(cfg.ws_auth_token.as_deref(), Some("secret"));
    }
}
