//! Detection of "frozen position" regressions in the location telemetry.
//!
//! The signature this module looks for (see `docs/location-freeze-regression.md`)
//! is a location log gap that satisfies all three of:
//!
//! 1. no location row for longer than `gap_threshold_ms`,
//! 2. the OS-reported speed on the row right before the gap was high, and
//! 3. the app itself kept running during the gap (log or interaction events
//!    were still being submitted for the same session).
//!
//! Taken together those rule out a normal stop at a station (2) and an app or
//! device that simply went away (3), leaving the case where the app is alive
//! but its position stopped advancing.

use sqlx::{postgres::PgArguments, query::QueryAs, Postgres};

use crate::storage::Storage;

/// Mean Earth radius used by [`haversine_meters`].
const EARTH_RADIUS_METERS: f64 = 6_371_000.0;

/// Filters shared by the three freeze queries. The time bounds are
/// client-reported unix milliseconds, matching the `timestamp` column.
pub struct LocationFreezeQuery {
    /// Inclusive lower bound on the client-reported timestamp, unix millis.
    pub from_ts: i64,
    /// Exclusive upper bound on the client-reported timestamp, unix millis.
    pub to_ts: i64,
    pub session_id: Option<String>,
    pub line_id: Option<i32>,
    pub segment_id: Option<String>,
    pub device: Option<String>,
    pub app_version: Option<String>,
    pub platform: Option<String>,
    pub channel: Option<String>,
    /// A location gap longer than this many milliseconds is a freeze candidate.
    pub gap_threshold_ms: i64,
    /// Only gaps whose preceding row reported a speed above this (km/h) count.
    pub speed_threshold_kmh: f64,
    /// When true, a gap only counts if the app kept emitting events during it.
    pub require_app_alive: bool,
    pub limit: i32,
}

/// One detected gap, with the rows on either side of it.
#[derive(Clone, sqlx::FromRow)]
pub struct LocationFreezeRow {
    pub session_id: String,
    pub device: String,
    pub line_id: Option<i32>,
    pub segment_id: Option<String>,
    pub from_station_id: Option<i32>,
    pub to_station_id: Option<i32>,
    pub app_version: Option<String>,
    pub platform: Option<String>,
    pub channel: Option<String>,
    /// Client timestamp of the last row before the gap, unix millis.
    pub gap_start: i64,
    /// Client timestamp of the first row after the gap, unix millis.
    pub gap_end: i64,
    pub gap_ms: i64,
    pub speed_before_gap: f64,
    pub lat_before_gap: f64,
    pub lon_before_gap: f64,
    pub accuracy_before_gap: Option<f64>,
    pub lat_after_gap: f64,
    pub lon_after_gap: f64,
    pub accuracy_after_gap: Option<f64>,
    pub speed_after_gap: Option<f64>,
    pub alive_event_count: i32,
}

/// Per-session rollup. Sessions without any freeze are included so two builds
/// that ran the same segment can be compared side by side.
#[derive(Clone, sqlx::FromRow)]
pub struct LocationFreezeSessionRow {
    pub session_id: String,
    pub device: String,
    pub line_id: Option<i32>,
    pub app_version: Option<String>,
    pub platform: Option<String>,
    pub channel: Option<String>,
    /// Client timestamp of the session's first in-window row, unix millis.
    pub started_at: i64,
    /// Client timestamp of the session's last in-window row, unix millis.
    pub ended_at: i64,
    pub location_count: i32,
    pub max_speed: Option<f64>,
    pub freeze_count: i32,
    pub max_gap_ms: Option<i64>,
    pub total_gap_ms: i64,
}

/// Rollup by line / segment / device / build. Groups without any freeze are
/// included for the same reason as [`LocationFreezeSessionRow`].
#[derive(Clone, sqlx::FromRow)]
pub struct LocationFreezeSummaryRow {
    pub line_id: Option<i32>,
    pub segment_id: Option<String>,
    pub from_station_id: Option<i32>,
    pub to_station_id: Option<i32>,
    pub device: String,
    pub app_version: Option<String>,
    pub platform: Option<String>,
    pub channel: Option<String>,
    pub session_count: i32,
    pub location_count: i32,
    pub freeze_session_count: i32,
    pub freeze_count: i32,
    pub max_gap_ms: Option<i64>,
    pub total_gap_ms: i64,
}

