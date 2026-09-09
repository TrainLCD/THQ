use std::{sync::Arc, time::Instant};

use async_graphql::{
    Context, EmptySubscription, Enum, InputObject, Object, Result, Schema, SimpleObject, ID,
};
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use tracing::info;
use uuid::Uuid;

use crate::{
    domain::{
        BatteryState, Channel, LogBody, LogLevel, LogType, MovementState, OutgoingCoords,
        OutgoingInteraction, OutgoingLocation, OutgoingLog, OutgoingMessage, Platform, Properties,
    },
    freeze::{
        haversine_meters, LocationFreezeQuery, LocationFreezeRow, LocationFreezeSessionRow,
        LocationFreezeSummaryRow,
    },
    segment::SegmentEstimator,
    state::TelemetryHub,
    storage::{
        EventFilter, InteractionEventRow, LineAccuracyBucketRow, LocationEventRow, LogEventRow,
        Storage,
    },
};

const BAD_ACCURACY_THRESHOLD: f64 = 100.0; // meters

/// Public schema type so the server can hold and share it.
pub type AppSchema = Schema<QueryRoot, MutationRoot, EmptySubscription>;

/// Scopes granted by the HTTP-layer Bearer token check, injected per request.
///
/// - the events token only grants `can_send_events`
/// - the telemetry token grants `can_send_events` and `can_send_location`
/// - the observer token only grants `can_read_events` (raw history queries,
///   mirroring its WebSocket observation role)
#[derive(Clone, Copy)]
pub struct RequestAuth {
    pub can_send_events: bool,
    pub can_send_location: bool,
    pub can_read_events: bool,
}

const HARD_LIMIT: i32 = 2000;

/// Widest range the freeze queries accept, mirroring the `hour` bucket cap of
/// `accuracyByLine`.
const FREEZE_MAX_SPAN_DAYS: i64 = 90;

/// The app sends at most one location per second, so anything below this would
/// flag ordinary jitter rather than a freeze.
const FREEZE_MIN_GAP_THRESHOLD_MS: i32 = 1000;

#[derive(Enum, Copy, Clone, Eq, PartialEq, Debug)]
#[graphql(rename_items = "lowercase")]
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

/// A persisted log event, as accepted by the `sendLogEvent` mutation.
///
/// Fields that were added to the storage schema over time are nullable:
/// legacy rows recorded before the column existed return null. Enum fields
/// are also null when the stored value does not map to a known variant
/// (e.g. a row written by a newer server version).
#[derive(SimpleObject, Clone)]
pub struct LogEvent {
    /// Server-generated event ID.
    pub id: ID,
    pub session_id: Option<String>,
    /// Null when the event was submitted anonymously.
    pub device: Option<String>,
    pub app_version: Option<String>,
    pub platform: Option<Platform>,
    pub channel: Option<Channel>,
    /// Client-reported unix timestamp in milliseconds.
    pub timestamp: u64,
    #[graphql(name = "type")]
    pub log_type: Option<LogType>,
    pub level: Option<LogLevel>,
    pub message: String,
    /// Server-side time the event was persisted.
    pub recorded_at: DateTime<Utc>,
}

/// A persisted interaction event, as accepted by the `sendInteractionEvent`
/// mutation. See `LogEvent` for the nullability rules.
#[derive(SimpleObject, Clone)]
pub struct InteractionEvent {
    /// Server-generated event ID.
    pub id: ID,
    pub session_id: Option<String>,
    /// Null when the event was submitted anonymously.
    pub device: Option<String>,
    pub app_version: Option<String>,
    pub platform: Option<Platform>,
    pub channel: Option<Channel>,
    /// Client-reported unix timestamp in milliseconds.
    pub timestamp: u64,
    pub event_name: String,
    pub properties: Option<Properties>,
    /// Server-side time the event was persisted.
    pub recorded_at: DateTime<Utc>,
}

#[derive(SimpleObject, Clone)]
pub struct Coords {
    pub latitude: f64,
    pub longitude: f64,
    /// Horizontal accuracy in meters.
    pub accuracy: Option<f64>,
    /// Speed in km/h.
    pub speed: Option<f64>,
}

/// A persisted location update, as accepted by the `sendLocation` mutation,
/// including the segment annotation added by the server. See `LogEvent` for
/// the nullability rules.
#[derive(SimpleObject, Clone)]
pub struct LocationEvent {
    /// Server-generated event ID.
    pub id: ID,
    pub session_id: Option<String>,
    pub device: String,
    pub state: Option<MovementState>,
    pub station_id: Option<i32>,
    pub line_id: Option<i32>,
    pub coords: Coords,
    /// Client-reported unix timestamp in milliseconds.
    pub timestamp: u64,
    /// Segment annotation inferred by the server; null when no topology was
    /// loaded or the segment could not be estimated.
    pub segment_id: Option<String>,
    pub from_station_id: Option<i32>,
    pub to_station_id: Option<i32>,
    /// Battery level as a decimal (0.0 to 1.0).
    pub battery_level: Option<f64>,
    pub battery_state: Option<BatteryState>,
    /// Build metadata reported alongside the position. Null for rows written
    /// before the columns existed or by clients that do not send them.
    pub app_version: Option<String>,
    pub platform: Option<Platform>,
    pub channel: Option<Channel>,
    /// Server-side time the event was persisted.
    pub recorded_at: DateTime<Utc>,
}

/// Filters shared by `locationFreezes`, `locationFreezeSessions` and
/// `locationFreezeSummary`. See `docs/location-freeze-regression.md`.
#[derive(InputObject)]
pub struct LocationFreezeFilter {
    /// Inclusive lower bound on the client-reported timestamp.
    pub from: DateTime<Utc>,
    /// Exclusive upper bound on the client-reported timestamp. At most 90 days
    /// after `from`.
    pub to: DateTime<Utc>,
    pub line_id: Option<i32>,
    /// Server-assigned segment ID, matched against the row before the gap.
    pub segment_id: Option<String>,
    pub device: Option<String>,
    pub session_id: Option<String>,
    pub app_version: Option<String>,
    pub platform: Option<Platform>,
    pub channel: Option<Channel>,
    /// A location gap longer than this many milliseconds counts as a freeze
    /// candidate. The app sends at most one location per second, so the default
    /// of 60 000 is already two orders of magnitude above normal jitter.
    #[graphql(default = 60000)]
    pub gap_threshold_ms: i32,
    /// Only gaps whose preceding row reported a speed (km/h) above this count,
    /// which is what separates a freeze from a stop at a station.
    #[graphql(default = 30.0)]
    pub speed_threshold_kmh: f64,
    /// When true (the default), a gap only counts if the same session kept
    /// submitting log or interaction events while the position was missing —
    /// evidence that the app itself did not die.
    #[graphql(default = true)]
    pub require_app_alive: bool,
}

