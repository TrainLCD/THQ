use std::{sync::Arc, time::Instant};

use async_graphql::{
    Context, EmptySubscription, Enum, InputObject, Object, Result, Schema, SimpleObject, ID,
};
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use tracing::info;
use uuid::Uuid;

use crate::{
    domain::{
        BatteryState, LogBody, LogLevel, LogType, MovementState, OutgoingCoords, OutgoingLocation,
        OutgoingLog, OutgoingMessage,
    },
    segment::SegmentEstimator,
    state::TelemetryHub,
    storage::{LineAccuracyBucketRow, Storage},
};

const BAD_ACCURACY_THRESHOLD: f64 = 100.0; // meters

/// Public schema type so the server can hold and share it.
pub type AppSchema = Schema<QueryRoot, MutationRoot, EmptySubscription>;

/// Scopes granted by the HTTP-layer Bearer token check, injected per request.
///
/// - the events token only grants `can_send_events`
/// - the telemetry token grants both flags
/// - the observer token grants neither (it is WebSocket-only)
#[derive(Clone, Copy)]
pub struct MutationAuth {
    pub can_send_events: bool,
    pub can_send_location: bool,
}

const HARD_LIMIT: i32 = 2000;

#[derive(Enum, Copy, Clone, Eq, PartialEq, Debug)]
pub enum TimeBucketSize {
    Minute,
    Hour,
    Day,
}

impl TimeBucketSize {
    fn trunc_unit(self) -> &'static str {
        match self {
            TimeBucketSize::Minute => "minute",
            TimeBucketSize::Hour => "hour",
            TimeBucketSize::Day => "day",
        }
    }

    fn bucket_seconds(self) -> i64 {
        match self {
            TimeBucketSize::Minute => 60,
            TimeBucketSize::Hour => 60 * 60,
            TimeBucketSize::Day => 60 * 60 * 24,
        }
    }

    fn max_duration(self) -> ChronoDuration {
        match self {
            TimeBucketSize::Minute => ChronoDuration::days(7),
            TimeBucketSize::Hour => ChronoDuration::days(90),
            TimeBucketSize::Day => ChronoDuration::days(365),
        }
    }
}

#[derive(SimpleObject, Clone)]
pub struct LineAccuracyBucket {
    pub bucket_start: DateTime<Utc>,
    pub bucket_end: DateTime<Utc>,
    pub avg_accuracy: f64,
    pub p90_accuracy: f64,
    pub sample_count: i32,
    pub avg_speed: Option<f64>,
    pub max_speed: Option<f64>,
}

#[derive(SimpleObject, Clone)]
pub struct LineAccuracyReport {
    pub line_id: ID,
    pub buckets: Vec<LineAccuracyBucket>,
}

pub fn build_schema(
    storage: Storage,
    hub: Arc<TelemetryHub>,
    segmenter: SegmentEstimator,
) -> AppSchema {
    Schema::build(QueryRoot, MutationRoot, EmptySubscription)
        .data(storage)
        .data(hub)
        .data(segmenter)
        .finish()
}

#[derive(Default)]
pub struct QueryRoot;