/// Common CTE prefix shared by the three queries.
///
/// `LEAD` is deliberately computed over every row of the session, *before* the
/// line / segment / device / build filters are applied: filtering first would
/// drop the neighbouring rows at a segment boundary or a line change and
/// manufacture gaps that never happened. The filters are applied afterwards, in
/// `scoped`, against the row that precedes the gap.
///
/// Bind order, fixed for all three queries:
/// `$1` from_ts, `$2` to_ts, `$3` session_id, `$4` line_id, `$5` segment_id,
/// `$6` device, `$7` app_version, `$8` platform, `$9` channel,
/// `$10` gap_threshold_ms, `$11` speed_threshold_kmh, `$12` require_app_alive,
/// `$13` limit.
const COMMON_CTE: &str = r#"
WITH ordered AS (
  SELECT l.session_id, l.device, l.line_id, l.segment_id, l.from_station_id, l.to_station_id,
         l.latitude, l.longitude, l.accuracy, l.speed, l.timestamp,
         l.app_version, l.platform, l.channel,
         LEAD(l.timestamp) OVER w AS next_timestamp,
         LEAD(l.latitude)  OVER w AS next_latitude,
         LEAD(l.longitude) OVER w AS next_longitude,
         LEAD(l.accuracy)  OVER w AS next_accuracy,
         LEAD(l.speed)     OVER w AS next_speed
  FROM location_logs l
  WHERE l.session_id IS NOT NULL
    AND l.timestamp >= $1::bigint AND l.timestamp < $2::bigint
    AND ($3::text IS NULL OR l.session_id = $3)
  WINDOW w AS (PARTITION BY l.session_id ORDER BY l.timestamp)
),
session_meta AS (
  SELECT session_id, MIN(app_version) AS app_version, MIN(platform) AS platform, MIN(channel) AS channel
  FROM (
    SELECT session_id, app_version, platform, channel FROM log_events
     WHERE session_id IN (SELECT DISTINCT session_id FROM ordered)
    UNION ALL
    SELECT session_id, app_version, platform, channel FROM interaction_events
     WHERE session_id IN (SELECT DISTINCT session_id FROM ordered)
  ) e
  GROUP BY session_id
),
scoped AS (
  SELECT o.*,
         COALESCE(o.app_version, m.app_version) AS eff_app_version,
         COALESCE(o.platform,    m.platform)    AS eff_platform,
         COALESCE(o.channel,     m.channel)     AS eff_channel
  FROM ordered o LEFT JOIN session_meta m ON m.session_id = o.session_id
  WHERE ($4::int  IS NULL OR o.line_id = $4)
    AND ($5::text IS NULL OR o.segment_id = $5)
    AND ($6::text IS NULL OR o.device = $6)
    AND ($7::text IS NULL OR COALESCE(o.app_version, m.app_version) = $7)
    AND ($8::text IS NULL OR COALESCE(o.platform, m.platform) = $8)
    AND ($9::text IS NULL OR COALESCE(o.channel, m.channel) = $9)
),
freezes AS (
  SELECT s.*, (s.next_timestamp - s.timestamp) AS gap_ms,
         (
           (SELECT COUNT(*) FROM log_events e
             WHERE e.session_id = s.session_id AND e.timestamp > s.timestamp AND e.timestamp < s.next_timestamp)
           +
           (SELECT COUNT(*) FROM interaction_events i
             WHERE i.session_id = s.session_id AND i.timestamp > s.timestamp AND i.timestamp < s.next_timestamp)
         )::int AS alive_event_count
  FROM scoped s
  WHERE s.next_timestamp IS NOT NULL
    AND s.next_timestamp - s.timestamp > $10::bigint
    AND s.speed IS NOT NULL AND s.speed > $11::double precision
),
freezes_alive AS (
  SELECT * FROM freezes WHERE (NOT $12::bool) OR alive_event_count > 0
)
"#;

/// Tail of the detail query: one row per detected gap, newest first.
const FREEZES_TAIL: &str = r#"
SELECT f.session_id,
       f.device,
       f.line_id,
       f.segment_id,
       f.from_station_id,
       f.to_station_id,
       f.eff_app_version AS app_version,
       f.eff_platform    AS platform,
       f.eff_channel     AS channel,
       f.timestamp       AS gap_start,
       f.next_timestamp  AS gap_end,
       f.gap_ms,
       f.speed           AS speed_before_gap,
       f.latitude        AS lat_before_gap,
       f.longitude       AS lon_before_gap,
       f.accuracy        AS accuracy_before_gap,
       f.next_latitude   AS lat_after_gap,
       f.next_longitude  AS lon_after_gap,
       f.next_accuracy   AS accuracy_after_gap,
       f.next_speed      AS speed_after_gap,
       f.alive_event_count