/// A single location log gap that matches the freeze signature.
#[derive(SimpleObject, Clone)]
pub struct LocationFreeze {
    pub session_id: String,
    pub device: String,
    pub line_id: Option<i32>,
    pub segment_id: Option<String>,
    pub from_station_id: Option<i32>,
    pub to_station_id: Option<i32>,
    /// Taken from `location_logs`, falling back to the log / interaction events
    /// of the same session; null when neither carries it.
    pub app_version: Option<String>,
    pub platform: Option<Platform>,
    pub channel: Option<Channel>,
    /// Client timestamp of the last row before the gap.
    pub gap_start: DateTime<Utc>,
    /// Client timestamp of the first row after the gap.
    pub gap_end: DateTime<Utc>,
    /// Length of the gap in milliseconds.
    pub gap_ms: u64,
    /// Speed (km/h) reported on the row before the gap.
    pub speed_before_gap: f64,
    /// Coordinates of the row before the gap, i.e. where the display froze.
    pub coords_before_gap: Coords,
    /// Coordinates of the first row after the gap.
    pub coords_after_gap: Coords,
    /// Great-circle distance in meters between the two rows above: roughly how
    /// far the displayed position had drifted from reality.
    pub jump_distance_meters: f64,
    /// Log + interaction events of the same session inside the gap.
    pub alive_event_count: i32,
}

/// Per-session rollup. Sessions with zero freezes are included so two builds
/// that rode the same segment can be compared side by side.
#[derive(SimpleObject, Clone)]
pub struct LocationFreezeSession {
    pub session_id: String,
    pub device: String,
    /// Lines the session had location rows on inside the window, ascending;
    /// empty when every row lacked a line.
    pub line_ids: Vec<i32>,
    pub app_version: Option<String>,
    pub platform: Option<Platform>,
    pub channel: Option<Channel>,
    /// Client timestamp of the session's first row inside the window.
    pub started_at: DateTime<Utc>,
    /// Client timestamp of the session's last row inside the window.
    pub ended_at: DateTime<Utc>,
    pub location_count: i32,
    pub max_speed: Option<f64>,
    pub freeze_count: i32,
    /// Null when the session has no freeze.
    pub max_gap_ms: Option<u64>,
    /// Zero when the session has no freeze.
    pub total_gap_ms: u64,
}

/// Rollup by line, segment, device and build. Groups with zero freezes are
/// included, so a build can be shown to be clean rather than merely absent.
#[derive(SimpleObject, Clone)]
pub struct LocationFreezeSummary {
    pub line_id: Option<i32>,
    pub segment_id: Option<String>,
    pub from_station_id: Option<i32>,
    pub to_station_id: Option<i32>,
    pub device: String,
    pub app_version: Option<String>,
    pub platform: Option<Platform>,
    pub channel: Option<Channel>,
    /// Sessions with at least one location row in this group.
    pub session_count: i32,
    pub location_count: i32,
    /// Sessions in this group with at least one freeze.
    pub freeze_session_count: i32,
    pub freeze_count: i32,
    /// Null when the group has no freeze.
    pub max_gap_ms: Option<u64>,
    /// Zero when the group has no freeze.
    pub total_gap_ms: u64,
}

