use std::{net::SocketAddr, sync::Arc};

use subtle::ConstantTimeEq;

use anyhow::Context;
use async_graphql::http::{playground_source, GraphQLPlaygroundConfig};
use async_graphql_axum::{GraphQLRequest, GraphQLResponse};
use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        ConnectInfo, State,
    },
    http::{header::AUTHORIZATION, header::SEC_WEBSOCKET_PROTOCOL, HeaderMap, StatusCode},
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
    domain::{ErrorBody, ErrorType, IncomingMessage, OutgoingError, OutgoingMessage},
    graphql::{build_schema, AppSchema, MutationAuth},
    segment::{LineTopology, SegmentEstimator},
    state::TelemetryHub,
    storage::Storage,
};

/// Per-scope shared secrets. Each token grants exactly one role:
/// - observer: WebSocket subscription only
/// - events: sendLogEvent mutation only
/// - telemetry: sendLogEvent and sendLocation mutations
#[derive(Clone)]
struct AuthConfig {
    observer_token: Option<String>,
    events_token: Option<String>,
    telemetry_token: Option<String>,
    required: bool,
}

#[derive(Clone)]
struct AppState {
    hub: Arc<TelemetryHub>,
    auth: AuthConfig,
    schema: AppSchema,
}

pub async fn run_server(config: Config) -> anyhow::Result<()> {
    let hub = Arc::new(TelemetryHub::new(config.ring_size));
    let storage = Storage::connect(config.database_url.clone()).await?;

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

    if !config.auth_required {
        warn!(
            "authentication is disabled; every client gets observer, events and telemetry access"
        );
    }

    let schema = build_schema(storage, hub.clone(), segmenter);

    let state = AppState {
        hub,
        auth: AuthConfig {
            observer_token: config.observer_auth_token.clone(),
            events_token: config.events_auth_token.clone(),
            telemetry_token: config.telemetry_auth_token.clone(),
            required: config.auth_required,
        },
        schema,
    };

    let app = Router::new()
        .route("/", get(ws_handler))
        .route("/ws", get(ws_handler))
        .route("/healthz", get(healthz))
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

async fn graphql_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    req: GraphQLRequest,
) -> GraphQLResponse {
    let auth = mutation_auth(&headers, &state.auth);
    let req = req.into_inner().data(auth);
    state.schema.execute(req).await.into()
}

async fn graphql_playground() -> impl IntoResponse {
    Html(playground_source(
        GraphQLPlaygroundConfig::new("/graphql").subscription_endpoint("/graphql"),
    ))
}

/// Resolves the mutation scopes granted by the Authorization header.
/// Queries stay open, so the result is carried into the GraphQL context
/// instead of rejecting the request here.
fn mutation_auth(headers: &HeaderMap, auth: &AuthConfig) -> MutationAuth {
    if !auth.required {
        return MutationAuth {
            can_send_events: true,
            can_send_location: true,
        };
    }

    let Some(token) = bearer_token(headers) else {
        return MutationAuth {
            can_send_events: false,
            can_send_location: false,
        };
    };

    let matches = |expected: &Option<String>| -> bool {
        expected
            .as_ref()
            .map(|e| token.as_bytes().ct_eq(e.as_bytes()).into())
            .unwrap_or(false)
    };

    let can_send_location = matches(&auth.telemetry_token);
    let can_send_events = can_send_location || matches(&auth.events_token);

    MutationAuth {
        can_send_events,
        can_send_location,
    }
}

fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    let header = headers.get(AUTHORIZATION)?.to_str().ok()?;
    header.get(..7).and_then(|pref| {
        if pref.eq_ignore_ascii_case("bearer ") {
            header.get(7..)
        } else {
            None
        }
    })
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