FROM freezes_alive f
ORDER BY f.timestamp DESC
LIMIT $13
"#;

/// Tail of the per-session query. `session_stats` covers every session in the
/// window, so sessions with zero freezes still show up.
const SESSIONS_TAIL: &str = r#"
, session_stats AS (
  SELECT s.session_id, s.device, s.line_id,
         s.eff_app_version, s.eff_platform, s.eff_channel,
         MIN(s.timestamp)::bigint AS started_at,
         MAX(s.timestamp)::bigint AS ended_at,
         COUNT(*)::int AS location_count,
         MAX(s.speed) AS max_speed
  FROM scoped s
  GROUP BY s.session_id, s.device, s.line_id, s.eff_app_version, s.eff_platform, s.eff_channel
),
session_freezes AS (
  SELECT f.session_id, f.line_id,
         COUNT(*)::int AS freeze_count,
         MAX(f.gap_ms)::bigint AS max_gap_ms,
         COALESCE(SUM(f.gap_ms), 0)::bigint AS total_gap_ms
  FROM freezes_alive f
  GROUP BY f.session_id, f.line_id
)
SELECT s.session_id,
       s.device,
       s.line_id,
       s.eff_app_version AS app_version,
       s.eff_platform    AS platform,
       s.eff_channel     AS channel,
       s.started_at,
       s.ended_at,
       s.location_count,
       s.max_speed,
       COALESCE(f.freeze_count, 0) AS freeze_count,
       f.max_gap_ms,
       COALESCE(f.total_gap_ms, 0)::bigint AS total_gap_ms
FROM session_stats s
LEFT JOIN session_freezes f
       ON f.session_id = s.session_id
      AND f.line_id IS NOT DISTINCT FROM s.line_id
ORDER BY s.started_at DESC
LIMIT $13
"#;

/// Tail of the aggregated query, grouped by line / segment / device / build.
const SUMMARY_TAIL: &str = r#"
, group_stats AS (
  SELECT s.line_id, s.segment_id, s.device,
         s.eff_app_version, s.eff_platform, s.eff_channel,
         MIN(s.from_station_id) AS from_station_id,
         MIN(s.to_station_id) AS to_station_id,
         COUNT(DISTINCT s.session_id)::int AS session_count,
         COUNT(*)::int AS location_count
  FROM scoped s
  GROUP BY s.line_id, s.segment_id, s.device, s.eff_app_version, s.eff_platform, s.eff_channel
),
group_freezes AS (
  SELECT f.line_id, f.segment_id, f.device,
         f.eff_app_version, f.eff_platform, f.eff_channel,
         COUNT(DISTINCT f.session_id)::int AS freeze_session_count,
         COUNT(*)::int AS freeze_count,
         MAX(f.gap_ms)::bigint AS max_gap_ms,
         COALESCE(SUM(f.gap_ms), 0)::bigint AS total_gap_ms
  FROM freezes_alive f
  GROUP BY f.line_id, f.segment_id, f.device, f.eff_app_version, f.eff_platform, f.eff_channel
)
SELECT g.line_id,
       g.segment_id,
       g.from_station_id,
       g.to_station_id,
       g.device,
       g.eff_app_version AS app_version,
       g.eff_platform    AS platform,
       g.eff_channel     AS channel,
       g.session_count,
       g.location_count,
       COALESCE(f.freeze_session_count, 0) AS freeze_session_count,
       COALESCE(f.freeze_count, 0) AS freeze_count,
       f.max_gap_ms,
       COALESCE(f.total_gap_ms, 0)::bigint AS total_gap_ms
FROM group_stats g
LEFT JOIN group_freezes f
       ON f.device = g.device
      AND f.line_id IS NOT DISTINCT FROM g.line_id
      AND f.segment_id IS NOT DISTINCT FROM g.segment_id
      AND f.eff_app_version IS NOT DISTINCT FROM g.eff_app_version
      AND f.eff_platform IS NOT DISTINCT FROM g.eff_platform
      AND f.eff_channel IS NOT DISTINCT FROM g.eff_channel
