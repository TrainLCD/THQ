use crate::domain::ErrorData;
use crate::domain::LocationData;
use crate::domain::LogData;
use crate::domain::TelemetryEvent;
use crate::tauri_bridge::emit_event;
use futures_util::SinkExt;
use futures_util::StreamExt;
use log::error;
use serde_json::Value;
use std::{collections::HashMap, sync::Arc};
use tauri::AppHandle;
use tokio::net::TcpStream;
use tokio::{
    net::TcpListener,
    sync::{
        mpsc::{unbounded_channel, UnboundedSender},
        RwLock,
    },
};
use tokio_tungstenite::{accept_async, tungstenite::Message, WebSocketStream};

#[derive(Default)]
struct State {
    subscribers: HashMap<String, Vec<UnboundedSender<Message>>>,
}

async fn handle_connection(
    app: &AppHandle,
    ws_stream: WebSocketStream<TcpStream>,
    state: Arc<RwLock<State>>,
) {
    let (mut write, mut read) = ws_stream.split();
    let (tx, mut rx) = unbounded_channel::<Message>();

    // 書き込み専用タスク
    tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            let _ = write.send(msg).await;
        }
    });

    // 読み取りループ
    while let Some(Ok(msg)) = read.next().await {
        if let Ok(text) = msg.to_text() {
            let value: Value = serde_json::from_str(text).unwrap();

            if let Some("subscribe") = value["type"].as_str() {
                let mut st = state.write().await;
                st.subscribers
                    .entry("ALL".to_string())
                    .or_default()
                    .push(tx.clone());
            }

            let type_value = value["type"].as_str();

            let (event, msg) = match type_value {
                Some("location_update") => {
                    let device_id_value = value["device"].as_str().unwrap();
                    let state_value = value["state"].as_str().unwrap();
                    let coords_value = value["coords"].clone();
                    let timestamp_value = value["timestamp"].clone();

                    (
                        TelemetryEvent::LocationUpdate(LocationData {
                            id: nanoid::nanoid!(),
                            lat: coords_value["latitude"].as_f64().unwrap(),
                            lon: coords_value["longitude"].as_f64().unwrap(),
                            accuracy: coords_value["accuracy"].as_f64(),
                            speed: coords_value["speed"].as_f64().unwrap(),
                            device: device_id_value.to_string(),
                            state: state_value.to_string(),
                            timestamp: timestamp_value.as_u64().unwrap(),
                        }),
                        Message::Text(
                            serde_json::json!({
                                "id": nanoid::nanoid!(),
                                "type": "location_update",
                                "device": device_id_value,
                                "state": state_value,
                                "coords": coords_value,
                                "timestamp": timestamp_value.as_u64().unwrap()
                            })
                            .to_string()
                            .into(),
                        ),
                    )
                }
                Some("log") => {
                    let device_id_value = value["device"].as_str().unwrap();
                    let timestamp_value = value["timestamp"].clone();
                    let log_value = value["log"].clone();

                    (
                        TelemetryEvent::LogUpdate(LogData {
                            id: nanoid::nanoid!(),
                            r#type: "log".to_string(),
                            timestamp: timestamp_value.as_u64().unwrap(),
                            level: log_value["level"].as_str().unwrap().to_string(),
                            message: log_value["message"].as_str().unwrap().to_string(),
                            device: device_id_value.to_string(),
                        }),
                        Message::Text(
                            serde_json::json!({
                            "id": nanoid::nanoid!(),
                            "type": "log".to_string(),
                            "timestamp": timestamp_value.as_u64().unwrap(),
                            "level": log_value["level"].to_string(),
                            "message": log_value["message"].to_string(),
                            "device": device_id_value.to_string(),
                            })
                            .to_string()
                            .into(),
                        ),
                    )
                }
                Some("subscribe") => (
                    TelemetryEvent::LogUpdate(LogData {
                        id: nanoid::nanoid!(),
                        r#type: "log".to_string(),
                        timestamp: chrono::Utc::now().timestamp_millis() as u64,
                        level: "info".to_string(),
                        message: "New subscriber added.".to_string(),
                        device: "THQ Client".to_string(),
                    }),
                    Message::Text(
                        serde_json::json!({
                        "id": nanoid::nanoid!(),
                        "type": "log".to_string(),
                        "timestamp":chrono::Utc::now().timestamp_millis() as u64,
                        "level": "info".to_string(),
                        "message": "New subscriber added.".to_string(),
                        "device": "THQ Client".to_string(),
                        })
                        .to_string()
                        .into(),
                    ),
                ),
                txt => (
                    TelemetryEvent::Error(ErrorData {
                        r#type: "unknown".to_string(),
                        raw: serde_json::json!({
                            "error": "Unknown event type",
                            "raw": txt.unwrap_or_default().to_string(),
                        }),
                    }),
                    Message::Text(
                        serde_json::json!({
                            "type": "error",
                            "raw": txt.unwrap_or_default().to_string(),
                        })
                        .to_string()
                        .into(),
                    ),
                ),
            };

            emit_event(app, &event);

            let st = state.read().await;
            if let Some(subs) = st.subscribers.get("ALL") {
                for sub_tx in subs {
                    let _ = sub_tx.send(msg.clone());
                }
            }
        }
    }
}

