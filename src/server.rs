use std::{
    net::SocketAddr,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::Context;
use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        ConnectInfo, State,
    },
    http::StatusCode,
    response::IntoResponse,
    routing::get,
    Router,
};
use futures::{SinkExt, StreamExt};
use tokio::{net::TcpListener, sync::mpsc};
use uuid::Uuid;

use crate::{
    config::Config,
    domain::{
        Coords, ErrorBody, ErrorType, IncomingMessage, LogBody, MovementState, OutgoingCoords,
        OutgoingError, OutgoingLocation, OutgoingLog, OutgoingMessage,
    },
    state::TelemetryHub,
};

const BAD_ACCURACY_THRESHOLD: f64 = 100.0; // meters

pub async fn run_server(config: Config) -> anyhow::Result<()> {
    let hub = Arc::new(TelemetryHub::new(config.ring_size));

    let app = Router::new()
        .route("/", get(ws_handler))
        .route("/ws", get(ws_handler))
        .route("/healthz", get(healthz))
        .with_state(hub.clone());

    let addr: SocketAddr = format!("{}:{}", config.host, config.port)
        .parse()
        .with_context(|| {
            format!(
                "invalid host/port combination: {}:{}",
                config.host, config.port
            )
        })?;

    let listener = TcpListener::bind(addr)
        .await
        .with_context(|| format!("failed to bind on {addr}"))?;

    tracing::info!(%addr, "thq-server listening (ws endpoint at /ws)");

    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await
    .context("server error")
}

async fn ws_handler(
    ws: WebSocketUpgrade,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    State(hub): State<Arc<TelemetryHub>>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, peer, hub))
}

async fn healthz() -> impl IntoResponse {
    StatusCode::OK
}

async fn handle_socket(socket: WebSocket, peer: SocketAddr, hub: Arc<TelemetryHub>) {
    let (mut ws_tx, mut ws_rx) = socket.split();
    let (tx, mut rx) = mpsc::channel::<Message>(256);
    let client_id = Uuid::new_v4();
    let mut subscribed = false;

    let writer = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            if ws_tx.send(msg).await.is_err() {
                break;
            }
        }
    });

    tracing::info!(%peer, %client_id, "client connected");

    while let Some(msg) = ws_rx.next().await {
        match msg {
            Ok(Message::Text(text)) => {
                if let Err(err) = handle_text(&text, &hub, &tx, client_id, &mut subscribed).await {
                    tracing::warn!(%peer, ?err, "failed to handle text frame");
                }
            }
            Ok(Message::Binary(_)) => {
                send_error(
                    &tx,
                    ErrorType::WebsocketMessageError,
                    "binary frames are not supported",
                )
                .await;
            }
            Ok(Message::Ping(payload)) => {
                let _ = tx.send(Message::Pong(payload)).await;
            }
            Ok(Message::Close(_)) => break,
            Ok(Message::Pong(_)) => {}
            Err(err) => {
                tracing::warn!(%peer, ?err, "websocket receive error");
                break;
            }
        }
    }

    hub.remove_subscriber(&client_id).await;
    writer.abort();
    tracing::info!(%peer, %client_id, "client disconnected");
}

async fn handle_text(
    text: &str,
    hub: &Arc<TelemetryHub>,
    tx: &mpsc::Sender<Message>,
    client_id: Uuid,
    subscribed: &mut bool,
) -> anyhow::Result<()> {
    let parsed: IncomingMessage = match serde_json::from_str(text) {
        Ok(val) => val,
        Err(err) => {
            send_error(
                tx,
                ErrorType::JsonParseError,
                format!("failed to parse JSON: {err}"),
            )
            .await;
            return Ok(());
        }
    };

    match parsed {
        IncomingMessage::Subscribe { device } => {
            if !*subscribed {
                hub.add_subscriber(client_id, tx.clone()).await;
                *subscribed = true;

                // send snapshot first so the client catches up
                for entry in hub.snapshot().await {
                    let _ = tx.send(Message::Text(entry)).await;
                }

                let who = device.unwrap_or_else(|| "unknown-client".to_string());
                let ack = system_log(&format!("subscriber registered: {who}"));
                if let Ok(payload) = serde_json::to_string(&ack) {
                    hub.broadcast(payload).await;
                }
            }
        }
        IncomingMessage::LocationUpdate {
            id,
            device,
            state,
            coords,
            timestamp,
        } => {
            let warning_accuracy = coords.accuracy.filter(|v| *v > BAD_ACCURACY_THRESHOLD);
            let message = match normalize_location(id, device, state, coords, timestamp) {
                Ok(msg) => msg,
                Err(err) => {
                    send_error(tx, err.0, err.1).await;
                    return Ok(());
                }
            };

            if let Ok(serialized) = serde_json::to_string(&message) {
                hub.broadcast(serialized).await;
            }

            if let Some(acc) = warning_accuracy {
                send_error(
                    tx,
                    ErrorType::AccuracyLow,
                    format!(
                        "reported accuracy {acc:.1}m exceeds threshold {BAD_ACCURACY_THRESHOLD:.0}m"
                    ),
                )
                .await;
            }
        }
        IncomingMessage::Log {
            id,
            device,
            timestamp,
            log,
        } => {
            let message = match normalize_log(id, device, log, timestamp) {
                Ok(msg) => msg,
                Err(err) => {
                    send_error(tx, err.0, err.1).await;
                    return Ok(());
                }
            };

            if let Ok(serialized) = serde_json::to_string(&message) {
                hub.broadcast(serialized).await;
            }
        }
    }

    Ok(())
}