ORDER BY freeze_count DESC, session_count DESC
LIMIT $13
"#;

/// Concatenates the shared CTE with a query-specific tail.
fn freeze_sql(tail: &str) -> String {
    format!("{COMMON_CTE}{tail}")
}

/// Applies the 13 shared bind parameters in the order documented on [`COMMON_CTE`].
fn bind_filter<'q, T>(
    query: QueryAs<'q, Postgres, T, PgArguments>,
    filter: &'q LocationFreezeQuery,
) -> QueryAs<'q, Postgres, T, PgArguments> {
    query
        .bind(filter.from_ts)
        .bind(filter.to_ts)
        .bind(&filter.session_id)
        .bind(filter.line_id)
        .bind(&filter.segment_id)
        .bind(&filter.device)
        .bind(&filter.app_version)
        .bind(&filter.platform)
        .bind(&filter.channel)
        .bind(filter.gap_threshold_ms)
        .bind(filter.speed_threshold_kmh)
        .bind(filter.require_app_alive)
        .bind(filter.limit)
}

/// Great-circle distance between two WGS84 points, in meters.
///
/// Used for `jumpDistanceMeters`: how far the reported position jumped while
/// it was frozen, i.e. roughly how far the displayed position had drifted from
/// the real one by the time updates resumed.
pub(crate) fn haversine_meters(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let phi1 = lat1.to_radians();
    let phi2 = lat2.to_radians();
    let delta_phi = (lat2 - lat1).to_radians();
    let delta_lambda = (lon2 - lon1).to_radians();

    let a = (delta_phi / 2.0).sin().powi(2)
        + phi1.cos() * phi2.cos() * (delta_lambda / 2.0).sin().powi(2);
    // atan2 form stays accurate for antipodal points, where asin saturates.
    2.0 * EARTH_RADIUS_METERS * a.sqrt().atan2((1.0 - a).max(0.0).sqrt())
}

impl Storage {
    /// Detected location freezes, newest gap first.
    ///
    /// # Errors
    ///
    /// Returns an error when the database is not configured or the query fails.
    pub async fn fetch_location_freezes(
        &self,
        filter: &LocationFreezeQuery,
    ) -> anyhow::Result<Vec<LocationFreezeRow>> {
        let pool = self.pool()?;
        let sql = freeze_sql(FREEZES_TAIL);
        let rows = bind_filter(sqlx::query_as::<_, LocationFreezeRow>(&sql), filter)
            .fetch_all(pool)
            .await?;
        Ok(rows)
    }

    /// Per-session freeze rollup, newest session first. Sessions with no freeze
    /// are included.
    ///
    /// # Errors
    ///
    /// Returns an error when the database is not configured or the query fails.
    pub async fn fetch_location_freeze_sessions(
        &self,
        filter: &LocationFreezeQuery,
    ) -> anyhow::Result<Vec<LocationFreezeSessionRow>> {
        let pool = self.pool()?;
        let sql = freeze_sql(SESSIONS_TAIL);
        let rows = bind_filter(sqlx::query_as::<_, LocationFreezeSessionRow>(&sql), filter)
            .fetch_all(pool)
            .await?;
        Ok(rows)
    }

