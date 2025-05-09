use std::sync::Arc;

use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};
use tokio::net::TcpListener;
use tokio_tungstenite::{accept_async, tungstenite::Message};

#[derive(Deserialize)]
pub struct RawTelemetryPayload {
    r#type: String,
    pub coords: Option<RawCoords>,
    pub log: Option<RawLogPayload>,
    pub state: Option<String>,
    pub device: String,
    pub timestamp: u64,
}

#[derive(Deserialize)]
pub struct RawLogPayload {
    level: String,
    message: String,
}

#[derive(Deserialize)]
pub struct RawCoords {
    pub latitude: f64,
    pub longitude: f64,
    pub accuracy: Option<f64>,
    pub speed: f64,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum TelemetryEvent {
    #[serde(rename = "location_update")]
    LocationUpdate(LocationData),
    #[serde(rename = "error")]
    Error(ErrorData),
    #[serde(rename = "log")]
    LogUpdate(LogData),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LocationData {
    id: String,
    lat: f64,
    lon: f64,
    accuracy: Option<f64>,
    speed: f64,
    device: String,
    state: String,
    timestamp: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ErrorData {
    r#type: String,
    raw: serde_json::Value,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LogData {
    id: String,
    timestamp: u64,
    level: String,
    message: String,
    device: String,
}

fn handle_location_update(payload: RawTelemetryPayload) -> TelemetryEvent {
    let coords = payload.coords.expect("coords should be present");
    TelemetryEvent::LocationUpdate(LocationData {
        id: nanoid::nanoid!(),
        lat: coords.latitude,
        lon: coords.longitude,
        accuracy: coords.accuracy,
        speed: coords.speed,
        device: payload.device,
        state: payload.state.unwrap_or("unknown".to_string()),
        timestamp: payload.timestamp,
    })
}

fn handle_log_update(payload: RawTelemetryPayload) -> TelemetryEvent {
    let log = payload.log.expect("log should be present");
    TelemetryEvent::LogUpdate(LogData {
        id: nanoid::nanoid!(),
        timestamp: payload.timestamp,
        level: log.level,
        message: log.message,
        device: payload.device,
    })
}

fn handle_unknown_event(txt: &str) -> TelemetryEvent {
    TelemetryEvent::Error(ErrorData {
        r#type: "unknown".to_string(),
        raw: serde_json::json!({
            "error": "Unknown event type",
            "raw": txt.to_string(),
        }),
    })
}

fn emit_event(app: &AppHandle, event: &TelemetryEvent) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.emit("telemetry", event);
    }
}

fn handle_error(app: &AppHandle, err: &serde_json::Error, txt: &str) {
    let error_event = TelemetryEvent::Error(ErrorData {
        r#type: "unknown".to_string(),
        raw: serde_json::json!({
            "error": err.to_string(),
            "raw": txt.to_string(),
        }),
    });
    emit_event(app, &error_event);
}

pub async fn start_ws_server(app: Arc<AppHandle>) -> anyhow::Result<()> {
    let app = Arc::clone(&app);
    let listener = TcpListener::bind("0.0.0.0:8080").await?;

    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let app = app.clone();
            tokio::spawn(async move {
                let ws_stream = accept_async(stream).await.expect("WebSocket failed");
                let (_write, mut read) = ws_stream.split();

                while let Some(Ok(msg)) = read.next().await {
                    if let Message::Text(txt) = msg {
                        match serde_json::from_str::<RawTelemetryPayload>(&txt) {
                            Ok(payload) => {
                                let event = match payload.r#type.as_str() {
                                    "location_update" => handle_location_update(payload),
                                    "log" => handle_log_update(payload),
                                    _ => handle_unknown_event(&txt),
                                };
                                emit_event(&app, &event);
                            }
                            Err(err) => handle_error(&app, &err, &txt),
                        }
                    }
                }
            });
        }
    });

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_deserialize_valid_location() {
        let input = json!({
            "type": "location_update",
            "data": {
                "lat": 35.0,
                "lon": 139.0,
                "accuracy": 5.0,
                "speed": 10.0,
                "timestamp": 1234567890
            }
        });

        let result: TelemetryEvent = serde_json::from_value(input).expect("should deserialize");

        match result {
            TelemetryEvent::LocationUpdate(data) => {
                assert_eq!(data.lat, 35.0);
                assert_eq!(data.lon, 139.0);
            }
            _ => panic!("expected location_update variant"),
        }
    }

    #[test]
    fn test_deserialize_error_event() {
        let input = json!({
            "type": "error",
            "data": {
                "type": "accuracy_low",
                "raw": {"foo": "bar"}
            }
        });

        let result: TelemetryEvent = serde_json::from_value(input).expect("should deserialize");

        match result {
            TelemetryEvent::Error(data) => {
                assert_eq!(data.r#type, "accuracy_low");
                assert_eq!(data.raw["foo"], "bar");
            }
            _ => panic!("expected error variant"),
        }
    }

    #[test]
    fn test_invalid_json_should_fail() {
        let input = json!({
            "type": "location_update",
            "data": {
                "lat": "not a number",
                "lon": 139.0,
                "accuracy": 5.0,
                "speed": 10.0,
                "timestamp": 1234567890
            }
        });

        let result = serde_json::from_value::<TelemetryEvent>(input);
        assert!(result.is_err());
    }
}
