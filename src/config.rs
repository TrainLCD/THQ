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

    /// Shared secret for observers subscribing via WebSocket (Sec-WebSocket-Protocol)
    #[arg(long, env = "THQ_OBSERVER_AUTH_TOKEN", value_name = "TOKEN")]
    pub observer_auth_token: Option<String>,

    /// Shared secret allowed to send log events via GraphQL (Bearer)
    #[arg(long, env = "THQ_EVENTS_AUTH_TOKEN", value_name = "TOKEN")]
    pub events_auth_token: Option<String>,

    /// Shared secret allowed to send both log events and location updates via GraphQL (Bearer)
    #[arg(long, env = "THQ_TELEMETRY_AUTH_TOKEN", value_name = "TOKEN")]
    pub telemetry_auth_token: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub host: String,
    pub port: u16,
    pub ring_size: usize,
    pub database_url: Option<String>,
    pub observer_auth_token: Option<String>,
    pub events_auth_token: Option<String>,
    pub telemetry_auth_token: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct FileConfig {
    host: Option<String>,
    port: Option<u16>,
    ring_size: Option<usize>,
    database_url: Option<String>,
    observer_auth_token: Option<String>,
    events_auth_token: Option<String>,
    telemetry_auth_token: Option<String>,
}

/// 空文字・空白のみのトークンは未設定(None)として扱う。
/// clap は空の環境変数を Some("") として渡すため、ここで弾く。
fn normalize_token(token: Option<String>) -> Option<String> {
    token.filter(|t| !t.trim().is_empty())
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
        if let Some(observer_auth_token) = cli.observer_auth_token {
            file_cfg.observer_auth_token = Some(observer_auth_token);
        }
        if let Some(events_auth_token) = cli.events_auth_token {
            file_cfg.events_auth_token = Some(events_auth_token);
        }
        if let Some(telemetry_auth_token) = cli.telemetry_auth_token {
            file_cfg.telemetry_auth_token = Some(telemetry_auth_token);
        }

        // 空文字・空白のみは未設定として扱い、空トークンで起動して
        // 認証不能になる設定を弾く
        let observer_auth_token = normalize_token(file_cfg.observer_auth_token);
        let events_auth_token = normalize_token(file_cfg.events_auth_token);
        let telemetry_auth_token = normalize_token(file_cfg.telemetry_auth_token);

        let any_token = observer_auth_token.is_some()
            || events_auth_token.is_some()
            || telemetry_auth_token.is_some();

        if !any_token {
            anyhow::bail!(
                "no auth token is configured; set THQ_OBSERVER_AUTH_TOKEN / THQ_EVENTS_AUTH_TOKEN / THQ_TELEMETRY_AUTH_TOKEN"
            );
        }

        Ok(Config {
            host: file_cfg.host.unwrap_or_else(|| "0.0.0.0".to_string()),
            port: file_cfg.port.unwrap_or(8080),
            ring_size: file_cfg.ring_size.unwrap_or(1000).max(1),
            database_url: file_cfg.database_url,
            observer_auth_token,
            events_auth_token,
            telemetry_auth_token,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use uuid::Uuid;

    fn empty_cli() -> Cli {
        Cli {
            host: None,
            port: None,
            config: None,
            ring_size: None,
            database_url: None,
            observer_auth_token: None,
            events_auth_token: None,
            telemetry_auth_token: None,
        }
    }

    fn tmp_path(name: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("{}_{}.toml", name, Uuid::new_v4()));
        p
    }

    #[test]
    fn defaults_are_used_when_no_cli_or_file() {
        let cfg = Config::from_cli(Cli {
            observer_auth_token: Some("secret".into()),
            ..empty_cli()
        })
        .unwrap();

        assert_eq!(cfg.host, "0.0.0.0");
        assert_eq!(cfg.port, 8080);
        assert_eq!(cfg.ring_size, 1000);
        assert_eq!(cfg.observer_auth_token.as_deref(), Some("secret"));
        assert!(cfg.events_auth_token.is_none());
        assert!(cfg.telemetry_auth_token.is_none());
    }

    #[test]
    fn file_values_are_loaded() {
        let path = tmp_path("config_file_values");
        fs::write(
            &path,
            "host = '127.0.0.1'\nport = 9000\nring_size = 50\nobserver_auth_token = 'file-observer'",
        )
        .unwrap();

        let cfg = Config::from_cli(Cli {
            config: Some(path.clone()),
            ..empty_cli()
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
        fs::write(
            &path,
            "host = '0.0.0.0'\nport = 8080\nring_size = 10\nobserver_auth_token = 'file-observer'",
        )
        .unwrap();

        let cfg = Config::from_cli(Cli {
            host: Some("127.0.0.1".into()),
            port: Some(7000),
            config: Some(path.clone()),
            ring_size: Some(5),
            database_url: Some("postgres://cli/override".into()),
            observer_auth_token: Some("cli-observer".into()),
            events_auth_token: Some("cli-events".into()),
            telemetry_auth_token: Some("cli-telemetry".into()),
        })
        .unwrap();

        assert_eq!(cfg.host, "127.0.0.1");
        assert_eq!(cfg.port, 7000);
        assert_eq!(cfg.ring_size, 5);
        assert_eq!(cfg.database_url.as_deref(), Some("postgres://cli/override"));
        assert_eq!(cfg.observer_auth_token.as_deref(), Some("cli-observer"));
        assert_eq!(cfg.events_auth_token.as_deref(), Some("cli-events"));
        assert_eq!(cfg.telemetry_auth_token.as_deref(), Some("cli-telemetry"));

        let _ = fs::remove_file(path);
    }

    #[test]
    fn database_url_loaded_from_file() {
        let path = tmp_path("config_db_url");
        fs::write(
            &path,
            "database_url = 'postgres://user:pass@localhost/db'\nobserver_auth_token = 'x'",
        )
        .unwrap();

        let cfg = Config::from_cli(Cli {
            config: Some(path.clone()),
            ..empty_cli()
        })
        .unwrap();

        assert_eq!(
            cfg.database_url.as_deref(),
            Some("postgres://user:pass@localhost/db")
        );

        let _ = fs::remove_file(path);
    }

    #[test]
    fn no_tokens_is_rejected() {
        let err = Config::from_cli(empty_cli()).unwrap_err();

        assert!(err.to_string().contains("no auth token"));
    }

    #[test]
    fn empty_or_blank_tokens_are_treated_as_unset() {
        for token in ["", "   "] {
            let err = Config::from_cli(Cli {
                observer_auth_token: Some(token.into()),
                ..empty_cli()
            })
            .unwrap_err();

            assert!(err.to_string().contains("no auth token"));
        }
    }
}
