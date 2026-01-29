use std::{net::SocketAddr, sync::Arc};

use subtle::ConstantTimeEq;

use anyhow::Context;
use async_graphql::http::{playground_source, GraphQLPlaygroundConfig};
use async_graphql_axum::{GraphQLRequest, GraphQLResponse};
use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        ConnectInfo, FromRequestParts, State,
    },
    http::{header::SEC_WEBSOCKET_PROTOCOL, header::AUTHORIZATION, request::Parts, HeaderMap, StatusCode},
    response::{Html, IntoResponse, Json},
    routing::{get, post},
    Router,
};
use futures::{SinkExt, StreamExt};
use serde::Serialize;
use tokio::sync::mpsc;
use tracing::warn;
use uuid::Uuid;

use crate::{
    config::Config,
    domain::{
        ErrorBody, ErrorType, IncomingMessage, LocationUpdateRequest, LogRequest, MovementState,
        OutgoingCoords, OutgoingError, OutgoingLocation, OutgoingLog, OutgoingMessage,
    },
    graphql::{build_schema, AppSchema},
    segment::{LineTopology, SegmentEstimator},
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
    segmenter: SegmentEstimator,
}

pub async fn run_server(config: Config) -> anyhow::Result<()> {
    let hub = Arc::new(TelemetryHub::new(config.ring_size));
    let storage = Storage::connect(config.database_url.clone()).await?;
    let schema = build_schema(storage.clone());

    let topology = match LineTopology::from_env_var("THQ_LINE_TOPOLOGY_PATH")? {
        Some(topo) => {
            tracing::info!(
                lines = topo.line_count(),
                "loaded line topology for segment inference"
            );
            topo
        }
        None => {
            tracing::warn!(
                "segment inference disabled: set THQ_LINE_TOPOLOGY_PATH to a JSON file mapping line_id to ordered station_id array"
            );
            LineTopology::empty()
        }
    };

    let segmenter = SegmentEstimator::new(topology.clone());

    if topology.is_empty() {
        tracing::warn!(
            "segment inference will persist NULL segment fields because no topology data is loaded; set THQ_LINE_TOPOLOGY_PATH to enable"
        );
    }

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
        segmenter: segmenter.clone(),
    };

    let app = Router::new()
        .route("/", get(ws_handler))
        .route("/ws", get(ws_handler))
        .route("/healthz", get(healthz))
        .route("/api/location", post(post_location))
        .route("/api/log", post(post_log))
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

#[derive(Serialize)]
struct ApiResponse {
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    warning: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

/// Extractor that enforces Bearer token authentication for REST API
struct Authenticated;

#[axum::async_trait]
impl FromRequestParts<AppState> for Authenticated {
    type Rejection = (StatusCode, Json<ApiResponse>);

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        if !state.auth.required {
            return Ok(Authenticated);
        }

        let expected = state.auth.token.as_ref().ok_or_else(|| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    ok: false,
                    id: None,
                    warning: None,
                    error: Some("server token is not configured".to_string()),
                }),
            )
        })?;

        let auth_header = parts
            .headers
            .get(AUTHORIZATION)
            .and_then(|v| v.to_str().ok());

        let token = auth_header
            .and_then(|h| {
                h.get(..7).and_then(|pref| {
                    if pref.eq_ignore_ascii_case("bearer ") {
                        h.get(7..)
                    } else {
                        None
                    }
                })
            })
            .ok_or_else(|| {
                (
                    StatusCode::UNAUTHORIZED,
                    Json(ApiResponse {
                        ok: false,
                        id: None,
                        warning: None,
                        error: Some("missing or invalid Authorization header".to_string()),
                    }),
                )
            })?;

        if token.as_bytes().ct_eq(expected.as_bytes()).into() {
            Ok(Authenticated)
        } else {
            Err((
                StatusCode::UNAUTHORIZED,
                Json(ApiResponse {
                    ok: false,
                    id: None,
                    warning: None,
                    error: Some("invalid auth token".to_string()),
                }),
            ))
        }
    }
}

