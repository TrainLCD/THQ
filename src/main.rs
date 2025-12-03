mod config;
mod domain;
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
    server::run_server(config).await
}

fn init_tracing() {
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| "info,thq_server=debug".into());
    tracing_subscriber::fmt()
        .with_env_filter(env_filter)
        .with_target(false)
        .init();
}
