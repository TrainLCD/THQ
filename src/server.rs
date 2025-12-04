use std::{
    net::SocketAddr,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::Context;
use async_graphql::http::{playground_source, GraphQLPlaygroundConfig};
use async_graphql_axum::{GraphQLRequest, GraphQLResponse};
use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        ConnectInfo, State,
    },
    http::{header::SEC_WEBSOCKET_PROTOCOL, HeaderMap, StatusCode},
    response::{Html, IntoResponse},
    routing::get,
    Router,
};
use futures::{SinkExt, StreamExt};
use tokio::sync::mpsc;
use tracing::warn;
use uuid::Uuid;

use crate::{
    config::Config,
    domain::{
        Coords, ErrorBody, ErrorType, IncomingMessage, LogBody, MovementState, OutgoingCoords,
        OutgoingError, OutgoingLocation, OutgoingLog, OutgoingMessage,
    },
    graphql::{build_schema, AppSchema},
    state::TelemetryHub,
    storage::Storage,
};

const BAD_ACCURACY_THRESHOLD: f64 = 100.0; // meters

#[derive(Clone)]
struct AuthConfig {
    token: Option<String>,
    required: bool,
}

#[derive(Clone)]
struct AppState {
    hub: Arc<TelemetryHub>,
    storage: Storage,
    auth: AuthConfig,
    schema: AppSchema,
}

pub async fn run_server(config: Config) -> anyhow::Result<()> {
    let hub = Arc::new(TelemetryHub::new(config.ring_size));
    let storage = Storage::connect(config.database_url.clone()).await?;
    let schema = build_schema(storage.clone());

    if storage.enabled() {
        tracing::info!("database persistence enabled");
    } else {
        tracing::info!("database_url not set; persistence is disabled");
    }

    if !config.ws_auth_required && config.ws_auth_token.is_none() {
        warn!("websocket auth is disabled because THQ_WS_AUTH_TOKEN is not set");
    }

    let state = AppState {
        hub: hub.clone(),
        storage: storage.clone(),
        auth: AuthConfig {
            token: config.ws_auth_token.clone(),
            required: config.ws_auth_required,
        },
        schema: schema.clone(),
    };

    let app = Router::new()
        .route("/", get(ws_handler))
        .route("/ws", get(ws_handler))
        .route("/healthz", get(healthz))
        .with_state(state.clone())
        .route("/graphql", get(graphql_playground).post(graphql_handler))
        .with_state(state);

    let addr: SocketAddr = format!("{}:{}", config.host, config.port)
        .parse()
        .with_context(|| {
            format!(
                "invalid host/port combination: {}:{}",
                config.host, config.port
            )
        })?;

    tracing::info!(%addr, "thq-server listening (ws endpoint at /ws)");

    axum::Server::bind(&addr)
        .serve(app.into_make_service_with_connect_info::<SocketAddr>())
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("server error")
}

async fn ws_handler(
    ws: WebSocketUpgrade,
    headers: HeaderMap,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    let protocol_header = headers
        .get(SEC_WEBSOCKET_PROTOCOL)
        .and_then(|v| v.to_str().ok());

    if let Err(err) = enforce_ws_auth(protocol_header, &state.auth) {
        tracing::warn!(%peer, reason = err.message(), "websocket auth failed");
        return (err.status(), err.message()).into_response();
    }

    // Only echo the formal protocol name back when the client proposed it.
    let upgrade = match protocol_header.map(parse_protocol_header) {
        Some(parsed) if parsed.has_thq => ws.protocols(["thq"]),
        _ => ws,
    };

    upgrade.on_upgrade(move |socket| handle_socket(socket, peer, state))
}

async fn healthz() -> impl IntoResponse {
    StatusCode::OK
}
async fn graphql_handler(State(state): State<AppState>, req: GraphQLRequest) -> GraphQLResponse {
    state.schema.execute(req.into_inner()).await.into()
}

async fn graphql_playground() -> impl IntoResponse {
    Html(playground_source(
        GraphQLPlaygroundConfig::new("/graphql").subscription_endpoint("/graphql"),
    ))
}

