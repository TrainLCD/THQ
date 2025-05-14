use log::{error, info, warn};
use std::sync::Arc;
use std::{env, process};
use tauri_plugin_cli::CliExt;

mod domain;
mod tauri_bridge;
mod ws_client;
mod ws_server;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_log::Builder::new().build())
        .plugin(tauri_plugin_cli::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            match app.cli().matches() {
                Ok(matches) => {
                    let server_enabled = matches
                        .args
                        .get("enable-server")
                        .and_then(|arg| arg.value.as_bool())
                        .unwrap_or(false);

                    let app = Arc::new(app.handle().clone());

                    // 起動時に WebSocket サーバを開始
                    if server_enabled {
                        info!("WebSocket server enabled!");
                        tauri::async_runtime::spawn(ws_server::start_ws_server(app));
                    } else {
                        if dotenv::from_filename(".env.client.local").is_err() {
                            warn!("Could not load .env.client.local");
                        };

                        info!("Client mode enabled!");
                        tauri::async_runtime::spawn(ws_client::start_ws_client(app));
                    }
                }
                Err(err) => {
                    error!("{:?}", err);
                    process::exit(1);
                }
            }

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
