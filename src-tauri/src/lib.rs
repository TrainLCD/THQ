use log::{error, info, warn};
use std::{env, process, sync::Arc};
use tauri_plugin_cli::CliExt;

mod external;
mod ws_server;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    if dotenv::from_filename(".env.local").is_err() {
        warn!("Could not load .env.local");
    };

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

                    if server_enabled {
                        info!("WebSocket server enabled!");
                        let handle = Arc::new(app.handle().clone());
                        tauri::async_runtime::spawn(ws_server::start_ws_server(handle));
                    } else {
                        info!("WebSocket server has been disabled.");
                    }
                }
                Err(err) => {
                    error!("{:?}", err);
                    process::exit(1);
                }
            }

            // 起動時に WebSocket サーバを開始
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