async fn post_location(
    _auth: Authenticated,
    State(state): State<AppState>,
    Json(req): Json<LocationUpdateRequest>,
) -> impl IntoResponse {
    // Validate coordinates
    if !req.coords.latitude.is_finite() || !req.coords.longitude.is_finite() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse {
                ok: false,
                id: None,
                warning: None,
                error: Some("latitude/longitude must be finite numbers".to_string()),
            }),
        );
    }

    if req.coords.latitude.abs() > 90.0 || req.coords.longitude.abs() > 180.0 {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse {
                ok: false,
                id: None,
                warning: None,
                error: Some(format!(
                    "latitude {:.6} or longitude {:.6} is out of range",
                    req.coords.latitude, req.coords.longitude
                )),
            }),
        );
    }

    let speed = match req.coords.speed {
        Some(s) if !s.is_finite() => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ApiResponse {
                    ok: false,
                    id: None,
                    warning: None,
                    error: Some("speed must be finite".to_string()),
                }),
            );
        }
        Some(s) if s < 0.0 => None,
        other => other,
    };

    if let Some(acc) = req.coords.accuracy {
        if !acc.is_finite() {
            return (
                StatusCode::BAD_REQUEST,
                Json(ApiResponse {
                    ok: false,
                    id: None,
                    warning: None,
                    error: Some("accuracy must be finite".to_string()),
                }),
            );
        }
        if acc < 0.0 {
            return (
                StatusCode::BAD_REQUEST,
                Json(ApiResponse {
                    ok: false,
                    id: None,
                    warning: None,
                    error: Some("accuracy must be >= 0".to_string()),
                }),
            );
        }
    }

    // station_id is only meaningful when not moving/approaching
    let station_id = if matches!(req.state, MovementState::Moving | MovementState::Approaching) {
        None
    } else {
        req.station_id
    };

    let id = req.id.unwrap_or_else(|| Uuid::new_v4().to_string());
    let loc = OutgoingLocation {
        id: id.clone(),
        device: req.device,
        state: req.state,
        station_id,
        line_id: req.line_id,
        coords: OutgoingCoords {
            latitude: req.coords.latitude,
            longitude: req.coords.longitude,
            accuracy: req.coords.accuracy,
            speed,
        },
        timestamp: req.timestamp,
        segment_id: None,
        from_station_id: None,
        to_station_id: None,
    };

    // Annotate with segment info
    let loc = state.segmenter.annotate(loc).await;

    // Broadcast to WebSocket subscribers
    let message = OutgoingMessage::LocationUpdate(loc.clone());
    match serde_json::to_string(&message) {
        Ok(serialized) => state.hub.broadcast(serialized).await,
        Err(err) => {
            tracing::error!(?err, "failed to serialize location_update message");
        }
    }

    // Store in database
    if let Err(err) = state.storage.store_location(&loc).await {
        tracing::error!(?err, "failed to persist location_update");
    }

    // Check accuracy warning
    let warning = req
        .coords
        .accuracy
        .filter(|v| *v > BAD_ACCURACY_THRESHOLD)
        .map(|acc| {
            format!(
                "reported accuracy {acc:.1}m exceeds threshold {BAD_ACCURACY_THRESHOLD:.0}m"
            )
        });

    (
        StatusCode::OK,
        Json(ApiResponse {
            ok: true,
            id: Some(id),
            warning,
            error: None,
        }),
    )
}

async fn post_log(
    _auth: Authenticated,
    State(state): State<AppState>,
    Json(req): Json<LogRequest>,
) -> impl IntoResponse {
    // Validate log message
    if req.log.message.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse {
                ok: false,
                id: None,
                warning: None,
                error: Some("log.message must not be empty".to_string()),
            }),
        );
    }

    let id = req.id.unwrap_or_else(|| Uuid::new_v4().to_string());
    let log = OutgoingLog {
        id: id.clone(),
        device: req.device,
        timestamp: req.timestamp,
        log: req.log,
    };

    // Broadcast to WebSocket subscribers
    let message = OutgoingMessage::Log(log.clone());
    match serde_json::to_string(&message) {
        Ok(serialized) => state.hub.broadcast(serialized).await,
        Err(err) => {
            tracing::error!(?err, "failed to serialize log message");
        }
    }

    // Store in database
    if let Err(err) = state.storage.store_log(&log).await {
        tracing::error!(?err, "failed to persist log message");
    }

    (
        StatusCode::OK,
        Json(ApiResponse {
            ok: true,
            id: Some(id),
            warning: None,
            error: None,
        }),
    )
}