struct ValidationError(ErrorType, String);

fn normalize_location(
    id: Option<String>,
    device: String,
    state: MovementState,
    coords: Coords,
    timestamp: u64,
) -> Result<OutgoingMessage, ValidationError> {
    if !coords.latitude.is_finite() || !coords.longitude.is_finite() {
        return Err(ValidationError(
            ErrorType::InvalidCoords,
            "latitude/longitude must be finite numbers".to_string(),
        ));
    }

    if coords.latitude.abs() > 90.0 || coords.longitude.abs() > 180.0 {
        return Err(ValidationError(
            ErrorType::InvalidCoords,
            format!(
                "latitude {:.6} or longitude {:.6} is out of range",
                coords.latitude, coords.longitude
            ),
        ));
    }

    let speed = coords.speed.unwrap_or(0.0);
    if !speed.is_finite() {
        return Err(ValidationError(
            ErrorType::PayloadParseError,
            "speed must be finite".to_string(),
        ));
    }

    if let Some(acc) = coords.accuracy {
        if !acc.is_finite() {
            return Err(ValidationError(
                ErrorType::PayloadParseError,
                "accuracy must be finite and >= 0".to_string(),
            ));
        }
        if acc < 0.0 {
            return Err(ValidationError(
                ErrorType::PayloadParseError,
                "accuracy must be finite and >= 0".to_string(),
            ));
        }
    }

    Ok(OutgoingMessage::LocationUpdate(OutgoingLocation {
        id: id.unwrap_or_else(|| Uuid::new_v4().to_string()),
        device,
        state,
        coords: OutgoingCoords {
            latitude: coords.latitude,
            longitude: coords.longitude,
            accuracy: coords.accuracy,
            speed,
        },
        timestamp,
    }))
}

fn normalize_log(
    id: Option<String>,
    device: String,
    log: LogBody,
    timestamp: u64,
) -> Result<OutgoingMessage, ValidationError> {
    if log.message.trim().is_empty() {
        return Err(ValidationError(
            ErrorType::PayloadParseError,
            "log.message must not be empty".to_string(),
        ));
    }

    Ok(OutgoingMessage::Log(OutgoingLog {
        id: id.unwrap_or_else(|| Uuid::new_v4().to_string()),
        device,
        timestamp,
        log,
    }))
}

async fn send_error(tx: &mpsc::Sender<Message>, r#type: ErrorType, reason: impl Into<String>) {
    let payload = OutgoingMessage::Error(OutgoingError {
        error: ErrorBody {
            r#type,
            reason: reason.into(),
        },
    });

    if let Ok(json) = serde_json::to_string(&payload) {
        let _ = tx.send(Message::Text(json)).await;
    }
}

fn system_log(message: &str) -> OutgoingMessage {
    OutgoingMessage::Log(OutgoingLog {
        id: Uuid::new_v4().to_string(),
        device: "thq-server".to_string(),
        timestamp: now_millis(),
        log: LogBody {
            r#type: crate::domain::LogType::System,
            level: crate::domain::LogLevel::Info,
            message: message.to_string(),
        },
    })
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };

    #[cfg(unix)]
    let terminate = async {
        use tokio::signal::unix::{signal, SignalKind};
        let mut sigterm =
            signal(SignalKind::terminate()).expect("could not install SIGTERM handler");
        sigterm.recv().await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    tracing::info!("shutdown signal received");
}