#[Object]
impl QueryRoot {
    /// Aggregated accuracy metrics per line and time bucket.
    async fn accuracy_by_line(
        &self,
        ctx: &Context<'_>,
        line_id: ID,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        bucket_size: TimeBucketSize,
        #[graphql(default = 500)] limit: i32,
    ) -> Result<LineAccuracyReport> {
        let storage = ctx
            .data::<Storage>()
            .map_err(|_| "storage is not configured; DATABASE_URL is required")?;

        if !storage.enabled() {
            return Err(
                "database-backed storage is disabled; GraphQL reports are unavailable".into(),
            );
        }

        if from >= to {
            return Err("from must be earlier than to".into());
        }

        let max_span = bucket_size.max_duration();
        if to - from > max_span {
            return Err(format!(
                "requested span exceeds maximum for bucket size {:?}: max {} days",
                bucket_size,
                max_span.num_days()
            )
            .into());
        }

        let limit = limit.clamp(1, HARD_LIMIT);
        let bucket_seconds = bucket_size.bucket_seconds();
        let estimated = estimate_bucket_count(from, to, bucket_seconds);
        if estimated as i32 > HARD_LIMIT {
            return Err(format!(
                "bucket count {} would exceed hard limit {} – narrow the range or use a coarser bucket",
                estimated, HARD_LIMIT
            )
            .into());
        }

        let line_id_num: i32 = line_id
            .as_str()
            .parse()
            .map_err(|_| "lineId must be a numeric ID")?;

        let started = Instant::now();
        let rows = storage
            .fetch_line_accuracy(
                line_id_num,
                from,
                to,
                bucket_size.trunc_unit(),
                bucket_seconds,
                limit,
            )
            .await
            .map_err(|e| format!("failed to fetch accuracy report: {e}"))?;

        let duration_ms = started.elapsed().as_millis();
        info!(
            line_id = line_id.as_str(),
            bucket_size = ?bucket_size,
            bucket_count = rows.len(),
            limit,
            from = %from,
            to = %to,
            duration_ms,
            "accuracyByLine resolver completed"
        );

        Ok(LineAccuracyReport {
            line_id,
            buckets: rows.into_iter().map(LineAccuracyBucket::from).collect(),
        })
    }
}

#[derive(InputObject)]
pub struct LogEventInput {
    /// Client-generated unique session identifier (arbitrary string).
    pub session_id: String,
    /// Device identifier. Optional so events can be submitted anonymously.
    pub device: Option<String>,
    /// Unix timestamp in milliseconds.
    pub timestamp: u64,
    #[graphql(name = "type")]
    pub log_type: LogType,
    pub level: LogLevel,
    pub message: String,
}

#[derive(SimpleObject)]
pub struct SendLogEventPayload {
    pub session_id: String,
}

#[derive(InputObject)]
pub struct CoordsInput {
    pub latitude: f64,
    pub longitude: f64,
    /// Horizontal accuracy in meters.
    pub accuracy: Option<f64>,
    /// Speed in km/h.
    pub speed: Option<f64>,
}

#[derive(InputObject)]
pub struct LocationEventInput {
    /// Client-generated unique session identifier (arbitrary string).
    pub session_id: String,
    /// Device identifier.
    pub device: String,
    pub state: MovementState,
    /// Only meaningful when state is ARRIVED or PASSING; ignored otherwise.
    pub station_id: Option<i32>,
    pub line_id: i32,
    pub coords: CoordsInput,
    /// Unix timestamp in milliseconds.
    pub timestamp: u64,
    /// Battery level as a decimal (0.0 to 1.0).
    pub battery_level: Option<f64>,
    pub battery_state: Option<BatteryState>,
}

#[derive(SimpleObject)]
pub struct SendLocationPayload {
    pub session_id: String,
    /// Set when the update was accepted with a caveat (e.g. bad accuracy).
    pub warning: Option<String>,
}

pub struct MutationRoot;

#[Object]
impl MutationRoot {
    /// Submit a log event. Requires the events token or the telemetry token.
    /// The event is broadcast to WebSocket subscribers and persisted when a
    /// database is configured.
    async fn send_log_event(
        &self,
        ctx: &Context<'_>,
        input: LogEventInput,
    ) -> Result<SendLogEventPayload> {
        let auth = ctx
            .data::<MutationAuth>()
            .map_err(|_| "auth context is missing")?;
        if !auth.can_send_events {
            return Err(
                "unauthorized: a valid events or telemetry bearer token is required".into(),
            );
        }

        if input.session_id.trim().is_empty() {
            return Err("sessionId must not be empty".into());
        }

        if input.message.trim().is_empty() {
            return Err("message must not be empty".into());
        }

        let hub = ctx
            .data::<Arc<TelemetryHub>>()
            .map_err(|_| "telemetry hub is not configured")?;
        let storage = ctx
            .data::<Storage>()
            .map_err(|_| "storage is not configured")?;

        let log = OutgoingLog {
            id: Uuid::new_v4().to_string(),
            session_id: input.session_id,
            device: input.device,
            timestamp: input.timestamp,
            log: LogBody {
                r#type: input.log_type,
                level: input.level,
                message: input.message,
            },
        };

        match serde_json::to_string(&OutgoingMessage::Log(log.clone())) {
            Ok(serialized) => hub.broadcast(serialized).await,
            Err(err) => {
                tracing::error!(?err, "failed to serialize log message");
            }
        }

        if let Err(err) = storage.store_log(&log).await {
            tracing::error!(?err, "failed to persist log event");
        }

        Ok(SendLogEventPayload {
            session_id: log.session_id,
        })
    }