async fn handle_socket(socket: WebSocket, peer: SocketAddr, state: AppState) {
    let hub = state.hub.clone();
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

    if token.as_bytes().ct_eq(expected.as_bytes()).into() {
        Ok(())
    } else {
        Err(AuthError::TokenMismatch)
    }
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
                tracing::info!(%client_id, device = %who, "subscriber registered");
            }
        }
    }

    Ok(())
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
    use axum::{body::Body, extract::ws::Message, http::Request};
    use hyper::body::to_bytes;
    use serde_json::{json, Value};
    use tokio::sync::mpsc;
    use tower::ServiceExt;
    use uuid::Uuid;

    fn test_state() -> AppState {
        AppState {
            hub: Arc::new(TelemetryHub::new(10)),
            storage: Storage::default(),
            auth: AuthConfig {
                token: None,
                required: false,
            },
            schema: build_schema(Storage::default()),
            segmenter: SegmentEstimator::new(LineTopology::empty()),
        }
    }

    fn test_router() -> Router {
        Router::new()
            .route("/api/location", post(post_location))
            .route("/api/log", post(post_log))
            .with_state(test_state())
    }

    #[tokio::test]
    async fn handle_text_sends_json_parse_error() {
        let hub = Arc::new(TelemetryHub::new(10));
        let (tx, mut rx) = mpsc::channel(4);
        let mut subscribed = false;

        handle_text("not-json", &hub, &tx, Uuid::new_v4(), &mut subscribed)
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

    // REST API tests

    #[tokio::test]
    async fn post_location_success() {
        let app = test_router();

        let payload = json!({
            "device": "test-device",
            "state": "moving",
            "lineId": 1,
            "coords": {
                "latitude": 35.6812,
                "longitude": 139.7671,
                "accuracy": 10.0,
                "speed": 50.0
            },
            "timestamp": 1234567890
        });

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/location")
                    .header("content-type", "application/json")
                    .body(Body::from(payload.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = to_bytes(response.into_body())
            .await
            .unwrap();
        let v: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["ok"], true);
        assert!(v["id"].is_string());
    }

    #[tokio::test]
    async fn post_location_with_custom_id() {
        let app = test_router();

        let payload = json!({
            "id": "custom-id-123",
            "device": "test-device",
            "state": "arrived",
            "stationId": 42,
            "lineId": 1,
            "coords": {
                "latitude": 35.6812,
                "longitude": 139.7671
            },
            "timestamp": 1234567890
        });

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/location")
                    .header("content-type", "application/json")
                    .body(Body::from(payload.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = to_bytes(response.into_body())
            .await
            .unwrap();
        let v: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["ok"], true);
        assert_eq!(v["id"], "custom-id-123");
    }

    #[tokio::test]
    async fn post_location_warns_on_low_accuracy() {
        let app = test_router();

        let payload = json!({
            "device": "test-device",
            "state": "moving",
            "lineId": 1,
            "coords": {
                "latitude": 35.6812,
                "longitude": 139.7671,
                "accuracy": 150.0,
                "speed": 50.0
            },
            "timestamp": 1234567890
        });

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/location")
                    .header("content-type", "application/json")
                    .body(Body::from(payload.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = to_bytes(response.into_body())
            .await
            .unwrap();
        let v: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["ok"], true);
        assert!(v["warning"].as_str().unwrap().contains("accuracy"));
    }

    #[tokio::test]
    async fn post_location_rejects_invalid_latitude() {
        let app = test_router();

        let payload = json!({
            "device": "test-device",
            "state": "moving",
            "lineId": 1,
            "coords": {
                "latitude": 91.0,
                "longitude": 139.7671
            },
            "timestamp": 1234567890
        });

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/location")
                    .header("content-type", "application/json")
                    .body(Body::from(payload.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        let body = to_bytes(response.into_body())
            .await
            .unwrap();
        let v: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["ok"], false);
        assert!(v["error"].as_str().unwrap().contains("out of range"));
    }

    #[tokio::test]
    async fn post_location_rejects_negative_accuracy() {
        let app = test_router();

        let payload = json!({
            "device": "test-device",
            "state": "moving",
            "lineId": 1,
            "coords": {
                "latitude": 35.6812,
                "longitude": 139.7671,
                "accuracy": -1.0
            },
            "timestamp": 1234567890
        });

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/location")
                    .header("content-type", "application/json")
                    .body(Body::from(payload.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        let body = to_bytes(response.into_body())
            .await
            .unwrap();
        let v: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["ok"], false);
        assert!(v["error"].as_str().unwrap().contains("accuracy"));
    }

    #[tokio::test]
    async fn post_location_drops_station_id_when_moving() {
        let state = test_state();
        let hub = state.hub.clone();
        let app = Router::new()
            .route("/api/location", post(post_location))
            .with_state(state);

        let payload = json!({
            "device": "test-device",
            "state": "moving",
            "stationId": 42,
            "lineId": 1,
            "coords": {
                "latitude": 35.6812,
                "longitude": 139.7671
            },
            "timestamp": 1234567890
        });

        let _response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/location")
                    .header("content-type", "application/json")
                    .body(Body::from(payload.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        let snapshot = hub.snapshot().await;
        assert_eq!(snapshot.len(), 1);
        let v: Value = serde_json::from_str(&snapshot[0]).unwrap();
        assert!(v["station_id"].is_null());
    }

    #[tokio::test]
    async fn post_log_success() {
        let app = test_router();

        let payload = json!({
            "device": "test-device",
            "timestamp": 1234567890,
            "log": {
                "type": "app",
                "level": "info",
                "message": "Hello, world!"
            }
        });

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/log")
                    .header("content-type", "application/json")
                    .body(Body::from(payload.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = to_bytes(response.into_body())
            .await
            .unwrap();
        let v: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["ok"], true);
        assert!(v["id"].is_string());
    }

    #[tokio::test]
    async fn post_log_rejects_empty_message() {
        let app = test_router();

        let payload = json!({
            "device": "test-device",
            "timestamp": 1234567890,
            "log": {
                "type": "app",
                "level": "info",
                "message": "   "
            }
        });

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/log")
                    .header("content-type", "application/json")
                    .body(Body::from(payload.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        let body = to_bytes(response.into_body())
            .await
            .unwrap();
        let v: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["ok"], false);
        assert!(v["error"].as_str().unwrap().contains("message"));
    }

    #[tokio::test]
    async fn post_log_broadcasts_to_hub() {
        let state = test_state();
        let hub = state.hub.clone();
        let app = Router::new()
            .route("/api/log", post(post_log))
            .with_state(state);

        let payload = json!({
            "device": "test-device",
            "timestamp": 1234567890,
            "log": {
                "type": "system",
                "level": "warn",
                "message": "Test warning"
            }
        });

        let _response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/log")
                    .header("content-type", "application/json")
                    .body(Body::from(payload.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        let snapshot = hub.snapshot().await;
        assert_eq!(snapshot.len(), 1);
        let v: Value = serde_json::from_str(&snapshot[0]).unwrap();
        assert_eq!(v["type"], "log");
        assert_eq!(v["log"]["message"], "Test warning");
    }

    // REST API auth tests

    fn auth_required_state() -> AppState {
        AppState {
            hub: Arc::new(TelemetryHub::new(10)),
            storage: Storage::default(),
            auth: AuthConfig {
                token: Some("secret-token".into()),
                required: true,
            },
            schema: build_schema(Storage::default()),
            segmenter: SegmentEstimator::new(LineTopology::empty()),
        }
    }

    fn auth_required_router() -> Router {
        Router::new()
            .route("/api/location", post(post_location))
            .route("/api/log", post(post_log))
            .with_state(auth_required_state())
    }

    #[tokio::test]
    async fn rest_api_rejects_missing_auth() {
        let app = auth_required_router();

        let payload = json!({
            "device": "test-device",
            "state": "moving",
            "lineId": 1,
            "coords": { "latitude": 35.0, "longitude": 139.0 },
            "timestamp": 123
        });

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/location")
                    .header("content-type", "application/json")
                    .body(Body::from(payload.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

        let body = to_bytes(response.into_body()).await.unwrap();
        let v: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["ok"], false);
        assert!(v["error"].as_str().unwrap().contains("Authorization"));
    }

    #[tokio::test]
    async fn rest_api_rejects_wrong_token() {
        let app = auth_required_router();

        let payload = json!({
            "device": "test-device",
            "state": "moving",
            "lineId": 1,
            "coords": { "latitude": 35.0, "longitude": 139.0 },
            "timestamp": 123
        });

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/location")
                    .header("content-type", "application/json")
                    .header("authorization", "Bearer wrong-token")
                    .body(Body::from(payload.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

        let body = to_bytes(response.into_body()).await.unwrap();
        let v: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["ok"], false);
        assert!(v["error"].as_str().unwrap().contains("invalid"));
    }

    #[tokio::test]
    async fn rest_api_accepts_correct_token() {
        let app = auth_required_router();

        let payload = json!({
            "device": "test-device",
            "state": "moving",
            "lineId": 1,
            "coords": { "latitude": 35.0, "longitude": 139.0 },
            "timestamp": 123
        });

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/location")
                    .header("content-type", "application/json")
                    .header("authorization", "Bearer secret-token")
                    .body(Body::from(payload.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = to_bytes(response.into_body()).await.unwrap();
        let v: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["ok"], true);
    }
}