/// Builds the application GraphQL schema with storage, telemetry, and segment-estimation dependencies.
///
/// # Examples
///
/// ```
/// # use std::sync::Arc;
/// # use crate::{build_schema, SegmentEstimator, Storage, TelemetryHub};
/// let schema = build_schema(
///     Storage::default(),
///     Arc::new(TelemetryHub::default()),
///     SegmentEstimator::default(),
/// );
/// ```
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
    /// Builds an accuracy report for a line over a bounded time range, grouped into time buckets.
    ///
    /// The line identifier must be numeric. The requested range and estimated bucket count are
    /// validated against the selected bucket size and hard limit.
    ///
    /// # Errors
    ///
    /// Returns an error when storage is unavailable or disabled, the time range is invalid or too
    /// large, the estimated bucket count exceeds the hard limit, the line identifier is not numeric,
    /// or the report cannot be fetched.
    ///
    /// # Examples
    ///
    /// ```
    /// let query = r#"
    ///   {
    ///     accuracyByLine(
    ///       lineId: "42",
    ///       from: "2024-01-01T00:00:00Z",
    ///       to: "2024-01-01T01:00:00Z",
    ///       bucketSize: HOUR
    ///     ) {
    ///       lineId
    ///       buckets { start end averageAccuracy }
    ///     }
    ///   }
    /// "#;
    ///
    /// assert!(query.contains("accuracyByLine"));
    /// ```
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

    /// Persisted log events, newest first. Counterpart of the
    /// `sendLogEvent` mutation. Requires the observer token.
    #[allow(clippy::too_many_arguments)] // flat filter args mirror accuracyByLine
    /// Retrieves persisted log events matching the requested filters.
    ///
    /// Access requires event-history read authorization. The time range must be valid, and
    /// the requested result limit is constrained by the history query limits.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let response = schema.execute(
    ///     "{ logEvents(sessionId: \"session-1\", limit: 100) { id message } }",
    /// ).await;
    /// assert!(response.errors.is_empty());
    /// ```
    ///
    /// # Returns
    ///
    /// The matching log events.
    async fn log_events(
        &self,
        ctx: &Context<'_>,
        session_id: Option<String>,
        device: Option<String>,
        from: Option<DateTime<Utc>>,
        to: Option<DateTime<Utc>>,
        #[graphql(name = "type")] log_type: Option<LogType>,
        level: Option<LogLevel>,
        #[graphql(default = 100)] limit: i32,
    ) -> Result<Vec<LogEvent>> {
        let (storage, filter) = history_query(ctx, session_id, device, from, to, limit)?;

        let started = Instant::now();
        let rows = storage
            .fetch_log_events(
                &filter,
                log_type.map(|t| t.as_str()),
                level.map(|l| l.as_str()),
            )
            .await
            .map_err(|e| format!("failed to fetch log events: {e}"))?;

        info!(
            count = rows.len(),
            limit = filter.limit,
            duration_ms = started.elapsed().as_millis(),
            "logEvents resolver completed"
        );

        Ok(rows.into_iter().map(LogEvent::from).collect())
    }

    /// Persisted interaction events, newest first. Counterpart of the
    /// `sendInteractionEvent` mutation. Requires the observer token.
    #[allow(clippy::too_many_arguments)] // flat filter args mirror accuracyByLine
    /// Retrieves persisted interaction events matching the supplied filters.
    ///
    /// # Examples
    ///
    /// ```
    /// let query = r#"
    ///     {
    ///         interactionEvents(limit: 10) {
    ///             id
    ///             eventName
    ///             timestamp
    ///         }
    ///     }
    /// "#;
    /// assert!(query.contains("interactionEvents"));
    /// ```
    ///
    /// `session_id`, `device`, and `event_name` restrict the results when provided.
    /// `from` and `to` define an optional time range, and `limit` controls the maximum
    /// number of events returned.
    ///
    /// # Returns
    ///
    /// The matching interaction events.
    async fn interaction_events(
        &self,
        ctx: &Context<'_>,
        session_id: Option<String>,
        device: Option<String>,
        from: Option<DateTime<Utc>>,
        to: Option<DateTime<Utc>>,
        event_name: Option<String>,
        #[graphql(default = 100)] limit: i32,
    ) -> Result<Vec<InteractionEvent>> {
        let (storage, filter) = history_query(ctx, session_id, device, from, to, limit)?;

        let started = Instant::now();
        let rows = storage
            .fetch_interaction_events(&filter, event_name.as_deref())
            .await
            .map_err(|e| format!("failed to fetch interaction events: {e}"))?;

        info!(
            count = rows.len(),
            limit = filter.limit,
            duration_ms = started.elapsed().as_millis(),
            "interactionEvents resolver completed"
        );

        Ok(rows.into_iter().map(InteractionEvent::from).collect())
    }

    /// Persisted location updates, newest first. Counterpart of the
    /// `sendLocation` mutation. Requires the observer token.
    #[allow(clippy::too_many_arguments)] // flat filter args mirror accuracyByLine
    /// Queries persisted location events using optional session, device, time-range,
    /// line, and movement-state filters.
    ///
    /// # Returns
    ///
    /// The matching location events, up to the requested limit.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// let request = async_graphql::Request::new(
    ///     "{ locations(sessionId: \"session-1\", limit: 100) { id } }",
    /// );
    /// let response = schema.execute(request).await;
    /// assert!(response.errors.is_empty());
    /// ```
    async fn locations(
        &self,
        ctx: &Context<'_>,
        session_id: Option<String>,
        device: Option<String>,
        from: Option<DateTime<Utc>>,
        to: Option<DateTime<Utc>>,
        line_id: Option<i32>,
        state: Option<MovementState>,
        #[graphql(default = 100)] limit: i32,
    ) -> Result<Vec<LocationEvent>> {
        let (storage, filter) = history_query(ctx, session_id, device, from, to, limit)?;

        let started = Instant::now();
        let rows = storage
            .fetch_locations(&filter, line_id, state.map(|s| s.as_str()))
            .await
            .map_err(|e| format!("failed to fetch location updates: {e}"))?;

        info!(
            count = rows.len(),
            limit = filter.limit,
            duration_ms = started.elapsed().as_millis(),
            "locations resolver completed"
        );

        Ok(rows.into_iter().map(LocationEvent::from).collect())
    }

    /// Location log gaps that match the frozen-position signature, newest gap
    /// first. Requires the observer token and a configured database.
    ///
    /// See `docs/location-freeze-regression.md` for the three conditions and
    /// for the gaps this deliberately cannot see (a gap whose closing row falls
    /// outside the window, and a session that never comes back).
    async fn location_freezes(
        &self,
        ctx: &Context<'_>,
        filter: LocationFreezeFilter,
        #[graphql(default = 100)] limit: i32,
    ) -> Result<Vec<LocationFreeze>> {
        let (storage, query) = freeze_query(ctx, filter, limit)?;

        let started = Instant::now();
        let rows = storage
            .fetch_location_freezes(&query)
            .await
            .map_err(|e| format!("failed to fetch location freezes: {e}"))?;

        info!(
            count = rows.len(),
            limit = query.limit,
            gap_threshold_ms = query.gap_threshold_ms,
            speed_threshold_kmh = query.speed_threshold_kmh,
            require_app_alive = query.require_app_alive,
            duration_ms = started.elapsed().as_millis(),
            "locationFreezes resolver completed"
        );

        Ok(rows.into_iter().map(LocationFreeze::from).collect())
    }

    /// Per-session freeze rollup, newest session first. Sessions without a
    /// freeze are included. Requires the observer token.
    async fn location_freeze_sessions(
        &self,
        ctx: &Context<'_>,
        filter: LocationFreezeFilter,
        #[graphql(default = 100)] limit: i32,
    ) -> Result<Vec<LocationFreezeSession>> {
        let (storage, query) = freeze_query(ctx, filter, limit)?;

        let started = Instant::now();
        let rows = storage
            .fetch_location_freeze_sessions(&query)
            .await
            .map_err(|e| format!("failed to fetch location freeze sessions: {e}"))?;

        info!(
            count = rows.len(),
            limit = query.limit,
            gap_threshold_ms = query.gap_threshold_ms,
            speed_threshold_kmh = query.speed_threshold_kmh,
            require_app_alive = query.require_app_alive,
            duration_ms = started.elapsed().as_millis(),
            "locationFreezeSessions resolver completed"
        );

        Ok(rows.into_iter().map(LocationFreezeSession::from).collect())
    }

    /// Freeze rollup by line, segment, device and build, worst group first.
    /// Groups without a freeze are included. Requires the observer token.
    async fn location_freeze_summary(
        &self,
        ctx: &Context<'_>,
        filter: LocationFreezeFilter,
        #[graphql(default = 100)] limit: i32,
    ) -> Result<Vec<LocationFreezeSummary>> {
        let (storage, query) = freeze_query(ctx, filter, limit)?;

        let started = Instant::now();
        let rows = storage
            .fetch_location_freeze_summary(&query)
            .await
            .map_err(|e| format!("failed to fetch location freeze summary: {e}"))?;

        info!(
            count = rows.len(),
            limit = query.limit,
            gap_threshold_ms = query.gap_threshold_ms,
            speed_threshold_kmh = query.speed_threshold_kmh,
            require_app_alive = query.require_app_alive,
            duration_ms = started.elapsed().as_millis(),
            "locationFreezeSummary resolver completed"
        );

        Ok(rows.into_iter().map(LocationFreezeSummary::from).collect())
    }
}

/// Prepares the validated filter shared by the three freeze queries.
///
/// Applies the same observer-token and storage checks as `history_query`, then
/// the freeze-specific range and threshold rules.
///
/// # Errors
///
/// Returns an error when authorization or storage configuration is missing,
/// history queries are disabled, the range is inverted or wider than 90 days,
/// or a threshold is out of range.
fn freeze_query<'a>(
    ctx: &'a Context<'_>,
    filter: LocationFreezeFilter,
    limit: i32,
) -> Result<(&'a Storage, LocationFreezeQuery)> {
    let auth = ctx
        .data::<RequestAuth>()
        .map_err(|_| "auth context is missing")?;
    if !auth.can_read_events {
        return Err("unauthorized: a valid observer bearer token is required".into());
    }

    if filter.from >= filter.to {
        return Err("from must be earlier than to".into());
    }

    let max_span = ChronoDuration::days(FREEZE_MAX_SPAN_DAYS);
    if filter.to - filter.from > max_span {
        return Err(format!(
            "requested span exceeds maximum for location freeze queries: max {} days",
            max_span.num_days()
        )
        .into());
    }

    if filter.gap_threshold_ms < FREEZE_MIN_GAP_THRESHOLD_MS {
        return Err(
            format!("gapThresholdMs must be at least {FREEZE_MIN_GAP_THRESHOLD_MS}").into(),
        );
    }

    if !filter.speed_threshold_kmh.is_finite() || filter.speed_threshold_kmh < 0.0 {
        return Err("speedThresholdKmh must be a finite value >= 0".into());
    }

    let storage = ctx
        .data::<Storage>()
        .map_err(|_| "storage is not configured; DATABASE_URL is required")?;
    if !storage.enabled() {
        return Err("database-backed storage is disabled; history queries are unavailable".into());
    }

    Ok((
        storage,
        LocationFreezeQuery {
            from_ts: filter.from.timestamp_millis(),
            to_ts: filter.to.timestamp_millis(),
            session_id: filter.session_id,
            line_id: filter.line_id,
            segment_id: filter.segment_id,
            device: filter.device,
            app_version: filter.app_version,
            platform: filter.platform.map(|p| p.as_str().to_string()),
            channel: filter.channel.map(|c| c.as_str().to_string()),
            gap_threshold_ms: i64::from(filter.gap_threshold_ms),
            speed_threshold_kmh: filter.speed_threshold_kmh,
            require_app_alive: filter.require_app_alive,
            limit: limit.clamp(1, HARD_LIMIT),
        },
    ))
}

