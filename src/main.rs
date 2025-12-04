mod config;
mod domain;
mod graphql;
mod server;
mod state;
mod storage;

use clap::Parser;
use config::{Cli, Config};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing();
    let cli = Cli::parse();
    let config = Config::from_cli(cli)?;
    tracing::info!(
        host = %config.host,
        port = config.port,
        db = %config.database_url.as_deref().unwrap_or("<none>"),
        ws_auth_configured = config.ws_auth_token.is_some(),
        ws_auth_required = config.ws_auth_required,
        "starting thq-server"
    );
    match server::run_server(config).await {
        Ok(()) => {
            tracing::warn!("thq-server exited normally (no error)");
            Ok(())
        }
        Err(err) => {
            tracing::error!(?err, "thq-server exited with error");
            Err(err)
        }
    }
}

fn init_tracing() {
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| "info,thq_server=debug".into());
    tracing_subscriber::fmt()
        .with_env_filter(env_filter)
        .with_target(false)
        .init();
}