async fn handle_socket(socket: WebSocket, peer: SocketAddr, state: AppState) {
    let hub = state.hub.clone();
    let storage = state.storage.clone();
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
                if let Err(err) =
                    handle_text(&text, &hub, &storage, &tx, client_id, &mut subscribed).await
                {
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

#[derive(Debug, PartialEq, Eq)]
struct ParsedProtocols {
    has_thq: bool,
    token: Option<String>,
}

fn parse_protocol_header(raw: &str) -> ParsedProtocols {
    let mut has_thq = false;
    let mut token = None;

    for entry in raw.split(',').map(|v| v.trim()).filter(|v| !v.is_empty()) {
        if entry.eq_ignore_ascii_case("thq") {
            has_thq = true;
        }

        if let Some(rest) = entry.strip_prefix("thq-auth-") {
            if token.is_none() {
                token = Some(rest.to_string());
            }
        }
    }

    ParsedProtocols { has_thq, token }
}

#[derive(Debug, PartialEq, Eq)]
enum AuthError {
    MissingHeader,
    MissingThqProtocol,
    MissingToken,
    TokenNotConfigured,
    TokenMismatch,
}

impl AuthError {
    fn status(&self) -> StatusCode {
        match self {
            AuthError::TokenNotConfigured => StatusCode::INTERNAL_SERVER_ERROR,
            _ => StatusCode::UNAUTHORIZED,
        }
    }

    fn message(&self) -> &'static str {
        match self {
            AuthError::MissingHeader => "missing Sec-WebSocket-Protocol header",
            AuthError::MissingThqProtocol => "'thq' protocol not requested",
            AuthError::MissingToken => "missing thq-auth token",
            AuthError::TokenNotConfigured => "server token is not configured",
            AuthError::TokenMismatch => "invalid websocket auth token",
        }
    }
}

fn enforce_ws_auth(header: Option<&str>, auth: &AuthConfig) -> Result<(), AuthError> {
    if !auth.required {
        return Ok(());
    }

    let raw = header.ok_or(AuthError::MissingHeader)?;
    let parsed = parse_protocol_header(raw);

    if !parsed.has_thq {
        return Err(AuthError::MissingThqProtocol);
    }

    let token = parsed.token.ok_or(AuthError::MissingToken)?;
    let expected = auth.token.as_ref().ok_or(AuthError::TokenNotConfigured)?;

    if token == *expected {
        Ok(())
    } else {
        Err(AuthError::TokenMismatch)
    }
}

async fn handle_text(
    text: &str,
    hub: &Arc<TelemetryHub>,
    storage: &Storage,
    tx: &mpsc::Sender<Message>,
    client_id: Uuid,
    subscribed: &mut bool,
) -> anyhow::Result<()> {
    let parsed: IncomingMessage = match serde_json::from_str(text) {
        Ok(val) => val,
        Err(err) => {
            tracing::warn!(%text, ?err, "failed to parse incoming JSON");
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
                match serde_json::to_string(&ack) {
                    Ok(payload) => hub.broadcast(payload).await,
                    Err(err) => {
                        tracing::error!(?err, ?ack, "failed to serialize subscribe ack");
                    }
                }
            }
        }
        IncomingMessage::LocationUpdate {
            id,
            device,
            state,
            station_id,
            line_id,
            coords,
            timestamp,
        } => {
            let warning_accuracy = coords.accuracy.filter(|v| *v > BAD_ACCURACY_THRESHOLD);
            let message =
                match normalize_location(id, device, state, station_id, line_id, coords, timestamp)
                {
                    Ok(msg) => msg,
                    Err(err) => {
                        send_error(tx, err.0, err.1).await;
                        return Ok(());
                    }
                };

            match serde_json::to_string(&message) {
                Ok(serialized) => hub.broadcast(serialized).await,
                Err(err) => {
                    tracing::error!(
                        ?err,
                        ?message,
                        "failed to serialize location_update message"
                    );
                }
            }

            if let OutgoingMessage::LocationUpdate(loc) = &message {
                if let Err(err) = storage.store_location(loc).await {
                    tracing::error!(?err, "failed to persist location_update");
                }
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

            match serde_json::to_string(&message) {
                Ok(serialized) => hub.broadcast(serialized).await,
                Err(err) => {
                    tracing::error!(?err, ?message, "failed to serialize log message");
                }
            }

            if let OutgoingMessage::Log(log) = &message {
                if let Err(err) = storage.store_log(log).await {
                    tracing::error!(?err, "failed to persist log message");
                }
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
    station_id: Option<i32>,
    line_id: i32,
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
                "accuracy must be finite".to_string(),
            ));
        }
        if acc < 0.0 {
            return Err(ValidationError(
                ErrorType::PayloadParseError,
                "accuracy must be >= 0".to_string(),
            ));
        }
    }

    // station_id is only meaningful when not moving/approaching; drop it otherwise
    let station_id = if matches!(state, MovementState::Moving | MovementState::Approaching) {
        None
    } else {
        station_id
    };

    Ok(OutgoingMessage::LocationUpdate(OutgoingLocation {
        id: id.unwrap_or_else(|| Uuid::new_v4().to_string()),
        device,
        state,
        station_id,
        line_id,
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

    match serde_json::to_string(&payload) {
        Ok(json) => {
            let _ = tx.send(Message::Text(json)).await;
        }
        Err(err) => {
            tracing::error!(?err, ?payload, "failed to serialize error payload");
        }
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
        if let Err(err) = tokio::signal::ctrl_c().await {
            tracing::warn!(?err, "failed to install ctrl+c handler; ignoring");
            futures::future::pending::<()>().await;
        }
    };

    #[cfg(unix)]
    let terminate = async {
        use tokio::signal::unix::{signal, SignalKind};
        match signal(SignalKind::terminate()) {
            Ok(mut sigterm) => {
                sigterm.recv().await;
            }
            Err(err) => {
                tracing::warn!(?err, "failed to install SIGTERM handler; ignoring");
                futures::future::pending::<()>().await;
            }
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    tracing::info!("shutdown signal received");
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::ws::Message;
    use serde_json::Value;
    use tokio::sync::mpsc;
    use uuid::Uuid;

    #[test]
    fn normalize_location_rejects_non_finite_accuracy() {
        let res = normalize_location(
            None,
            "dev".to_string(),
            MovementState::Moving,
            None,
            1,
            Coords {
                latitude: 35.0,
                longitude: 139.0,
                accuracy: Some(f64::NAN),
                speed: Some(10.0),
            },
            1,
        );

        let ValidationError(_, msg) = res.expect_err("expected validation error");
        assert_eq!(msg, "accuracy must be finite");
    }

    #[test]
    fn normalize_location_rejects_negative_accuracy() {
        let res = normalize_location(
            None,
            "dev".to_string(),
            MovementState::Moving,
            None,
            1,
            Coords {
                latitude: 35.0,
                longitude: 139.0,
                accuracy: Some(-1.0),
                speed: Some(10.0),
            },
            1,
        );

        let ValidationError(_, msg) = res.expect_err("expected validation error");
        assert_eq!(msg, "accuracy must be >= 0");
    }

    #[tokio::test]
    async fn handle_text_sends_json_parse_error() {
        let hub = Arc::new(TelemetryHub::new(10));
        let storage = Storage::default();
        let (tx, mut rx) = mpsc::channel(4);
        let mut subscribed = false;

        handle_text(
            "not-json",
            &hub,
            &storage,
            &tx,
            Uuid::new_v4(),
            &mut subscribed,
        )
        .await
        .unwrap();

        let msg = rx.recv().await.expect("expected error message");
        let Message::Text(text) = msg else {
            panic!("expected text frame");
        };
        let v: Value = serde_json::from_str(&text).expect("valid json in error payload");
        assert_eq!(v["type"], "error");
        assert_eq!(v["error"]["type"], "json_parse_error");
    }

    #[tokio::test]
    async fn location_update_is_broadcast_and_buffered() {
        let hub = Arc::new(TelemetryHub::new(10));
        let storage = Storage::default();
        let (tx, _rx) = mpsc::channel(4);
        let mut subscribed = false;

        let payload = serde_json::json!({
            "type": "location_update",
            "device": "dev",
            "state": "moving",
            "line_id": 100,
            "station_id": 9,
            "coords": {
                "latitude": 35.0,
                "longitude": 139.0,
                "accuracy": 5.0,
                "speed": 12.0
            },
            "timestamp": 123
        })
        .to_string();

        handle_text(
            &payload,
            &hub,
            &storage,
            &tx,
            Uuid::new_v4(),
            &mut subscribed,
        )
        .await
        .unwrap();

        let snapshot = hub.snapshot().await;
        assert_eq!(snapshot.len(), 1);
        let v: Value = serde_json::from_str(&snapshot[0]).expect("broadcast must be valid json");
        assert_eq!(v["type"], "location_update");
        assert_eq!(v["device"], "dev");
        assert!(v["station_id"].is_null());
    }

    #[tokio::test]
    async fn approaching_drops_station_id() {
        let hub = Arc::new(TelemetryHub::new(10));
        let storage = Storage::default();
        let (tx, _rx) = mpsc::channel(4);
        let mut subscribed = false;

        let payload = serde_json::json!({
            "type": "location_update",
            "device": "dev",
            "state": "approaching",
            "station_id": 11,
            "line_id": 100,
            "coords": {
                "latitude": 35.0,
                "longitude": 139.0,
                "accuracy": 5.0,
                "speed": 12.0
            },
            "timestamp": 123
        })
        .to_string();

        handle_text(
            &payload,
            &hub,
            &storage,
            &tx,
            Uuid::new_v4(),
            &mut subscribed,
        )
        .await
        .unwrap();

        let snapshot = hub.snapshot().await;
        assert_eq!(snapshot.len(), 1);
        let v: Value = serde_json::from_str(&snapshot[0]).expect("broadcast must be valid json");
        assert_eq!(v["state"], "approaching");
        assert!(v["station_id"].is_null());
    }

    #[tokio::test]
    async fn log_message_is_broadcast_and_buffered() {
        let hub = Arc::new(TelemetryHub::new(10));
        let storage = Storage::default();
        let (tx, _rx) = mpsc::channel(4);
        let mut subscribed = false;

        let payload = serde_json::json!({
            "type": "log",
            "device": "dev",
            "timestamp": 123,
            "log": {
                "type": "system",
                "level": "info",
                "message": "ok"
            }
        })
        .to_string();

        handle_text(
            &payload,
            &hub,
            &storage,
            &tx,
            Uuid::new_v4(),
            &mut subscribed,
        )
        .await
        .unwrap();

        let snapshot = hub.snapshot().await;
        assert_eq!(snapshot.len(), 1);
        let v: Value = serde_json::from_str(&snapshot[0]).expect("broadcast must be valid json");
        assert_eq!(v["type"], "log");
        assert_eq!(v["log"]["message"], "ok");
    }

    #[test]
    fn parses_protocol_and_token() {
        let parsed = parse_protocol_header("thq, thq-auth-abcdef");
        assert!(parsed.has_thq);
        assert_eq!(parsed.token.as_deref(), Some("abcdef"));
    }

    #[test]
    fn parses_protocol_token_in_any_order() {
        let parsed = parse_protocol_header("thq-auth-abcdef, thq");
        assert!(parsed.has_thq);
        assert_eq!(parsed.token.as_deref(), Some("abcdef"));
    }

    #[test]
    fn enforce_requires_token_when_enabled() {
        let res = enforce_ws_auth(
            Some("thq"),
            &AuthConfig {
                token: Some("secret".into()),
                required: true,
            },
        );

        assert_eq!(res.unwrap_err(), AuthError::MissingToken);
    }

    #[test]
    fn enforce_accepts_correct_token() {
        let res = enforce_ws_auth(
            Some("thq, thq-auth-secret"),
            &AuthConfig {
                token: Some("secret".into()),
                required: true,
            },
        );

        assert!(res.is_ok());
    }

    #[test]
    fn enforce_rejects_wrong_token() {
        let res = enforce_ws_auth(
            Some("thq, thq-auth-wrong"),
            &AuthConfig {
                token: Some("secret".into()),
                required: true,
            },
        );

        assert_eq!(res.unwrap_err(), AuthError::TokenMismatch);
    }
}