/// Prepares the validated filter used by raw history queries.
///
/// Requires read authorization, an enabled storage backend, and an ascending
/// optional time range. The limit is clamped to the supported range.
///
/// # Errors
///
/// Returns an error when authorization or storage configuration is missing,
/// history queries are disabled, or the time range is invalid.
///
/// # Examples
///
/// ```ignore
/// let (storage, filter) = history_query(ctx, None, None, None, None, 100)?;
/// assert!(storage.enabled());
/// assert_eq!(filter.limit, 100);
/// ```
fn history_query<'a>(
    ctx: &'a Context<'_>,
    session_id: Option<String>,
    device: Option<String>,
    from: Option<DateTime<Utc>>,
    to: Option<DateTime<Utc>>,
    limit: i32,
) -> Result<(&'a Storage, EventFilter)> {
    let auth = ctx
        .data::<RequestAuth>()
        .map_err(|_| "auth context is missing")?;
    if !auth.can_read_events {
        return Err("unauthorized: a valid observer bearer token is required".into());
    }

    if let (Some(from), Some(to)) = (from, to) {
        if from >= to {
            return Err("from must be earlier than to".into());
        }
    }

    let storage = ctx
        .data::<Storage>()
        .map_err(|_| "storage is not configured; DATABASE_URL is required")?;
    if !storage.enabled() {
        return Err("database-backed storage is disabled; history queries are unavailable".into());
    }

    Ok((
        storage,
        EventFilter {
            session_id,
            device,
            from_ts: from.map(|t| t.timestamp_millis()),
            to_ts: to.map(|t| t.timestamp_millis()),
            limit: limit.clamp(1, HARD_LIMIT),
        },
    ))
}