    /// Submit a location update. Requires the telemetry token; the events
    /// token is deliberately not enough to publish positional data. The
    /// update is annotated with segment information, broadcast to WebSocket
    /// subscribers and persisted when a database is configured.
    async fn send_location(
        &self,
        ctx: &Context<'_>,
        input: LocationEventInput,
    ) -> Result<SendLocationPayload> {
        let auth = ctx
            .data::<MutationAuth>()
            .map_err(|_| "auth context is missing")?;
        if !auth.can_send_location {
            return Err("unauthorized: a valid telemetry bearer token is required".into());
        }

        if input.session_id.trim().is_empty() {
            return Err("sessionId must not be empty".into());
        }

        if !input.coords.latitude.is_finite() || !input.coords.longitude.is_finite() {
            return Err("latitude/longitude must be finite numbers".into());
        }

        if input.coords.latitude.abs() > 90.0 || input.coords.longitude.abs() > 180.0 {
            return Err(format!(
                "latitude {:.6} or longitude {:.6} is out of range",
                input.coords.latitude, input.coords.longitude
            )
            .into());
        }

        let speed = match input.coords.speed {
            Some(s) if !s.is_finite() => return Err("speed must be finite".into()),
            Some(s) if s < 0.0 => None,
            other => other,
        };

        if let Some(acc) = input.coords.accuracy {
            if !acc.is_finite() {
                return Err("accuracy must be finite".into());
            }
            if acc < 0.0 {
                return Err("accuracy must be >= 0".into());
            }
        }

        if let Some(level) = input.battery_level {
            if !(0.0..=1.0).contains(&level) {
                return Err("battery_level must be between 0.0 and 1.0".into());
            }
        }

        // station_id is only meaningful when not moving/approaching
        let station_id = if matches!(
            input.state,
            MovementState::Moving | MovementState::Approaching
        ) {
            None
        } else {
            input.station_id
        };

        let hub = ctx
            .data::<Arc<TelemetryHub>>()
            .map_err(|_| "telemetry hub is not configured")?;
        let storage = ctx
            .data::<Storage>()
            .map_err(|_| "storage is not configured")?;
        let segmenter = ctx
            .data::<SegmentEstimator>()
            .map_err(|_| "segment estimator is not configured")?;

        let loc = OutgoingLocation {
            id: Uuid::new_v4().to_string(),
            session_id: input.session_id,
            device: input.device,
            state: input.state,
            station_id,
            line_id: input.line_id,
            coords: OutgoingCoords {
                latitude: input.coords.latitude,
                longitude: input.coords.longitude,
                accuracy: input.coords.accuracy,
                speed,
            },
            timestamp: input.timestamp,
            segment_id: None,
            from_station_id: None,
            to_station_id: None,
            battery_level: input.battery_level,
            battery_state: input.battery_state,
        };

        let loc = segmenter.annotate(loc).await;

        match serde_json::to_string(&OutgoingMessage::LocationUpdate(loc.clone())) {
            Ok(serialized) => hub.broadcast(serialized).await,
            Err(err) => {
                tracing::error!(?err, "failed to serialize location_update message");
            }
        }

        if let Err(err) = storage.store_location(&loc).await {
            tracing::error!(?err, "failed to persist location_update");
        }

        let warning = input
            .coords
            .accuracy
            .filter(|v| *v > BAD_ACCURACY_THRESHOLD)
            .map(|acc| {
                format!(
                    "reported accuracy {acc:.1}m exceeds threshold {BAD_ACCURACY_THRESHOLD:.0}m"
                )
            });

        Ok(SendLocationPayload {
            session_id: loc.session_id,
            warning,
        })
    }
}