    /// Freeze rollup by line / segment / device / build, worst group first.
    /// Groups with no freeze are included.
    ///
    /// # Errors
    ///
    /// Returns an error when the database is not configured or the query fails.
    pub async fn fetch_location_freeze_summary(
        &self,
        filter: &LocationFreezeQuery,
    ) -> anyhow::Result<Vec<LocationFreezeSummaryRow>> {
        let pool = self.pool()?;
        let sql = freeze_sql(SUMMARY_TAIL);
        let rows = bind_filter(sqlx::query_as::<_, LocationFreezeSummaryRow>(&sql), filter)
            .fetch_all(pool)
            .await?;
        Ok(rows)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn haversine_is_zero_for_the_same_point() {
        assert_eq!(haversine_meters(35.6812, 139.7671, 35.6812, 139.7671), 0.0);
    }

    #[test]
    fn haversine_matches_tokyo_to_shin_osaka() {
        // Tokyo station -> Shin-Osaka station, ~400 km great-circle.
        let d = haversine_meters(35.681236, 139.767125, 34.733380, 135.500218);
        assert!(
            (390_000.0..410_000.0).contains(&d),
            "unexpected distance: {d}"
        );
    }

    #[test]
    fn haversine_matches_a_meridian_offset() {
        // One degree of latitude is ~111.19 km on a sphere of radius 6371 km.
        let d = haversine_meters(35.0, 139.0, 36.0, 139.0);
        assert!((d - 111_195.0).abs() < 50.0, "unexpected distance: {d}");
    }

    #[test]
    fn common_cte_binds_are_shared_by_every_tail() {
        for tail in [FREEZES_TAIL, SESSIONS_TAIL, SUMMARY_TAIL] {
            let sql = freeze_sql(tail);
            assert!(sql.starts_with("\nWITH ordered AS"));
            assert!(sql.contains("freezes_alive"));
            // the limit is always the last bind parameter
            assert!(sql.contains("LIMIT $13"));
        }
    }

    // ---------------------------------------------------------------------
    // PostgreSQL integration test.
    //
    // Skipped unless THQ_TEST_DATABASE_URL points at a database the test may
    // create tables in, e.g.
    //
    //   THQ_TEST_DATABASE_URL=postgres://thq@127.0.0.1:5433/thq_test cargo test
    //
    // Every run uses a fresh uuid device and session ids, and every query
    // filters on that device, so concurrent runs cannot see each other's rows.
    // ---------------------------------------------------------------------

    use crate::domain::{
        Channel, LogBody, LogLevel, LogType, MovementState, OutgoingCoords, OutgoingInteraction,
        OutgoingLocation, OutgoingLog, Platform,
    };
    use uuid::Uuid;

    const BASE_TS: i64 = 1_700_000_000_000;
    const LAT0: f64 = 35.681236;
    const LON0: f64 = 139.767125;
    /// ~30 km north of `LAT0` (30 000 m / 6 371 000 m, in degrees).
    const LAT_JUMP: f64 = 0.269_795;

    fn location(
        session_id: &str,
        device: &str,
        ts: i64,
        lat: f64,
        speed: Option<f64>,
        app_version: Option<&str>,
    ) -> OutgoingLocation {
        OutgoingLocation {
            id: Uuid::new_v4().to_string(),
            session_id: session_id.to_string(),
            device: device.to_string(),
            state: MovementState::Moving,
            station_id: None,
            line_id: 1,
            coords: OutgoingCoords {
                latitude: lat,
                longitude: LON0,
                accuracy: Some(5.0),
                speed,
            },
            timestamp: ts as u64,
            segment_id: Some("1:101:102".to_string()),
            from_station_id: Some(101),
            to_station_id: Some(102),
            battery_level: Some(0.8),
            battery_state: None,
            app_version: app_version.map(str::to_string),
            platform: app_version.map(|_| Platform::Ios),
            channel: app_version.map(|_| Channel::Canary),
        }
    }

    fn log_event(session_id: &str, ts: i64, app_version: &str) -> OutgoingLog {
        OutgoingLog {
            id: Uuid::new_v4().to_string(),
            session_id: session_id.to_string(),
            device: None,
            app_version: app_version.to_string(),
            platform: Platform::Ios,
            channel: Channel::Canary,
            timestamp: ts as u64,
            log: LogBody {
                r#type: LogType::App,
                level: LogLevel::Info,
                message: "still alive".to_string(),
            },
        }
    }

    fn interaction_event(session_id: &str, ts: i64, app_version: &str) -> OutgoingInteraction {
        OutgoingInteraction {
            id: Uuid::new_v4().to_string(),
            session_id: session_id.to_string(),
            device: None,
            app_version: app_version.to_string(),
            platform: Platform::Ios,
            channel: Channel::Canary,
            timestamp: ts as u64,
            event_name: "tab_change".to_string(),
            properties: None,
        }
    }

    fn base_filter(device: &str) -> LocationFreezeQuery {
        LocationFreezeQuery {
            from_ts: BASE_TS - 60_000,
            to_ts: BASE_TS + 3_600_000,
            session_id: None,
            line_id: None,
            segment_id: None,
            device: Some(device.to_string()),
            app_version: None,
            platform: None,
            channel: None,
            gap_threshold_ms: 60_000,
            speed_threshold_kmh: 30.0,
            require_app_alive: true,
            limit: 100,
        }
    }

    #[tokio::test]
    async fn freeze_queries_detect_the_mobileapp_6883_signature() {
        let Ok(url) = std::env::var("THQ_TEST_DATABASE_URL") else {
            eprintln!(
                "skipping freeze_queries_detect_the_mobileapp_6883_signature: \
                 set THQ_TEST_DATABASE_URL to run it"
            );
            return;
        };

        let storage = Storage::connect(Some(url))
            .await
            .expect("connect to the test database");

        let device = format!("dev-{}", Uuid::new_v4());
        let session_a = format!("a-{}", Uuid::new_v4());
        let session_b = format!("b-{}", Uuid::new_v4());
        let session_c = format!("c-{}", Uuid::new_v4());
        let session_d = format!("d-{}", Uuid::new_v4());

        // Session A: 320 km/h, 30 s of 1 Hz rows, then a 300 s hole during
        // which only log events arrive, then rows resume 30 km further north.
        // Its location rows carry no build metadata, so the 10.4.1(100) shown
        // by the query has to come from the session's log events.
        for i in 0..30 {
            storage
                .store_location(&location(
                    &session_a,
                    &device,
                    BASE_TS + i * 1_000,
                    LAT0,
                    Some(320.0),
                    None,
                ))
                .await
                .expect("store session A pre-gap row");
        }
        for i in 0..30 {
            storage
                .store_location(&location(
                    &session_a,
                    &device,
                    BASE_TS + 329_000 + i * 1_000,
                    LAT0 + LAT_JUMP,
                    Some(320.0),
                    None,
                ))
                .await
                .expect("store session A post-gap row");
        }
        // 10 log events strictly inside the gap (29 000 .. 329 000).
        for k in 0..10 {
            storage
                .store_log(&log_event(
                    &session_a,
                    BASE_TS + 54_000 + k * 30_000,
                    "10.4.1(100)",
                ))
                .await
                .expect("store session A in-gap log event");
        }
        // One interaction event outside the gap: it must feed the app_version
        // backfill without inflating aliveEventCount.
        storage
            .store_interaction(&interaction_event(
                &session_a,
                BASE_TS + 5_000,
                "10.4.1(100)",
            ))
            .await
            .expect("store session A interaction event");

        // Session B: the same segment on a newer build, no gap at all.
        for i in 0..120 {
            storage
                .store_location(&location(
                    &session_b,
                    &device,
                    BASE_TS + i * 1_000,
                    LAT0,
                    Some(320.0),
                    Some("10.4.2(101)"),
                ))
                .await
                .expect("store session B row");
        }

        // Session C: stopped at a station, so the same 300 s hole is expected.
        for i in 0..10 {
            storage
                .store_location(&location(
                    &session_c,
                    &device,
                    BASE_TS + i * 1_000,
                    LAT0,
                    Some(0.0),
                    Some("10.4.4(103)"),
                ))
                .await
                .expect("store session C pre-gap row");
        }
        for i in 0..10 {
            storage
                .store_location(&location(
                    &session_c,
                    &device,
                    BASE_TS + 309_000 + i * 1_000,
                    LAT0,
                    Some(0.0),
                    Some("10.4.4(103)"),
                ))
                .await
                .expect("store session C post-gap row");
        }

        // Session D: same hole at speed, but nothing proves the app was alive.
        for i in 0..10 {
            storage
                .store_location(&location(
                    &session_d,
                    &device,
                    BASE_TS + i * 1_000,
                    LAT0,
                    Some(320.0),
                    Some("10.4.3(102)"),
                ))
                .await
                .expect("store session D pre-gap row");
        }
        for i in 0..10 {
            storage
                .store_location(&location(
                    &session_d,
                    &device,
                    BASE_TS + 309_000 + i * 1_000,
                    LAT0 + LAT_JUMP,
                    Some(320.0),
                    Some("10.4.3(102)"),
                ))
                .await
                .expect("store session D post-gap row");
        }

        // --- locationFreezes -------------------------------------------------
        let filter = base_filter(&device);
        let freezes = storage
            .fetch_location_freezes(&filter)
            .await
            .expect("fetch freezes");

        assert_eq!(
            freezes.len(),
            1,
            "only session A satisfies all three conditions"
        );
        let a = &freezes[0];
        assert_eq!(a.session_id, session_a);
        assert_eq!(a.gap_ms, 300_000);
        assert_eq!(a.gap_start, BASE_TS + 29_000);
        assert_eq!(a.gap_end, BASE_TS + 329_000);
        assert_eq!(a.alive_event_count, 10);
        assert_eq!(a.speed_before_gap, 320.0);
        assert_eq!(a.segment_id.as_deref(), Some("1:101:102"));
        // backfilled from the session's log / interaction events
        assert_eq!(a.app_version.as_deref(), Some("10.4.1(100)"));
        assert_eq!(a.platform.as_deref(), Some("ios"));
        assert_eq!(a.channel.as_deref(), Some("canary"));
        let jump = haversine_meters(
            a.lat_before_gap,
            a.lon_before_gap,
            a.lat_after_gap,
            a.lon_after_gap,
        );
        assert!(
            (29_000.0..31_000.0).contains(&jump),
            "unexpected jump distance: {jump}"
        );

        // Dropping the liveness requirement also surfaces session D.
        let lenient = LocationFreezeQuery {
            require_app_alive: false,
            ..base_filter(&device)
        };
        let freezes = storage
            .fetch_location_freezes(&lenient)
            .await
            .expect("fetch freezes without the liveness requirement");
        let ids: Vec<&str> = freezes.iter().map(|r| r.session_id.as_str()).collect();
        assert_eq!(freezes.len(), 2, "sessions A and D, got {ids:?}");
        assert!(ids.contains(&session_a.as_str()));
        assert!(ids.contains(&session_d.as_str()));

        // --- locationFreezeSessions -----------------------------------------
        let sessions = storage
            .fetch_location_freeze_sessions(&filter)
            .await
            .expect("fetch freeze sessions");
        assert_eq!(sessions.len(), 4, "every session in the window is listed");

        let by_id = |id: &str| {
            sessions
                .iter()
                .find(|r| r.session_id == id)
                .unwrap_or_else(|| panic!("session {id} missing"))
        };
        let row_a = by_id(&session_a);
        assert_eq!(row_a.freeze_count, 1);
        assert_eq!(row_a.max_gap_ms, Some(300_000));
        assert_eq!(row_a.total_gap_ms, 300_000);
        assert_eq!(row_a.location_count, 60);
        assert_eq!(row_a.started_at, BASE_TS);
        assert_eq!(row_a.ended_at, BASE_TS + 358_000);
        assert_eq!(row_a.app_version.as_deref(), Some("10.4.1(100)"));
        assert_eq!(row_a.max_speed, Some(320.0));

        let row_b = by_id(&session_b);
        assert_eq!(row_b.freeze_count, 0);
        assert_eq!(row_b.max_gap_ms, None);
        assert_eq!(row_b.total_gap_ms, 0);
        assert_eq!(row_b.location_count, 120);
        assert_eq!(row_b.app_version.as_deref(), Some("10.4.2(101)"));

        assert_eq!(by_id(&session_c).freeze_count, 0);
        assert_eq!(by_id(&session_d).freeze_count, 0);

        // --- locationFreezeSummary ------------------------------------------
        let summary = storage
            .fetch_location_freeze_summary(&filter)
            .await
            .expect("fetch freeze summary");
        assert_eq!(summary.len(), 4, "one row per build on this segment");

        let build = |v: &str| {
            summary
                .iter()
                .find(|r| r.app_version.as_deref() == Some(v))
                .unwrap_or_else(|| panic!("build {v} missing from the summary"))
        };
        let old = build("10.4.1(100)");
        assert_eq!(old.freeze_session_count, 1);
        assert_eq!(old.freeze_count, 1);
        assert_eq!(old.session_count, 1);
        assert_eq!(old.location_count, 60);
        assert_eq!(old.max_gap_ms, Some(300_000));
        assert_eq!(old.total_gap_ms, 300_000);
        assert_eq!(old.segment_id.as_deref(), Some("1:101:102"));
        assert_eq!(old.from_station_id, Some(101));
        assert_eq!(old.to_station_id, Some(102));

        let new = build("10.4.2(101)");
        assert_eq!(new.freeze_session_count, 0);
        assert_eq!(new.freeze_count, 0);
        assert_eq!(new.session_count, 1);
        assert_eq!(new.location_count, 120);
        assert_eq!(new.max_gap_ms, None);
        assert_eq!(new.total_gap_ms, 0);

        // the worst group sorts first
        assert_eq!(summary[0].app_version.as_deref(), Some("10.4.1(100)"));
    }
}