#[derive(InputObject)]
pub struct LogEventInput {
    /// Client-generated unique session identifier (arbitrary string).
    pub session_id: String,
    /// Device identifier. Optional so events can be submitted anonymously.
    pub device: Option<String>,
    /// Application version string (e.g. "1.2.3").
    pub app_version: String,
    pub platform: Platform,
    pub channel: Channel,
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
pub struct InteractionEventInput {
    /// Client-generated unique session identifier (arbitrary string).
    pub session_id: String,
    /// Device identifier. Optional so events can be submitted anonymously.
    pub device: Option<String>,
    /// Application version string (e.g. "1.2.3").
    pub app_version: String,
    pub platform: Platform,
    pub channel: Channel,
    /// Unix timestamp in milliseconds.
    pub timestamp: u64,
    /// Arbitrary name of the user-driven interaction,
    /// e.g. "app_launch", "tab_change", "tts_request", "feedback_success".
    pub event_name: String,
    /// Optional flat map of extra attributes describing the interaction.
    /// Values must be string, number, boolean or null; nested objects and
    /// arrays are rejected.
    pub properties: Option<Properties>,
}

#[derive(SimpleObject)]
pub struct SendInteractionEventPayload {
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
    /// Only meaningful when state is arrived or passing; ignored otherwise.
    pub station_id: Option<i32>,
    pub line_id: i32,
    pub coords: CoordsInput,
    /// Unix timestamp in milliseconds.
    pub timestamp: u64,
    /// Battery level as a decimal (0.0 to 1.0).
    pub battery_level: Option<f64>,
    pub battery_state: Option<BatteryState>,
    /// Application version string (e.g. "1.2.3"), same value as the one sent
    /// with `sendLogEvent`. Optional for backwards compatibility with clients
    /// that predate it; a blank string is rejected.
    pub app_version: Option<String>,
    pub platform: Option<Platform>,
    pub channel: Option<Channel>,
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
    /// Submits a log event for authorized broadcast and persistence.
    ///
    /// The event is broadcast to telemetry subscribers and stored when persistence succeeds.
    /// Validation or missing dependencies are reported as errors; broadcast and persistence
    /// failures are logged while the submission response is still returned.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// let response = schema.execute(async_graphql::Request::new(
    ///     r#"mutation {
    ///         sendLogEvent(input: {
    ///             sessionId: "session-1"
    ///             appVersion: "1.0.0"
    ///             platform: WEB
    ///             channel: APP
    ///             timestamp: 0
    ///             type: INFO
    ///             level: INFO
    ///             message: "Started"
    ///         }) {
    ///             sessionId
    ///         }
    ///     }"#,
    /// )).await;
    /// assert!(response.errors.is_empty());
    /// ```
    ///
    /// # Parameters
    ///
    /// * `input` — The log event data, including its session, application, timestamp, and message.
    ///
    /// # Returns
    ///
    /// The submitted event's session identifier.
    async fn send_log_event(
        &self,
        ctx: &Context<'_>,
        input: LogEventInput,
    ) -> Result<SendLogEventPayload> {
        let auth = ctx
            .data::<RequestAuth>()
            .map_err(|_| "auth context is missing")?;
        if !auth.can_send_events {
            return Err(
                "unauthorized: a valid events or telemetry bearer token is required".into(),
            );
        }

        if input.session_id.trim().is_empty() {
            return Err("sessionId must not be empty".into());
        }

        if input.app_version.trim().is_empty() {
            return Err("appVersion must not be empty".into());
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
            app_version: input.app_version,
            platform: input.platform,
            channel: input.channel,
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

    /// Records a named user interaction for telemetry consumers and history queries.
    ///
    /// The interaction requires event-sending authorization. Empty session IDs, app
    /// versions, and event names are rejected. The event is broadcast to subscribers
    /// and stored when persistence succeeds.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let response = schema.execute(
    ///     async_graphql::Request::new(
    ///         r#"mutation {
    ///             sendInteractionEvent(input: {
    ///                 sessionId: "session-1",
    ///                 appVersion: "1.0.0",
    ///                 platform: IOS,
    ///                 channel: APP,
    ///                 timestamp: 1700000000000,
    ///                 eventName: "app_launch"
    ///             }) {
    ///                 sessionId
    ///             }
    ///         }"#,
    ///     ),
    /// ).await;
    /// assert!(response.errors.is_empty());
    /// ```
    ///
    /// # Returns
    ///
    /// The submitted session ID.
    async fn send_interaction_event(
        &self,
        ctx: &Context<'_>,
        input: InteractionEventInput,
    ) -> Result<SendInteractionEventPayload> {
        let auth = ctx
            .data::<RequestAuth>()
            .map_err(|_| "auth context is missing")?;
        if !auth.can_send_events {
            return Err(
                "unauthorized: a valid events or telemetry bearer token is required".into(),
            );
        }

        if input.session_id.trim().is_empty() {
            return Err("sessionId must not be empty".into());
        }

        if input.app_version.trim().is_empty() {
            return Err("appVersion must not be empty".into());
        }

        if input.event_name.trim().is_empty() {
            return Err("eventName must not be empty".into());
        }

        let hub = ctx
            .data::<Arc<TelemetryHub>>()
            .map_err(|_| "telemetry hub is not configured")?;
        let storage = ctx
            .data::<Storage>()
            .map_err(|_| "storage is not configured")?;

        let event = OutgoingInteraction {
            id: Uuid::new_v4().to_string(),
            session_id: input.session_id,
            device: input.device,
            app_version: input.app_version,
            platform: input.platform,
            channel: input.channel,
            timestamp: input.timestamp,
            event_name: input.event_name,
            properties: input.properties,
        };

        match serde_json::to_string(&OutgoingMessage::Interaction(event.clone())) {
            Ok(serialized) => hub.broadcast(serialized).await,
            Err(err) => {
                tracing::error!(?err, "failed to serialize interaction message");
            }
        }

        if let Err(err) = storage.store_interaction(&event).await {
            tracing::error!(?err, "failed to persist interaction event");
        }

        Ok(SendInteractionEventPayload {
            session_id: event.session_id,
        })
    }

    /// Publishes a validated location update with segment annotations.
    ///
    /// The update is broadcast to telemetry subscribers and persisted when storage is
    /// available. Reported accuracy above the configured threshold produces a warning.
    ///
    /// # Errors
    ///
    /// Returns an error when authorization, required dependencies, or location data
    /// are invalid.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let result = schema.execute(
    ///     async_graphql::Request::new(
    ///         r#"mutation {
    ///             sendLocation(input: {
    ///                 sessionId: "session-1",
    ///                 device: "device-1",
    ///                 state: stationary,
    ///                 lineId: "line-1",
    ///                 coords: { latitude: 48.8566, longitude: 2.3522 },
    ///                 timestamp: 1
    ///             }) { sessionId warning }
    ///         }"#,
    ///     ),
    /// ).await;
    /// assert!(result.errors.is_empty());
    /// ```
    async fn send_location(
        &self,
        ctx: &Context<'_>,
        input: LocationEventInput,
    ) -> Result<SendLocationPayload> {
        let auth = ctx
            .data::<RequestAuth>()
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

        if let Some(version) = &input.app_version {
            if version.trim().is_empty() {
                return Err("appVersion must not be blank".into());
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
            app_version: input.app_version,
            platform: input.platform,
            channel: input.channel,
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
    /// Converts a storage accuracy bucket into its GraphQL representation.
    ///
    /// # Examples
    ///
    /// ```
    /// # let row = LineAccuracyBucketRow {
    /// #     bucket_start: todo!(),
    /// #     bucket_end: todo!(),
    /// #     avg_accuracy: todo!(),
    /// #     p90_accuracy: todo!(),
    /// #     sample_count: todo!(),
    /// #     avg_speed: todo!(),
    /// #     max_speed: todo!(),
    /// # };
    /// let bucket: LineAccuracyBucket = row.into();
    /// ```
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

impl From<LogEventRow> for LogEvent {
    /// Converts a stored log event row into its GraphQL representation.
    ///
    /// Invalid platform and channel values are represented as `None`, and negative
    /// timestamps are converted to `0`.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let event: LogEvent = row.into();
    /// assert_eq!(event.message, "Application started");
    /// ```
    fn from(row: LogEventRow) -> Self {
        Self {
            id: row.id.into(),
            session_id: row.session_id,
            device: row.device,
            app_version: row.app_version,
            platform: row.platform.as_deref().and_then(Platform::parse),
            channel: row.channel.as_deref().and_then(Channel::parse),
            timestamp: u64::try_from(row.timestamp).unwrap_or(0),
            log_type: LogType::parse(&row.log_type),
            level: LogLevel::parse(&row.log_level),
            message: row.message,
            recorded_at: row.recorded_at,
        }
    }
}

impl From<InteractionEventRow> for InteractionEvent {
    /// Converts a stored interaction event row into its GraphQL representation.
    ///
    /// Invalid or negative timestamps become `0`, and properties that cannot be
    /// deserialized into the expected shape become `None`.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let event: InteractionEvent = row.into();
    /// assert_eq!(event.event_name, "screen_view");
    /// ```
    fn from(row: InteractionEventRow) -> Self {
        Self {
            id: row.id.into(),
            session_id: row.session_id,
            device: row.device,
            app_version: row.app_version,
            platform: row.platform.as_deref().and_then(Platform::parse),
            channel: row.channel.as_deref().and_then(Channel::parse),
            timestamp: u64::try_from(row.timestamp).unwrap_or(0),
            event_name: row.event_name,
            // stored properties are validated flat maps, so a mismatch only
            // occurs on hand-edited data; degrade to null rather than fail
            properties: row.properties.and_then(|v| serde_json::from_value(v).ok()),
            recorded_at: row.recorded_at,
        }
    }
}

impl From<LocationEventRow> for LocationEvent {
    /// Converts a stored location event row into its GraphQL representation.
    ///
    /// Invalid movement or battery-state values are omitted, and negative timestamps become zero.
    ///
    /// # Examples
    ///
    /// ```
    /// let event: LocationEvent = row.into();
    /// assert_eq!(event.id, row.id.into());
    /// ```
    fn from(row: LocationEventRow) -> Self {
        Self {
            id: row.id.into(),
            session_id: row.session_id,
            device: row.device,
            state: MovementState::parse(&row.state),
            station_id: row.station_id,
            line_id: row.line_id,
            coords: Coords {
                latitude: row.latitude,
                longitude: row.longitude,
                accuracy: row.accuracy,
                speed: row.speed,
            },
            timestamp: u64::try_from(row.timestamp).unwrap_or(0),
            segment_id: row.segment_id,
            from_station_id: row.from_station_id,
            to_station_id: row.to_station_id,
            battery_level: row.battery_level,
            battery_state: row.battery_state.and_then(BatteryState::from_i16),
            app_version: row.app_version,
            platform: row.platform.as_deref().and_then(Platform::parse),
            channel: row.channel.as_deref().and_then(Channel::parse),
            recorded_at: row.recorded_at,
        }
    }
}

/// Converts a client-reported unix-millisecond timestamp into a `DateTime`,
/// falling back to the epoch for values outside the representable range.
fn millis_to_datetime(millis: i64) -> DateTime<Utc> {
    DateTime::from_timestamp_millis(millis).unwrap_or_else(|| DateTime::from_timestamp_nanos(0))
}

/// Converts a non-negative millisecond count into the `u64` the schema exposes.
/// The values come from `MAX`/`SUM` over timestamp differences, so a negative
/// result would mean corrupt data; degrade to zero rather than fail the query.
fn millis_to_u64(millis: i64) -> u64 {
    u64::try_from(millis).unwrap_or(0)
}

impl From<LocationFreezeRow> for LocationFreeze {
    fn from(row: LocationFreezeRow) -> Self {
        let jump_distance_meters = haversine_meters(
            row.lat_before_gap,
            row.lon_before_gap,
            row.lat_after_gap,
            row.lon_after_gap,
        );

        Self {
            session_id: row.session_id,
            device: row.device,
            line_id: row.line_id,
            segment_id: row.segment_id,
            from_station_id: row.from_station_id,
            to_station_id: row.to_station_id,
            app_version: row.app_version,
            platform: row.platform.as_deref().and_then(Platform::parse),
            channel: row.channel.as_deref().and_then(Channel::parse),
            gap_start: millis_to_datetime(row.gap_start),
            gap_end: millis_to_datetime(row.gap_end),
            gap_ms: millis_to_u64(row.gap_ms),
            speed_before_gap: row.speed_before_gap,
            coords_before_gap: Coords {
                latitude: row.lat_before_gap,
                longitude: row.lon_before_gap,
                accuracy: row.accuracy_before_gap,
                speed: Some(row.speed_before_gap),
            },
            coords_after_gap: Coords {
                latitude: row.lat_after_gap,
                longitude: row.lon_after_gap,
                accuracy: row.accuracy_after_gap,
                speed: row.speed_after_gap,
            },
            jump_distance_meters,
            alive_event_count: row.alive_event_count,
        }
    }
}

impl From<LocationFreezeSessionRow> for LocationFreezeSession {
    fn from(row: LocationFreezeSessionRow) -> Self {
        Self {
            session_id: row.session_id,
            device: row.device,
            line_ids: row.line_ids,
            app_version: row.app_version,
            platform: row.platform.as_deref().and_then(Platform::parse),
            channel: row.channel.as_deref().and_then(Channel::parse),
            started_at: millis_to_datetime(row.started_at),
            ended_at: millis_to_datetime(row.ended_at),
            location_count: row.location_count,
            max_speed: row.max_speed,
            freeze_count: row.freeze_count,
            max_gap_ms: row.max_gap_ms.map(millis_to_u64),
            total_gap_ms: millis_to_u64(row.total_gap_ms),
        }
    }
}

impl From<LocationFreezeSummaryRow> for LocationFreezeSummary {
    fn from(row: LocationFreezeSummaryRow) -> Self {
        Self {
            line_id: row.line_id,
            segment_id: row.segment_id,
            from_station_id: row.from_station_id,
            to_station_id: row.to_station_id,
            device: row.device,
            app_version: row.app_version,
            platform: row.platform.as_deref().and_then(Platform::parse),
            channel: row.channel.as_deref().and_then(Channel::parse),
            session_count: row.session_count,
            location_count: row.location_count,
            freeze_session_count: row.freeze_session_count,
            freeze_count: row.freeze_count,
            max_gap_ms: row.max_gap_ms.map(millis_to_u64),
            total_gap_ms: millis_to_u64(row.total_gap_ms),
        }
    }
}

/// Calculates the number of time buckets needed to cover a time range.
///
/// # Examples
///
/// ```
/// use chrono::{Duration, Utc};
///
/// let from = Utc::now();
/// let to = from + Duration::seconds(61);
///
/// assert_eq!(estimate_bucket_count(from, to, 60), 2);
/// ```
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

    const EVENTS_ONLY: RequestAuth = RequestAuth {
        can_send_events: true,
        can_send_location: false,
        can_read_events: false,
    };
    const TELEMETRY: RequestAuth = RequestAuth {
        can_send_events: true,
        can_send_location: true,
        can_read_events: false,
    };
    const OBSERVER: RequestAuth = RequestAuth {
        can_send_events: false,
        can_send_location: false,
        can_read_events: true,
    };

    fn test_schema(hub: Arc<TelemetryHub>) -> AppSchema {
        build_schema(
            Storage::default(),
            hub,
            SegmentEstimator::new(LineTopology::empty()),
        )
    }

    fn request(query: &str, auth: RequestAuth) -> async_graphql::Request {
        async_graphql::Request::new(query.to_string()).data(auth)
    }

    /// A syntactically valid freeze query with the supplied extra filter
    /// fields, so the tests exercise the resolver rather than schema parsing.
    fn freeze_query_str(field: &str, selection: &str, extra: &str) -> String {
        format!(
            r#"query {{
                {field}(filter: {{
                    from: "2026-07-01T00:00:00Z",
                    to: "2026-07-02T00:00:00Z"{extra}
                }}) {{ {selection} }}
            }}"#
        )
    }

    /// The three freeze queries with a valid filter, for the shared
    /// authorization and storage checks.
    fn freeze_queries() -> [String; 3] {
        [
            freeze_query_str("locationFreezes", "sessionId", ""),
            freeze_query_str("locationFreezeSessions", "sessionId", ""),
            freeze_query_str("locationFreezeSummary", "device", ""),
        ]
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
                        appVersion: "1.2.3",
                        platform: ios,
                        channel: production,
                        device: "dev",
                        timestamp: 1706000000000,
                        type: app,
                        level: info,
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
        assert_eq!(v["app_version"], "1.2.3");
        assert_eq!(v["platform"], "ios");
        assert_eq!(v["channel"], "production");
        assert_eq!(v["log"]["message"], "hello");
        assert_eq!(v["timestamp"], 1706000000000u64);
    }

    #[tokio::test]
    async fn send_log_event_rejects_empty_app_version() {
        let hub = Arc::new(TelemetryHub::new(10));
        let schema = test_schema(hub.clone());

        let resp = schema
            .execute(request(
                r#"mutation {
                    sendLogEvent(input: {
                        sessionId: "sess-1",
                        appVersion: "  ",
                        platform: android,
                        channel: canary,
                        timestamp: 1,
                        type: app,
                        level: info,
                        message: "hi"
                    }) { sessionId }
                }"#,
                EVENTS_ONLY,
            ))
            .await;

        assert!(!resp.errors.is_empty());
        assert!(resp.errors[0].message.contains("appVersion"));
        assert!(hub.snapshot().await.is_empty());
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
                        appVersion: "1.2.3",
                        platform: ios,
                        channel: production,
                        timestamp: 1,
                        type: app,
                        level: info,
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
                        state: moving,
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
                        appVersion: "1.2.3",
                        platform: ios,
                        channel: production,
                        device: "dev",
                        timestamp: 1,
                        type: system,
                        level: warn,
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
                        appVersion: "1.2.3",
                        platform: ios,
                        channel: production,
                        device: "dev",
                        timestamp: 1,
                        type: app,
                        level: info,
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
                        appVersion: "1.2.3",
                        platform: ios,
                        channel: production,
                        device: "dev",
                        timestamp: 1,
                        type: app,
                        level: info,
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
    async fn send_interaction_event_broadcasts_event_name() {
        let hub = Arc::new(TelemetryHub::new(10));
        let schema = test_schema(hub.clone());

        let resp = schema
            .execute(request(
                r#"mutation {
                    sendInteractionEvent(input: {
                        sessionId: "sess-1",
                        appVersion: "1.2.3",
                        platform: ios,
                        channel: production,
                        device: "dev",
                        timestamp: 1706000000000,
                        eventName: "tts_request"
                    }) { sessionId }
                }"#,
                EVENTS_ONLY,
            ))
            .await;

        assert!(resp.errors.is_empty(), "errors: {:?}", resp.errors);
        let data = resp.data.into_json().unwrap();
        assert_eq!(data["sendInteractionEvent"]["sessionId"], "sess-1");

        let snapshot = hub.snapshot().await;
        assert_eq!(snapshot.len(), 1);
        let v: serde_json::Value = serde_json::from_str(&snapshot[0]).unwrap();
        assert_eq!(v["type"], "interaction");
        assert_eq!(v["event_name"], "tts_request");
        assert_eq!(v["session_id"], "sess-1");
        assert!(!v["id"].as_str().unwrap().is_empty());
        assert_eq!(v["app_version"], "1.2.3");
        assert_eq!(v["platform"], "ios");
        assert_eq!(v["channel"], "production");
    }

    #[tokio::test]
    async fn send_interaction_event_records_flat_properties() {
        let hub = Arc::new(TelemetryHub::new(10));
        let schema = test_schema(hub.clone());

        let resp = schema
            .execute(request(
                r#"mutation {
                    sendInteractionEvent(input: {
                        sessionId: "sess-1",
                        appVersion: "1.2.3",
                        platform: ios,
                        channel: production,
                        timestamp: 1,
                        eventName: "tab_change",
                        properties: { tab: "map", index: 2, pinned: true, note: null }
                    }) { sessionId }
                }"#,
                EVENTS_ONLY,
            ))
            .await;

        assert!(resp.errors.is_empty(), "errors: {:?}", resp.errors);
        let snapshot = hub.snapshot().await;
        assert_eq!(snapshot.len(), 1);
        let v: serde_json::Value = serde_json::from_str(&snapshot[0]).unwrap();
        assert_eq!(v["properties"]["tab"], "map");
        assert_eq!(v["properties"]["index"], 2);
        assert_eq!(v["properties"]["pinned"], true);
        assert!(v["properties"]["note"].is_null());
    }

    #[tokio::test]
    async fn send_interaction_event_rejects_nested_properties() {
        let hub = Arc::new(TelemetryHub::new(10));
        let schema = test_schema(hub.clone());

        for bad in [r#"{ nested: { a: 1 } }"#, r#"{ list: [1, 2] }"#] {
            let resp = schema
                .execute(request(
                    &format!(
                        r#"mutation {{
                            sendInteractionEvent(input: {{
                                sessionId: "sess-1",
                                appVersion: "1.2.3",
                                platform: ios,
                                channel: production,
                                timestamp: 1,
                                eventName: "tab_change",
                                properties: {bad}
                            }}) {{ sessionId }}
                        }}"#
                    ),
                    EVENTS_ONLY,
                ))
                .await;

            assert!(!resp.errors.is_empty(), "expected rejection for {bad}");
        }