pub async fn start_ws_server(app: Arc<AppHandle>) -> anyhow::Result<()> {
    let listener = TcpListener::bind("0.0.0.0:8080").await?;

    let state = Arc::new(RwLock::new(State::default()));

    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let app = Arc::clone(&app);
            let state = Arc::clone(&state);

            tokio::spawn(async move {
                let app = Arc::clone(&app);

                let ws_stream = match accept_async(stream).await {
                    Ok(ws) => ws,
                    Err(e) => {
                        error!("WebSocket handshake failed: {}", e);
                        return;
                    }
                };

                handle_connection(&(*app), ws_stream, state).await;
            });
        }
    });

    Ok(())
}

// #[cfg(test)]
// mod tests {
//     use super::*;
//     use serde_json::json;

//     #[test]
//     fn test_deserialize_valid_location() {
//         let input = json!({
//             "type": "location_update",
//             "data": {
//                 "lat": 35.0,
//                 "lon": 139.0,
//                 "accuracy": 5.0,
//                 "speed": 10.0,
//                 "timestamp": 1234567890
//             }
//         });

//         let result: TelemetryEvent = serde_json::from_value(input).expect("should deserialize");

//         match result {
//             TelemetryEvent::LocationUpdate(data) => {
//                 assert_eq!(data.lat, 35.0);
//                 assert_eq!(data.lon, 139.0);
//             }
//             _ => panic!("expected location_update variant"),
//         }
//     }

//     #[test]
//     fn test_deserialize_error_event() {
//         let input = json!({
//             "type": "error",
//             "data": {
//                 "type": "accuracy_low",
//                 "raw": {"foo": "bar"}
//             }
//         });

//         let result: TelemetryEvent = serde_json::from_value(input).expect("should deserialize");

//         match result {
//             TelemetryEvent::Error(data) => {
//                 assert_eq!(data.r#type, "accuracy_low");
//                 assert_eq!(data.raw["foo"], "bar");
//             }
//             _ => panic!("expected error variant"),
//         }
//     }

//     #[test]
//     fn test_invalid_json_should_fail() {
//         let input = json!({
//             "type": "location_update",
//             "data": {
//                 "lat": "not a number",
//                 "lon": 139.0,
//                 "accuracy": 5.0,
//                 "speed": 10.0,
//                 "timestamp": 1234567890
//             }
//         });

//         let result = serde_json::from_value::<TelemetryEvent>(input);
//         assert!(result.is_err());
//     }
// }
