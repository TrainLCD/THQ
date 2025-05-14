use std::sync::Arc;

use crate::domain::ErrorData;
use crate::domain::LocationData;
use crate::domain::LogData;
use crate::domain::TelemetryEvent;
use crate::tauri_bridge::emit_event;
use futures_util::StreamExt;
use log::info;
use serde_json::Value;
use std::env;
use tauri::AppHandle;
use tokio_tungstenite::connect_async;

pub async fn start_ws_client(app: Arc<AppHandle>) {
    let (mut ws_stream, _) =
        connect_async(env::var("WEBSOCKET_ENDPOINT").expect("WEBSOCKET_ENDPOINT is not set"))
            .await
            .expect("Failed to connect");

    info!("Connected to remote WebSocket server");

    if let Some(Ok(msg)) = ws_stream.next().await {
        let value: Value = serde_json::from_str(msg.to_text().unwrap()).unwrap();

        let value_type = value["type"].as_str().unwrap();

        let event = match value_type {
            "location_update" => {
                let device_id_value = value["device"].as_str().unwrap();
                let state_value = value["state"].as_str().unwrap();
                let coords_value = value["coords"].clone();
                let timestamp_value = value["timestamp"].clone();

                TelemetryEvent::LocationUpdate(LocationData {
                    id: nanoid::nanoid!(),
                    lat: coords_value["latitude"].as_f64().unwrap(),
                    lon: coords_value["longitude"].as_f64().unwrap(),
                    accuracy: coords_value["accuracy"].as_f64(),
                    speed: coords_value["speed"].as_f64().unwrap(),
                    device: device_id_value.to_string(),
                    state: state_value.to_string(),
                    timestamp: timestamp_value.as_u64().unwrap(),
                })
            }
            "log" => {
                let timestamp_value = value["timestamp"].clone();
                let level_value = value["level"].as_str().unwrap();
                let message_value = value["message"].as_str().unwrap();
                let device_id_value = value["device"].as_str().unwrap();

                TelemetryEvent::LogUpdate(LogData {
                    id: nanoid::nanoid!(),
                    timestamp: timestamp_value.as_u64().unwrap(),
                    level: level_value.to_string(),
                    message: message_value.to_string(),
                    device: device_id_value.to_string(),
                })
            }
            txt => TelemetryEvent::Error(ErrorData {
                r#type: "unknown".to_string(),
                raw: serde_json::json!({
                    "error": "Unknown event type",
                    "raw": txt.to_string(),
                }),
            }),
        };

        emit_event(app.as_ref(), &event);
    }
}