        assert!(hub.snapshot().await.is_empty());
    }

    #[tokio::test]
    async fn send_interaction_event_accepts_anonymous_device() {
        let hub = Arc::new(TelemetryHub::new(10));
        let schema = test_schema(hub.clone());

        let resp = schema
            .execute(request(
                r#"mutation {
                    sendInteractionEvent(input: {
                        sessionId: "sess-1",
                        appVersion: "1.2.3",
                        platform: ios,
                        channel: production,
                        timestamp: 1,
                        eventName: "app_launch"
                    }) { sessionId }
                }"#,
                TELEMETRY,
            ))
            .await;

        assert!(resp.errors.is_empty(), "errors: {:?}", resp.errors);
        let snapshot = hub.snapshot().await;
        assert_eq!(snapshot.len(), 1);
        let v: serde_json::Value = serde_json::from_str(&snapshot[0]).unwrap();
        assert!(v["device"].is_null());
        assert_eq!(v["event_name"], "app_launch");
    }

    #[tokio::test]
    async fn send_interaction_event_rejects_empty_event_name() {
        let hub = Arc::new(TelemetryHub::new(10));
        let schema = test_schema(hub.clone());

        let resp = schema
            .execute(request(
                r#"mutation {
                    sendInteractionEvent(input: {
                        sessionId: "sess-1",
                        appVersion: "1.2.3",
                        platform: ios,
                        channel: production,
                        timestamp: 1,
                        eventName: "  "
                    }) { sessionId }
                }"#,
                EVENTS_ONLY,
            ))
            .await;

        assert!(!resp.errors.is_empty());
        assert!(resp.errors[0].message.contains("eventName"));
        assert!(hub.snapshot().await.is_empty());
    }

    #[tokio::test]
    async fn send_interaction_event_rejects_observer_scope() {
        let hub = Arc::new(TelemetryHub::new(10));
        let schema = test_schema(hub.clone());

        let resp = schema
            .execute(request(
                r#"mutation {
                    sendInteractionEvent(input: {
                        sessionId: "sess-1",
                        appVersion: "1.2.3",
                        platform: ios,
                        channel: production,
                        timestamp: 1,
                        eventName: "app_launch"
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
                &location_mutation("moving", ", speed: 50.0"),
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
            .execute(request(&location_mutation("moving", ""), EVENTS_ONLY))
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
                        state: moving,
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
                &location_mutation("moving", ", accuracy: -1.0"),
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
                &location_mutation("moving", ", accuracy: 150.0"),
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
                        state: moving,
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

    #[tokio::test]
    async fn history_queries_reject_missing_read_scope() {
        let hub = Arc::new(TelemetryHub::new(10));
        let schema = test_schema(hub);

        let queries: Vec<String> = [
            r#"query { logEvents { id } }"#.to_string(),
            r#"query { interactionEvents { id } }"#.to_string(),
            r#"query { locations { id } }"#.to_string(),
        ]
        .into_iter()
        .chain(freeze_queries())
        .collect();

        for query in &queries {
            for auth in [EVENTS_ONLY, TELEMETRY] {
                let resp = schema.execute(request(query, auth)).await;
                assert!(!resp.errors.is_empty(), "expected rejection for {query}");
                assert!(
                    resp.errors[0].message.contains("observer"),
                    "unexpected message for {query}: {}",
                    resp.errors[0].message
                );
            }
        }
    }

    #[tokio::test]
    async fn history_queries_reject_inverted_time_range() {
        let hub = Arc::new(TelemetryHub::new(10));
        let schema = test_schema(hub);

        let resp = schema
            .execute(request(
                r#"query {
                    logEvents(from: "2026-07-02T00:00:00Z", to: "2026-07-01T00:00:00Z") { id }
                }"#,
                OBSERVER,
            ))
            .await;

        assert!(!resp.errors.is_empty());
        assert!(resp.errors[0]
            .message
            .contains("from must be earlier than to"));
    }

    #[tokio::test]
    async fn history_queries_report_disabled_storage_with_read_scope() {
        let hub = Arc::new(TelemetryHub::new(10));
        let schema = test_schema(hub);

        let queries: Vec<String> = [
            r#"query { logEvents { id } }"#.to_string(),
            r#"query { interactionEvents { id } }"#.to_string(),
            r#"query { locations { id } }"#.to_string(),
        ]
        .into_iter()
        .chain(freeze_queries())
        .collect();

        for query in &queries {
            let resp = schema.execute(request(query, OBSERVER)).await;
            assert!(!resp.errors.is_empty(), "expected error for {query}");
            assert!(
                resp.errors[0].message.contains("storage is disabled"),
                "unexpected message for {query}: {}",
                resp.errors[0].message
            );
        }
    }

    #[test]
    fn bucket_limits_match_spec() {
        assert_eq!(TimeBucketSize::Minute.max_duration().num_days(), 7);
        assert_eq!(TimeBucketSize::Hour.max_duration().num_days(), 90);
        assert_eq!(TimeBucketSize::Day.max_duration().num_days(), 365);
    }

    #[tokio::test]
    async fn freeze_queries_reject_inverted_time_range() {
        let hub = Arc::new(TelemetryHub::new(10));
        let schema = test_schema(hub);

        let resp = schema
            .execute(request(
                r#"query {
                    locationFreezes(filter: {
                        from: "2026-07-02T00:00:00Z",
                        to: "2026-07-01T00:00:00Z"
                    }) { sessionId }
                }"#,
                OBSERVER,
            ))
            .await;

        assert!(!resp.errors.is_empty());
        assert!(resp.errors[0]
            .message
            .contains("from must be earlier than to"));
    }

    #[tokio::test]
    async fn freeze_queries_reject_span_beyond_ninety_days() {
        let hub = Arc::new(TelemetryHub::new(10));
        let schema = test_schema(hub);

        let resp = schema
            .execute(request(
                r#"query {
                    locationFreezeSessions(filter: {
                        from: "2026-01-01T00:00:00Z",
                        to: "2026-05-01T00:00:00Z"
                    }) { sessionId }
                }"#,
                OBSERVER,
            ))
            .await;

        assert!(!resp.errors.is_empty());
        assert!(
            resp.errors[0].message.contains("90 days"),
            "unexpected message: {}",
            resp.errors[0].message
        );
    }

    #[tokio::test]
    async fn freeze_queries_reject_sub_second_gap_threshold() {
        let hub = Arc::new(TelemetryHub::new(10));
        let schema = test_schema(hub);

        let resp = schema
            .execute(request(
                &freeze_query_str("locationFreezes", "sessionId", ", gapThresholdMs: 999"),
                OBSERVER,
            ))
            .await;

        assert!(!resp.errors.is_empty());
        assert!(
            resp.errors[0].message.contains("gapThresholdMs"),
            "unexpected message: {}",
            resp.errors[0].message
        );
    }

    #[tokio::test]
    async fn freeze_queries_reject_negative_speed_threshold() {
        let hub = Arc::new(TelemetryHub::new(10));
        let schema = test_schema(hub);

        let resp = schema
            .execute(request(
                &freeze_query_str(
                    "locationFreezeSummary",
                    "device",
                    ", speedThresholdKmh: -1.0",
                ),
                OBSERVER,
            ))
            .await;

        assert!(!resp.errors.is_empty());
        assert!(
            resp.errors[0].message.contains("speedThresholdKmh"),
            "unexpected message: {}",
            resp.errors[0].message
        );
    }

    #[tokio::test]
    async fn freeze_queries_accept_defaulted_thresholds() {
        let hub = Arc::new(TelemetryHub::new(10));
        let schema = test_schema(hub);

        // storage is disabled in tests, so passing validation surfaces as the
        // storage error rather than a threshold complaint
        let resp = schema
            .execute(request(
                &freeze_query_str("locationFreezes", "sessionId gapMs", ""),
                OBSERVER,
            ))
            .await;

        assert!(!resp.errors.is_empty());
        assert!(
            resp.errors[0].message.contains("storage is disabled"),
            "unexpected message: {}",
            resp.errors[0].message
        );
    }

    #[tokio::test]
    async fn send_location_broadcasts_build_metadata() {
        let hub = Arc::new(TelemetryHub::new(10));
        let schema = test_schema(hub.clone());

        let resp = schema
            .execute(request(
                r#"mutation {
                    sendLocation(input: {
                        sessionId: "sess-1",
                        device: "dev",
                        state: moving,
                        lineId: 1,
                        coords: { latitude: 35.6812, longitude: 139.7671, speed: 320.0 },
                        timestamp: 1706000000000,
                        appVersion: "10.4.2(101)",
                        platform: ios,
                        channel: canary
                    }) { sessionId }
                }"#,
                TELEMETRY,
            ))
            .await;

        assert!(resp.errors.is_empty(), "errors: {:?}", resp.errors);
        let snapshot = hub.snapshot().await;
        assert_eq!(snapshot.len(), 1);
        let v: serde_json::Value = serde_json::from_str(&snapshot[0]).unwrap();
        assert_eq!(v["app_version"], "10.4.2(101)");
        assert_eq!(v["platform"], "ios");
        assert_eq!(v["channel"], "canary");
    }

    #[tokio::test]
    async fn send_location_omits_build_metadata_when_not_sent() {
        let hub = Arc::new(TelemetryHub::new(10));
        let schema = test_schema(hub.clone());

        let resp = schema
            .execute(request(&location_mutation("moving", ""), TELEMETRY))
            .await;

        assert!(resp.errors.is_empty(), "errors: {:?}", resp.errors);
        let snapshot = hub.snapshot().await;
        assert_eq!(snapshot.len(), 1);
        let v: serde_json::Value = serde_json::from_str(&snapshot[0]).unwrap();
        assert!(v["app_version"].is_null());
        assert!(v["platform"].is_null());
        assert!(v["channel"].is_null());
    }

    #[tokio::test]
    async fn send_location_rejects_blank_app_version() {
        let hub = Arc::new(TelemetryHub::new(10));
        let schema = test_schema(hub.clone());

        let resp = schema
            .execute(request(
                r#"mutation {
                    sendLocation(input: {
                        sessionId: "sess-1",
                        device: "dev",
                        state: moving,
                        lineId: 1,
                        coords: { latitude: 35.6812, longitude: 139.7671 },
                        timestamp: 1,
                        appVersion: "   "
                    }) { sessionId }
                }"#,
                TELEMETRY,
            ))
            .await;

        assert!(!resp.errors.is_empty());
        assert!(resp.errors[0].message.contains("appVersion"));
        assert!(hub.snapshot().await.is_empty());
    }

    #[test]
    fn millis_round_trip_through_the_freeze_conversions() {
        assert_eq!(millis_to_datetime(0).timestamp_millis(), 0);
        assert_eq!(
            millis_to_datetime(1_706_000_000_000).timestamp_millis(),
            1_706_000_000_000
        );
        assert_eq!(millis_to_u64(300_000), 300_000);
        // corrupt data degrades to zero instead of failing the query
        assert_eq!(millis_to_u64(-1), 0);
    }

    #[test]
    fn estimate_bucket_count_rounds_up() {
        let from = Utc::now();
        let to = from + ChronoDuration::seconds(61);
        assert_eq!(estimate_bucket_count(from, to, 60), 2);
    }
}