impl From<LineAccuracyBucketRow> for LineAccuracyBucket {
    fn from(row: LineAccuracyBucketRow) -> Self {
        Self {
            bucket_start: row.bucket_start,
            bucket_end: row.bucket_end,
            avg_accuracy: row.avg_accuracy,
            p90_accuracy: row.p90_accuracy,
            sample_count: row.sample_count,
            avg_speed: row.avg_speed,
            max_speed: row.max_speed,
        }
    }
}

fn estimate_bucket_count(from: DateTime<Utc>, to: DateTime<Utc>, bucket_seconds: i64) -> i64 {
    let span = to - from;
    let total_secs = span.num_seconds();
    if total_secs <= 0 {
        return 0;
    }
    (total_secs + bucket_seconds - 1) / bucket_seconds
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::segment::LineTopology;

    const EVENTS_ONLY: MutationAuth = MutationAuth {
        can_send_events: true,
        can_send_location: false,
    };
    const TELEMETRY: MutationAuth = MutationAuth {
        can_send_events: true,
        can_send_location: true,
    };
    const OBSERVER: MutationAuth = MutationAuth {
        can_send_events: false,
        can_send_location: false,
    };

    fn test_schema(hub: Arc<TelemetryHub>) -> AppSchema {
        build_schema(
            Storage::default(),
            hub,
            SegmentEstimator::new(LineTopology::empty()),
        )
    }

    fn request(query: &str, auth: MutationAuth) -> async_graphql::Request {
        async_graphql::Request::new(query.to_string()).data(auth)
    }

    fn location_mutation(state: &str, extra: &str) -> String {
        format!(
            r#"mutation {{
                sendLocation(input: {{
                    sessionId: "sess-1",
                    device: "dev",
                    state: {state},
                    lineId: 1,
                    coords: {{ latitude: 35.6812, longitude: 139.7671{extra} }},
                    timestamp: 1706000000000
                }}) {{ sessionId warning }}
            }}"#
        )
    }

    #[tokio::test]
    async fn send_log_event_broadcasts_and_returns_session_id() {
        let hub = Arc::new(TelemetryHub::new(10));
        let schema = test_schema(hub.clone());

        let resp = schema
            .execute(request(
                r#"mutation {
                    sendLogEvent(input: {
                        sessionId: "sess-abc",
                        device: "dev",
                        timestamp: 1706000000000,
                        type: APP,
                        level: INFO,
                        message: "hello"
                    }) { sessionId }
                }"#,
                EVENTS_ONLY,
            ))
            .await;

        assert!(resp.errors.is_empty(), "errors: {:?}", resp.errors);
        let data = resp.data.into_json().unwrap();
        assert_eq!(data["sendLogEvent"]["sessionId"], "sess-abc");

        let snapshot = hub.snapshot().await;
        assert_eq!(snapshot.len(), 1);
        let v: serde_json::Value = serde_json::from_str(&snapshot[0]).unwrap();
        assert_eq!(v["type"], "log");
        assert_eq!(v["session_id"], "sess-abc");
        // the event ID is always generated server-side
        assert!(!v["id"].as_str().unwrap().is_empty());
        assert_eq!(v["log"]["message"], "hello");
        assert_eq!(v["timestamp"], 1706000000000u64);
    }

    #[tokio::test]
    async fn send_log_event_accepts_anonymous_device() {
        let hub = Arc::new(TelemetryHub::new(10));
        let schema = test_schema(hub.clone());

        let resp = schema
            .execute(request(
                r#"mutation {
                    sendLogEvent(input: {
                        sessionId: "sess-anon",
                        timestamp: 1,
                        type: APP,
                        level: INFO,
                        message: "anonymous hello"
                    }) { sessionId }
                }"#,
                EVENTS_ONLY,
            ))
            .await;

        assert!(resp.errors.is_empty(), "errors: {:?}", resp.errors);

        let snapshot = hub.snapshot().await;
        assert_eq!(snapshot.len(), 1);
        let v: serde_json::Value = serde_json::from_str(&snapshot[0]).unwrap();
        assert!(v["device"].is_null());
        assert_eq!(v["log"]["message"], "anonymous hello");
    }

    #[tokio::test]
    async fn send_location_requires_device() {
        let hub = Arc::new(TelemetryHub::new(10));
        let schema = test_schema(hub.clone());

        let resp = schema
            .execute(request(
                r#"mutation {
                    sendLocation(input: {
                        sessionId: "sess-1",
                        state: MOVING,
                        lineId: 1,
                        coords: { latitude: 35.6812, longitude: 139.7671 },
                        timestamp: 1
                    }) { sessionId }
                }"#,
                TELEMETRY,
            ))
            .await;

        // device is mandatory for positional data, so this fails schema validation
        assert!(!resp.errors.is_empty());
        assert!(resp.errors[0].message.contains("device"));
        assert!(hub.snapshot().await.is_empty());
    }

    #[tokio::test]
    async fn send_log_event_rejects_empty_session_id() {
        let hub = Arc::new(TelemetryHub::new(10));
        let schema = test_schema(hub.clone());

        let resp = schema
            .execute(request(
                r#"mutation {
                    sendLogEvent(input: {
                        sessionId: "   ",
                        device: "dev",
                        timestamp: 1,
                        type: SYSTEM,
                        level: WARN,
                        message: "hi"
                    }) { sessionId }
                }"#,
                TELEMETRY,
            ))
            .await;

        assert!(!resp.errors.is_empty());
        assert!(resp.errors[0].message.contains("sessionId"));
        assert!(hub.snapshot().await.is_empty());
    }

    #[tokio::test]
    async fn send_log_event_rejects_empty_message() {
        let hub = Arc::new(TelemetryHub::new(10));
        let schema = test_schema(hub.clone());

        let resp = schema
            .execute(request(
                r#"mutation {
                    sendLogEvent(input: {
                        sessionId: "sess-1",
                        device: "dev",
                        timestamp: 1,
                        type: APP,
                        level: INFO,
                        message: "   "
                    }) { sessionId }
                }"#,
                EVENTS_ONLY,
            ))
            .await;

        assert!(!resp.errors.is_empty());
        assert!(resp.errors[0].message.contains("message"));
        assert!(hub.snapshot().await.is_empty());
    }

    #[tokio::test]
    async fn send_log_event_rejects_observer_scope() {
        let hub = Arc::new(TelemetryHub::new(10));
        let schema = test_schema(hub.clone());

        let resp = schema
            .execute(request(
                r#"mutation {
                    sendLogEvent(input: {
                        sessionId: "sess-1",
                        device: "dev",
                        timestamp: 1,
                        type: APP,
                        level: INFO,
                        message: "hi"
                    }) { sessionId }
                }"#,
                OBSERVER,
            ))
            .await;

        assert!(!resp.errors.is_empty());
        assert!(resp.errors[0].message.contains("unauthorized"));
        assert!(hub.snapshot().await.is_empty());
    }

    #[tokio::test]
    async fn send_location_broadcasts_with_telemetry_scope() {
        let hub = Arc::new(TelemetryHub::new(10));
        let schema = test_schema(hub.clone());

        let resp = schema
            .execute(request(
                &location_mutation("MOVING", ", speed: 50.0"),
                TELEMETRY,
            ))
            .await;

        assert!(resp.errors.is_empty(), "errors: {:?}", resp.errors);
        let data = resp.data.into_json().unwrap();
        assert_eq!(data["sendLocation"]["sessionId"], "sess-1");
        assert!(data["sendLocation"]["warning"].is_null());

        let snapshot = hub.snapshot().await;
        assert_eq!(snapshot.len(), 1);
        let v: serde_json::Value = serde_json::from_str(&snapshot[0]).unwrap();
        assert_eq!(v["type"], "location_update");
        assert_eq!(v["coords"]["speed"], 50.0);
    }

    #[tokio::test]
    async fn send_location_rejects_events_only_scope() {
        let hub = Arc::new(TelemetryHub::new(10));
        let schema = test_schema(hub.clone());

        let resp = schema
            .execute(request(&location_mutation("MOVING", ""), EVENTS_ONLY))
            .await;

        assert!(!resp.errors.is_empty());
        assert!(resp.errors[0].message.contains("telemetry"));
        assert!(hub.snapshot().await.is_empty());
    }

    #[tokio::test]
    async fn send_location_rejects_invalid_latitude() {
        let hub = Arc::new(TelemetryHub::new(10));
        let schema = test_schema(hub.clone());

        let resp = schema
            .execute(request(
                r#"mutation {
                    sendLocation(input: {
                        sessionId: "sess-1",
                        device: "dev",
                        state: MOVING,
                        lineId: 1,
                        coords: { latitude: 91.0, longitude: 139.7671 },
                        timestamp: 1
                    }) { sessionId }
                }"#,
                TELEMETRY,
            ))
            .await;

        assert!(!resp.errors.is_empty());
        assert!(resp.errors[0].message.contains("out of range"));
        assert!(hub.snapshot().await.is_empty());
    }

    #[tokio::test]
    async fn send_location_rejects_negative_accuracy() {
        let hub = Arc::new(TelemetryHub::new(10));
        let schema = test_schema(hub.clone());

        let resp = schema
            .execute(request(
                &location_mutation("MOVING", ", accuracy: -1.0"),
                TELEMETRY,
            ))
            .await;

        assert!(!resp.errors.is_empty());
        assert!(resp.errors[0].message.contains("accuracy"));
        assert!(hub.snapshot().await.is_empty());
    }

    #[tokio::test]
    async fn send_location_warns_on_low_accuracy() {
        let hub = Arc::new(TelemetryHub::new(10));
        let schema = test_schema(hub);

        let resp = schema
            .execute(request(
                &location_mutation("MOVING", ", accuracy: 150.0"),
                TELEMETRY,
            ))
            .await;

        assert!(resp.errors.is_empty(), "errors: {:?}", resp.errors);
        let data = resp.data.into_json().unwrap();
        assert!(data["sendLocation"]["warning"]
            .as_str()
            .unwrap()
            .contains("accuracy"));
    }

    #[tokio::test]
    async fn send_location_drops_station_id_when_moving() {
        let hub = Arc::new(TelemetryHub::new(10));
        let schema = test_schema(hub.clone());

        let resp = schema
            .execute(request(
                r#"mutation {
                    sendLocation(input: {
                        sessionId: "sess-1",
                        device: "dev",
                        state: MOVING,
                        stationId: 42,
                        lineId: 1,
                        coords: { latitude: 35.6812, longitude: 139.7671 },
                        timestamp: 1
                    }) { sessionId }
                }"#,
                TELEMETRY,
            ))
            .await;

        assert!(resp.errors.is_empty(), "errors: {:?}", resp.errors);
        let snapshot = hub.snapshot().await;
        assert_eq!(snapshot.len(), 1);
        let v: serde_json::Value = serde_json::from_str(&snapshot[0]).unwrap();
        assert!(v["station_id"].is_null());
    }

    #[test]
    fn bucket_limits_match_spec() {
        assert_eq!(TimeBucketSize::Minute.max_duration().num_days(), 7);
        assert_eq!(TimeBucketSize::Hour.max_duration().num_days(), 90);
        assert_eq!(TimeBucketSize::Day.max_duration().num_days(), 365);
    }

    #[test]
    fn estimate_bucket_count_rounds_up() {
        let from = Utc::now();
        let to = from + ChronoDuration::seconds(61);
        assert_eq!(estimate_bucket_count(from, to, 60), 2);
    }
}
