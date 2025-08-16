use crate::{
    domain::{ErrorData, LocationData, LogData, TelemetryEvent},
    tauri_bridge::emit_event,
};
use futures::{SinkExt, StreamExt};
use log::info;
use serde_json::{json, Value};
use std::{env, sync::Arc};
use tauri::AppHandle;
use tokio::time::{sleep, Duration};
use tokio_tungstenite::connect_async;
use tungstenite::Message;

pub async fn start_ws_client(app: Arc<AppHandle>) {
    let endpoint = env::var("WEBSOCKET_ENDPOINT").expect("WEBSOCKET_ENDPOINT is not set");
    loop {
        match connect_async(&endpoint).await {
            Ok((mut ws_stream, _)) => {
                info!("Connected to remote WebSocket server");
                if let Err(e) = ws_stream
                    .send(Message::Text(
                        json!({ "type": "subscribe" }).to_string().into(),
                    ))
                    .await
                {
                    info!("Failed to send subscribe message: {}", e);
                    sleep(Duration::from_secs(5)).await;
                    continue;
                }

                while let Some(msg_result) = ws_stream.next().await {
                    match msg_result {
                        Ok(msg) => {
                            let text = match msg.to_text() {
                                Ok(t) => t,
                                Err(e) => {
                                    emit_event(
                                        &(*app),
                                        &TelemetryEvent::Error(ErrorData {
                                            r#type: "websocket_message_error".to_string(),
                                            raw: json!({ "error": e.to_string() }),
                                        }),
                                    );
                                    continue;
                                }
                            };

                            let value: Value = match serde_json::from_str(text) {
                                Ok(v) => v,
                                Err(e) => {
                                    emit_event(
                                        &(*app),
                                        &TelemetryEvent::Error(ErrorData {
                                            r#type: "json_parse_error".to_string(),
                                            raw: json!({ "error": e.to_string() }),
                                        }),
                                    );
                                    continue;
                                }
                            };

                            let value_type = value["type"].as_str().unwrap_or("");

                            let event = match value_type {
                                "location_update" => {
                                    let id_value = match value.get("id").and_then(Value::as_str) {
                                        Some(v) => v,
                                        None => continue,
                                    };
                                    let device_id_value =
                                        match value.get("device").and_then(Value::as_str) {
                                            Some(v) => v,
                                            None => continue,
                                        };
                                    let state_value =
                                        match value.get("state").and_then(Value::as_str) {
                                            Some(v) => v,
                                            None => continue,
                                        };
                                    let coords_value = value["coords"].clone();
                                    let timestamp_value = match value.get("timestamp") {
                                        Some(v) => v.clone(),
                                        None => continue,
                                    };

                                    let lat = match coords_value["latitude"].as_f64() {
                                        Some(v) => v,
                                        None => continue,
                                    };
                                    let lon = match coords_value["longitude"].as_f64() {
                                        Some(v) => v,
                                        None => continue,
                                    };
                                    let accuracy = coords_value["accuracy"].as_f64();
                                    let speed = match coords_value["speed"].as_f64() {
                                        Some(v) => v,
                                        None => continue,
                                    };
                                    let timestamp = match timestamp_value.as_u64() {
                                        Some(v) => v,
                                        None => continue,
                                    };

                                    TelemetryEvent::LocationUpdate(LocationData {
                                        id: id_value.to_string(),
                                        lat,
                                        lon,
                                        accuracy,
                                        speed,
                                        device: device_id_value.to_string(),
                                        state: state_value.to_string(),
                                        timestamp,
                                    })
                                }
                                "log" => {
                                    let id_value = match value.get("id").and_then(Value::as_str) {
                                        Some(v) => v,
                                        None => continue,
                                    };
                                    let device_id_value =
                                        match value.get("device").and_then(Value::as_str) {
                                            Some(v) => v,
                                            None => continue,
                                        };
                                    let type_value = match value.get("type").and_then(Value::as_str)
                                    {
                                        Some(v) => v,
                                        None => continue,
                                    };
                                    let unchecked_timestamp_value = match value.get("timestamp") {
                                        Some(v) => v.clone().as_u64(),
                                        None => continue,
                                    };
                                    let timestamp_value = match unchecked_timestamp_value {
                                        Some(v) => v,
                                        None => continue,
                                    };
                                    let level_value =
                                        match value.get("level").and_then(Value::as_str) {
                                            Some(v) => v,
                                            None => continue,
                                        };
                                    let message_value =
                                        match value.get("message").and_then(Value::as_str) {
                                            Some(v) => v,
                                            None => continue,
                                        };

                                    TelemetryEvent::LogUpdate(LogData {
                                        id: id_value.to_string(),
                                        r#type: type_value.to_string(),
                                        timestamp: timestamp_value,
                                        level: level_value.to_string(),
                                        message: message_value.to_string(),
                                        device: device_id_value.to_string(),
                                    })
                                }
                                t => TelemetryEvent::Error(ErrorData {
                                    r#type: "unknown".to_string(),
                                    raw: json!({
                                        "error": format!("Unknown event type: {}", t),
                                        "raw": value.to_string(),
                                    }),
                                }),
                            };

                            emit_event(&(*app), &event);
                        }
                        Err(e) => {
                            info!("WebSocket receive error: {}", e);
                            break; // 切断時は再接続
                        }
                    }
                }
                info!("WebSocket disconnected, retrying in 5 seconds...");
                sleep(Duration::from_secs(5)).await;
            }
            Err(e) => {
                info!("Failed to connect: {}. Retrying in 5 seconds...", e);
                sleep(Duration::from_secs(5)).await;
            }
        }
    }
}
