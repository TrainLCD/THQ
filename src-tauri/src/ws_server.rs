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
        mpsc::{channel, Sender},
        RwLock,
    },
    time::{interval, Duration},
};
use tokio_tungstenite::{accept_async, tungstenite::Message, WebSocketStream};

#[derive(Default)]
struct State {
    subscribers: HashMap<String, HashMap<String, Sender<Message>>>,
}

async fn handle_connection(
    app: &AppHandle,
    ws_stream: WebSocketStream<TcpStream>,
    state: Arc<RwLock<State>>,
) {
    let connection_id = nanoid::nanoid!(); // 各接続に一意のIDを生成
    let (mut write, mut read) = ws_stream.split();
    let (tx, mut rx) = channel::<Message>(1024);

    // 書き込み専用タスク
    let write_handle = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            if write.send(msg).await.is_err() {
                break; // 接続が切れた場合はループを終了
            }
        }
    });

    // 読み取りループ
    while let Some(msg_result) = read.next().await {
        let msg = match msg_result {
            Ok(msg) => msg,
            Err(_) => break, // エラーが発生した場合はループを終了
        };

        if let Ok(text) = msg.to_text() {
            let value: Value = match serde_json::from_str(text) {
                Ok(v) => v,
                Err(_) => continue, // JSONパースエラーの場合は次のメッセージを処理
            };

            if let Some("subscribe") = value["type"].as_str() {
                let mut st = state.write().await;
                st.subscribers
                    .entry("ALL".to_string())
                    .or_default()
                    .insert(connection_id.clone(), tx.clone());
            }

            let type_value = value["type"].as_str();

            let (event, msg) = match type_value {
                Some("location_update") => {
                    let device_id = match value.get("device").and_then(Value::as_str) {
                        Some(v) => v.to_string(),
                        None => continue,
                    };
                    let moving_state = match value.get("state").and_then(Value::as_str) {
                        Some(v) => v.to_string(),
                        None => continue,
                    };
                    let coords = &value["coords"];
                    let lat = match coords.get("latitude").and_then(Value::as_f64) {
                        Some(v) => v,
                        None => continue,
                    };
                    let lon = match coords.get("longitude").and_then(Value::as_f64) {
                        Some(v) => v,
                        None => continue,
                    };
                    let accuracy = coords.get("accuracy").and_then(Value::as_f64);
                    let speed = coords.get("speed").and_then(Value::as_f64).unwrap_or(0.0); // 欠損時は 0.0 を既定値とする
                    let timestamp = match value.get("timestamp").and_then(Value::as_u64) {
                        Some(v) => v,
                        None => continue,
                    };

                    (
                        TelemetryEvent::LocationUpdate(LocationData {
                            id: nanoid::nanoid!(),
                            lat,
                            lon,
                            accuracy,
                            speed,
                            device: device_id.clone(),
                            state: moving_state.clone(),
                            timestamp,
                        }),
                        Message::Text(
                            serde_json::json!({
                                "id": nanoid::nanoid!(),
                                "type": "location_update",
                                "device": device_id,
                                "state": moving_state,
                                "coords": {
                                    "latitude": lat,
                                    "longitude": lon,
                                    "accuracy": accuracy,
                                    "speed": speed
                                },
                                "timestamp": timestamp
                            })
                            .to_string()
                            .into(),
                        ),
                    )
                }
                Some("log") => {
                    let type_value = match value.get("type").and_then(Value::as_str) {
                        Some(v) => v.to_string(),
                        None => continue,
                    };
                    let device_id_value = match value.get("device").and_then(Value::as_str) {
                        Some(v) => v.to_string(),
                        None => continue,
                    };
                    let timestamp = match value.get("timestamp").and_then(Value::as_u64) {
                        Some(v) => v,
                        None => continue,
                    };
                    let log_value = &value["log"];
                    let level = match log_value.get("level").and_then(Value::as_str) {
                        Some(v) => v.to_string(),
                        None => continue,
                    };
                    let message = match log_value.get("message").and_then(Value::as_str) {
                        Some(v) => v.to_string(),
                        None => continue,
                    };

                    (
                        TelemetryEvent::LogUpdate(LogData {
                            id: nanoid::nanoid!(),
                            r#type: type_value,
                            timestamp,
                            level: level.clone(),
                            message: message.clone(),
                            device: device_id_value.clone(),
                        }),
                        Message::Text(
                            serde_json::json!({
                                "id": nanoid::nanoid!(),
                                "type": "log".to_string(),
                                "timestamp": timestamp,
                                 "level": level,
                                 "message": message,
                                 "device": device_id_value,
                            })
                            .to_string()
                            .into(),
                        ),
                    )
                }
                Some("subscribe") => (
                    TelemetryEvent::LogUpdate(LogData {
                        id: nanoid::nanoid!(),
                        r#type: "system".to_string(),
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

            let targets = {
                let st = state.read().await;
                st.subscribers
                    .get("ALL")
                    .map(|subs| subs.values().cloned().collect::<Vec<_>>())
                    .unwrap_or_default()
            };
            // ロック解放後に配信
            for sub_tx in targets {
                let _ = sub_tx.try_send(msg.clone());
            }
        }
    }

    // 接続終了時のクリーンアップ
    write_handle.abort();

    // subscribersから該当の接続を削除
    let mut st = state.write().await;
    if let Some(subs) = st.subscribers.get_mut("ALL") {
        subs.remove(&connection_id);
    }
}

pub async fn start_ws_server(app: Arc<AppHandle>) -> anyhow::Result<()> {
    let listener = TcpListener::bind("0.0.0.0:8080").await?;

    let state = Arc::new(RwLock::new(State::default()));

    // 定期的なクリーンアップタスク（切断された接続を削除）
    let cleanup_state = Arc::clone(&state);
    tokio::spawn(async move {
        let mut interval = interval(Duration::from_secs(60)); // 1分ごとにクリーンアップ
        loop {
            interval.tick().await;
            let mut st = cleanup_state.write().await;
            for (_, subs) in st.subscribers.iter_mut() {
                subs.retain(|_, tx| !tx.is_closed());
            }
        }
    });

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