/// WebSocket observation is granted by the observer token only; the events
/// and telemetry tokens deliberately do not open the subscription channel.
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
    let expected = auth
        .observer_token
        .as_ref()
        .ok_or(AuthError::TokenNotConfigured)?;

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
    use axum::{body::Body, extract::ws::Message, http::Request, routing::post};
    use hyper::body::to_bytes;
    use serde_json::{json, Value};
    use tokio::sync::mpsc;
    use tower::ServiceExt;
    use uuid::Uuid;

    fn open_auth() -> AuthConfig {
        AuthConfig {
            observer_token: None,
            events_token: None,
            telemetry_token: None,
            required: false,
        }
    }

    fn scoped_auth() -> AuthConfig {
        AuthConfig {
            observer_token: Some("observer-secret".into()),
            events_token: Some("events-secret".into()),
            telemetry_token: Some("telemetry-secret".into()),
            required: true,
        }
    }

    fn state_with_auth(auth: AuthConfig) -> AppState {
        let hub = Arc::new(TelemetryHub::new(10));
        AppState {
            hub: hub.clone(),
            auth,
            schema: build_schema(
                Storage::default(),
                hub,
                SegmentEstimator::new(LineTopology::empty()),
            ),
        }
    }

    fn graphql_router(state: AppState) -> Router {
        Router::new()
            .route("/graphql", post(graphql_handler))
            .with_state(state)
    }

    fn graphql_request(query: &str, auth_header: Option<&str>) -> Request<Body> {
        let payload = json!({ "query": query });

        let mut builder = Request::builder()
            .method("POST")
            .uri("/graphql")
            .header("content-type", "application/json");
        if let Some(value) = auth_header {
            builder = builder.header("authorization", value);
        }
        builder.body(Body::from(payload.to_string())).unwrap()
    }

    fn send_log_event_request(auth_header: Option<&str>) -> Request<Body> {
        graphql_request(
            r#"mutation {
                sendLogEvent(input: {
                    device: "test-device",
                    timestamp: 1706000000000,
                    type: APP,
                    level: INFO,
                    message: "Hello, world!"
                }) { id }
            }"#,
            auth_header,
        )
    }

    fn send_location_request(auth_header: Option<&str>) -> Request<Body> {
        graphql_request(
            r#"mutation {
                sendLocation(input: {
                    device: "test-device",
                    state: MOVING,
                    lineId: 1,
                    coords: { latitude: 35.6812, longitude: 139.7671 },
                    timestamp: 1706000000000
                }) { id }
            }"#,
            auth_header,
        )
    }

    async fn body_json(response: axum::response::Response) -> Value {
        let body = to_bytes(response.into_body()).await.unwrap();
        serde_json::from_slice(&body).unwrap()
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
        let res = enforce_ws_auth(Some("thq"), &scoped_auth());
        assert_eq!(res.unwrap_err(), AuthError::MissingToken);
    }

    #[test]
    fn enforce_accepts_observer_token() {
        let res = enforce_ws_auth(Some("thq, thq-auth-observer-secret"), &scoped_auth());
        assert!(res.is_ok());
    }

    #[test]
    fn enforce_rejects_wrong_token() {
        let res = enforce_ws_auth(Some("thq, thq-auth-wrong"), &scoped_auth());
        assert_eq!(res.unwrap_err(), AuthError::TokenMismatch);
    }

    #[test]
    fn enforce_rejects_non_observer_tokens() {
        for token in ["events-secret", "telemetry-secret"] {
            let res = enforce_ws_auth(Some(&format!("thq, thq-auth-{token}")), &scoped_auth());
            assert_eq!(res.unwrap_err(), AuthError::TokenMismatch);
        }
    }

    // GraphQL mutations over HTTP

    #[tokio::test]
    async fn graphql_mutation_broadcasts_log_event_when_auth_disabled() {
        let state = state_with_auth(open_auth());
        let hub = state.hub.clone();
        let app = graphql_router(state);

        let response = app.oneshot(send_log_event_request(None)).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let v = body_json(response).await;
        assert!(v["errors"].is_null(), "errors: {}", v["errors"]);
        assert!(v["data"]["sendLogEvent"]["id"].is_string());

        let snapshot = hub.snapshot().await;
        assert_eq!(snapshot.len(), 1);
        let msg: Value = serde_json::from_str(&snapshot[0]).unwrap();
        assert_eq!(msg["type"], "log");
        assert_eq!(msg["log"]["message"], "Hello, world!");
    }

    #[tokio::test]
    async fn log_event_rejects_missing_auth() {
        let state = state_with_auth(scoped_auth());
        let hub = state.hub.clone();
        let app = graphql_router(state);

        let response = app.oneshot(send_log_event_request(None)).await.unwrap();
        let v = body_json(response).await;
        assert!(v["errors"][0]["message"]
            .as_str()
            .unwrap()
            .contains("unauthorized"));
        assert!(hub.snapshot().await.is_empty());
    }

    #[tokio::test]
    async fn log_event_accepts_events_token() {
        let state = state_with_auth(scoped_auth());
        let hub = state.hub.clone();
        let app = graphql_router(state);

        let response = app
            .oneshot(send_log_event_request(Some("Bearer events-secret")))
            .await
            .unwrap();
        let v = body_json(response).await;
        assert!(v["errors"].is_null(), "errors: {}", v["errors"]);
        assert_eq!(hub.snapshot().await.len(), 1);
    }

    #[tokio::test]
    async fn log_event_accepts_telemetry_token() {
        let state = state_with_auth(scoped_auth());
        let hub = state.hub.clone();
        let app = graphql_router(state);

        let response = app
            .oneshot(send_log_event_request(Some("Bearer telemetry-secret")))
            .await
            .unwrap();
        let v = body_json(response).await;
        assert!(v["errors"].is_null(), "errors: {}", v["errors"]);
        assert_eq!(hub.snapshot().await.len(), 1);
    }

    #[tokio::test]
    async fn log_event_rejects_observer_token() {
        let state = state_with_auth(scoped_auth());
        let hub = state.hub.clone();
        let app = graphql_router(state);

        let response = app
            .oneshot(send_log_event_request(Some("Bearer observer-secret")))
            .await
            .unwrap();
        let v = body_json(response).await;
        assert!(v["errors"][0]["message"]
            .as_str()
            .unwrap()
            .contains("unauthorized"));
        assert!(hub.snapshot().await.is_empty());
    }

    #[tokio::test]
    async fn location_accepts_telemetry_token() {
        let state = state_with_auth(scoped_auth());
        let hub = state.hub.clone();
        let app = graphql_router(state);

        let response = app
            .oneshot(send_location_request(Some("Bearer telemetry-secret")))
            .await
            .unwrap();
        let v = body_json(response).await;
        assert!(v["errors"].is_null(), "errors: {}", v["errors"]);
        assert!(v["data"]["sendLocation"]["id"].is_string());

        let snapshot = hub.snapshot().await;
        assert_eq!(snapshot.len(), 1);
        let msg: Value = serde_json::from_str(&snapshot[0]).unwrap();
        assert_eq!(msg["type"], "location_update");
    }

    #[tokio::test]
    async fn location_rejects_events_token() {
        let state = state_with_auth(scoped_auth());
        let hub = state.hub.clone();
        let app = graphql_router(state);

        let response = app
            .oneshot(send_location_request(Some("Bearer events-secret")))
            .await
            .unwrap();
        let v = body_json(response).await;
        assert!(v["errors"][0]["message"]
            .as_str()
            .unwrap()
            .contains("telemetry"));
        assert!(hub.snapshot().await.is_empty());
    }

    #[tokio::test]
    async fn location_rejects_observer_token() {
        let state = state_with_auth(scoped_auth());
        let hub = state.hub.clone();
        let app = graphql_router(state);

        let response = app
            .oneshot(send_location_request(Some("Bearer observer-secret")))
            .await
            .unwrap();
        let v = body_json(response).await;
        assert!(!v["errors"].is_null());
        assert!(hub.snapshot().await.is_empty());
    }

    #[test]
    fn mutation_auth_grants_everything_when_disabled() {
        let auth = mutation_auth(&HeaderMap::new(), &open_auth());
        assert!(auth.can_send_events);
        assert!(auth.can_send_location);
    }

    #[test]
    fn mutation_auth_denies_missing_header() {
        let auth = mutation_auth(&HeaderMap::new(), &scoped_auth());
        assert!(!auth.can_send_events);
        assert!(!auth.can_send_location);
    }
}
