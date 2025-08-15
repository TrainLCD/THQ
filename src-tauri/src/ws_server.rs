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
                                "level": log_value["level"].as_str().unwrap().to_string(),
                                "message": log_value["message"].as_str().unwrap().to_string(),
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
                for (_, sub_tx) in subs {
                    let _ = sub_tx.try_send(msg.clone());
                }
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
