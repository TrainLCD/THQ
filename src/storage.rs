use std::time::Duration;

use anyhow::Context;
use sqlx::{postgres::PgPoolOptions, PgPool};
use tracing::info;

use crate::domain::{
    BatteryState, LogLevel, LogType, MovementState, OutgoingInteraction, OutgoingLocation,
    OutgoingLog,
};

#[derive(Clone, sqlx::FromRow)]
pub struct LineAccuracyBucketRow {
    pub bucket_start: sqlx::types::chrono::DateTime<sqlx::types::chrono::Utc>,
    pub bucket_end: sqlx::types::chrono::DateTime<sqlx::types::chrono::Utc>,
    pub avg_accuracy: f64,
    pub p90_accuracy: f64,
    pub sample_count: i32,
    pub avg_speed: Option<f64>,
    pub max_speed: Option<f64>,
}

/// Common optional filters shared by the raw history queries. `None` fields
/// leave the corresponding column unfiltered.
pub struct EventFilter {
    pub session_id: Option<String>,
    pub device: Option<String>,
    /// Inclusive lower bound on the client-reported timestamp, unix millis.
    pub from_ts: Option<i64>,
    /// Exclusive upper bound on the client-reported timestamp, unix millis.
    pub to_ts: Option<i64>,
    pub limit: i32,
}

/// Raw row of the `log_events` table. Columns added by later migrations are
/// nullable because legacy rows predate them.
#[derive(Clone, sqlx::FromRow)]
pub struct LogEventRow {
    pub id: String,
    pub session_id: Option<String>,
    pub device: Option<String>,
    pub app_version: Option<String>,
    pub platform: Option<String>,
    pub channel: Option<String>,
    pub log_type: String,
    pub log_level: String,
    pub message: String,
    pub timestamp: i64,
    pub recorded_at: sqlx::types::chrono::DateTime<sqlx::types::chrono::Utc>,
}

/// Raw row of the `interaction_events` table.
#[derive(Clone, sqlx::FromRow)]
pub struct InteractionEventRow {
    pub id: String,
    pub session_id: Option<String>,
    pub device: Option<String>,
    pub app_version: Option<String>,
    pub platform: Option<String>,
    pub channel: Option<String>,
    pub event_name: String,
    pub properties: Option<serde_json::Value>,
    pub timestamp: i64,
    pub recorded_at: sqlx::types::chrono::DateTime<sqlx::types::chrono::Utc>,
}

/// Raw row of the `location_logs` table.
#[derive(Clone, sqlx::FromRow)]
pub struct LocationEventRow {
    pub id: String,
    pub session_id: Option<String>,
    pub device: String,
    pub state: String,
    pub station_id: Option<i32>,
    pub line_id: Option<i32>,
    pub segment_id: Option<String>,
    pub from_station_id: Option<i32>,
    pub to_station_id: Option<i32>,
    pub latitude: f64,
    pub longitude: f64,
    pub accuracy: Option<f64>,
    pub speed: Option<f64>,
    pub timestamp: i64,
    pub battery_level: Option<f64>,
    pub battery_state: Option<i16>,
    pub recorded_at: sqlx::types::chrono::DateTime<sqlx::types::chrono::Utc>,
}

#[derive(Clone, Default)]
pub struct Storage {
    pool: Option<PgPool>,
}

impl Storage {
    pub async fn connect(database_url: Option<String>) -> anyhow::Result<Self> {
        let Some(url) = database_url else {
            return Ok(Self { pool: None });
        };

        info!(db_url = %mask_password(&url), "connecting to PostgreSQL");

        let pool = PgPoolOptions::new()
            .max_connections(5)
            .min_connections(1)
            .acquire_timeout(Duration::from_secs(5))
            .connect(&url)
            .await
            .context("failed to connect to PostgreSQL; check DATABASE_URL")?;

        let storage = Self { pool: Some(pool) };
        storage.prepare().await?;
        info!("PostgreSQL connection established; schema ready");
        Ok(storage)
    }

    pub fn enabled(&self) -> bool {
        self.pool.is_some()
    }

    async fn prepare(&self) -> anyhow::Result<()> {
        let Some(pool) = &self.pool else {
            return Ok(());
        };

        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS location_logs (
                id TEXT PRIMARY KEY,
                session_id TEXT,
                device TEXT NOT NULL,
                state TEXT NOT NULL,
                station_id INTEGER,
                line_id INTEGER NOT NULL,
                segment_id TEXT,
                from_station_id INTEGER,
                to_station_id INTEGER,
                latitude DOUBLE PRECISION NOT NULL,
                longitude DOUBLE PRECISION NOT NULL,
                accuracy DOUBLE PRECISION,
                speed DOUBLE PRECISION,
                timestamp BIGINT NOT NULL,
                battery_level DOUBLE PRECISION,
                battery_state SMALLINT,
                recorded_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
            );
            "#,
        )
        .execute(pool)
        .await?;

        // best-effort migrations for previously created tables
        sqlx::query("ALTER TABLE location_logs ADD COLUMN IF NOT EXISTS station_id INTEGER;")
            .execute(pool)
            .await?;
        sqlx::query("ALTER TABLE location_logs ADD COLUMN IF NOT EXISTS line_id INTEGER;")
            .execute(pool)
            .await?;
        sqlx::query("ALTER TABLE location_logs ADD COLUMN IF NOT EXISTS segment_id TEXT;")
            .execute(pool)
            .await?;
        sqlx::query("ALTER TABLE location_logs ADD COLUMN IF NOT EXISTS from_station_id INTEGER;")
            .execute(pool)
            .await?;
        sqlx::query("ALTER TABLE location_logs ADD COLUMN IF NOT EXISTS to_station_id INTEGER;")
            .execute(pool)
            .await?;
        // Allow NULL in speed column (previously NOT NULL); idempotent on columns already nullable.
        sqlx::query("ALTER TABLE location_logs ALTER COLUMN speed DROP NOT NULL;")
            .execute(pool)
            .await?;
        sqlx::query(
            "ALTER TABLE location_logs ADD COLUMN IF NOT EXISTS battery_level DOUBLE PRECISION;",
        )
        .execute(pool)
        .await?;
        sqlx::query("ALTER TABLE location_logs ADD COLUMN IF NOT EXISTS battery_state SMALLINT;")
            .execute(pool)
            .await?;
        sqlx::query("ALTER TABLE location_logs ADD COLUMN IF NOT EXISTS session_id TEXT;")
            .execute(pool)
            .await?;

        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS log_events (
                id TEXT PRIMARY KEY,
                session_id TEXT,
                device TEXT,
                app_version TEXT,
                platform TEXT,
                channel TEXT,
                log_type TEXT NOT NULL,
                log_level TEXT NOT NULL,
                message TEXT NOT NULL,
                timestamp BIGINT NOT NULL,
                recorded_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
            );
            "#,
        )
        .execute(pool)
        .await?;

        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS interaction_events (
                id TEXT PRIMARY KEY,
                session_id TEXT,
                device TEXT,
                app_version TEXT,
                platform TEXT,
                channel TEXT,
                properties JSONB,
                event_name TEXT NOT NULL,
                timestamp BIGINT NOT NULL,
                recorded_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
            );
            "#,
        )
        .execute(pool)
        .await?;

        sqlx::query("ALTER TABLE interaction_events ADD COLUMN IF NOT EXISTS app_version TEXT;")
            .execute(pool)
            .await?;
        sqlx::query("ALTER TABLE interaction_events ADD COLUMN IF NOT EXISTS platform TEXT;")
            .execute(pool)
            .await?;
        sqlx::query("ALTER TABLE interaction_events ADD COLUMN IF NOT EXISTS channel TEXT;")
            .execute(pool)
            .await?;
        sqlx::query("ALTER TABLE interaction_events ADD COLUMN IF NOT EXISTS properties JSONB;")
            .execute(pool)
            .await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_location_logs_device ON location_logs (device);",
        )
        .execute(pool)
        .await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_location_logs_segment ON location_logs (segment_id);",
        )
        .execute(pool)
        .await?;

        // Allow NULL in device column so log events can be submitted anonymously;
        // idempotent on columns already nullable.
        sqlx::query("ALTER TABLE log_events ALTER COLUMN device DROP NOT NULL;")
            .execute(pool)
            .await?;
        sqlx::query("ALTER TABLE log_events ADD COLUMN IF NOT EXISTS session_id TEXT;")
            .execute(pool)
            .await?;
        sqlx::query("ALTER TABLE log_events ADD COLUMN IF NOT EXISTS app_version TEXT;")
            .execute(pool)
            .await?;
        sqlx::query("ALTER TABLE log_events ADD COLUMN IF NOT EXISTS platform TEXT;")
            .execute(pool)
            .await?;
        sqlx::query("ALTER TABLE log_events ADD COLUMN IF NOT EXISTS channel TEXT;")
            .execute(pool)
            .await?;

        sqlx::query("CREATE INDEX IF NOT EXISTS idx_log_events_device ON log_events (device);")
            .execute(pool)
            .await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_interaction_events_name ON interaction_events (event_name);",
        )
        .execute(pool)
        .await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_interaction_events_session ON interaction_events (session_id);",
        )
        .execute(pool)
        .await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_interaction_events_device ON interaction_events (device);",
        )
        .execute(pool)
        .await?;

        // history queries page through events by client-reported timestamp
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_location_logs_timestamp ON location_logs (timestamp DESC);",
        )
        .execute(pool)
        .await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_log_events_timestamp ON log_events (timestamp DESC);",
        )
        .execute(pool)
        .await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_interaction_events_timestamp ON interaction_events (timestamp DESC);",
        )
        .execute(pool)
        .await?;

        Ok(())
    }

    pub async fn store_location(&self, loc: &OutgoingLocation) -> anyhow::Result<()> {
        let Some(pool) = &self.pool else {
            return Ok(());
        };

        let ts = i64::try_from(loc.timestamp).unwrap_or(i64::MAX);

        sqlx::query(
            "INSERT INTO location_logs (id, session_id, device, state, station_id, line_id, segment_id, from_station_id, to_station_id, latitude, longitude, accuracy, speed, timestamp, battery_level, battery_state) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16) ON CONFLICT (id) DO NOTHING",
        )
        .bind(&loc.id)
        .bind(&loc.session_id)
        .bind(&loc.device)
        .bind(movement_state_str(&loc.state))
        .bind(loc.station_id)
        .bind(loc.line_id)
        .bind(&loc.segment_id)
        .bind(loc.from_station_id)
        .bind(loc.to_station_id)
        .bind(loc.coords.latitude)
        .bind(loc.coords.longitude)
        .bind(loc.coords.accuracy)
        .bind(loc.coords.speed)
        .bind(ts)
        .bind(loc.battery_level)
        .bind(loc.battery_state.as_ref().map(battery_state_i16))
        .execute(pool)
        .await
        .context("failed to insert location log")?;

        Ok(())
    }

    pub async fn store_log(&self, log: &OutgoingLog) -> anyhow::Result<()> {
        let Some(pool) = &self.pool else {
            return Ok(());
        };

        let ts = i64::try_from(log.timestamp).unwrap_or(i64::MAX);

        sqlx::query(
            "INSERT INTO log_events (id, session_id, device, app_version, platform, channel, log_type, log_level, message, timestamp) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10) ON CONFLICT (id) DO NOTHING",
        )
        .bind(&log.id)
        .bind(&log.session_id)
        .bind(&log.device)
        .bind(&log.app_version)
        .bind(log.platform.as_str())
        .bind(log.channel.as_str())
        .bind(log_type_str(&log.log.r#type))
        .bind(log_level_str(&log.log.level))
        .bind(&log.log.message)
        .bind(ts)
        .execute(pool)
        .await
        .context("failed to insert log event")?;

        Ok(())
    }

    pub async fn store_interaction(&self, event: &OutgoingInteraction) -> anyhow::Result<()> {
        let Some(pool) = &self.pool else {
            return Ok(());
        };

        let ts = i64::try_from(event.timestamp).unwrap_or(i64::MAX);

        let properties = event
            .properties
            .as_ref()
            .and_then(|p| serde_json::to_value(p).ok());

        sqlx::query(
            "INSERT INTO interaction_events (id, session_id, device, app_version, platform, channel, properties, event_name, timestamp) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) ON CONFLICT (id) DO NOTHING",
        )
        .bind(&event.id)
        .bind(&event.session_id)
        .bind(&event.device)
        .bind(&event.app_version)
        .bind(event.platform.as_str())
        .bind(event.channel.as_str())
        .bind(properties)
        .bind(&event.event_name)
        .bind(ts)
        .execute(pool)
        .await
        .context("failed to insert interaction event")?;

        Ok(())
    }

    pub async fn fetch_line_accuracy(
        &self,
        line_id: i32,
        from: sqlx::types::chrono::DateTime<sqlx::types::chrono::Utc>,
        to: sqlx::types::chrono::DateTime<sqlx::types::chrono::Utc>,
        trunc_unit: &str,
        bucket_seconds: i64,
        limit: i32,
    ) -> anyhow::Result<Vec<LineAccuracyBucketRow>> {
        let pool = self
            .pool
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("database is not configured"))?;

        let rows = sqlx::query_as::<_, LineAccuracyBucketRow>(
            r#"
            SELECT
                date_trunc($1, ts) AS bucket_start,
                date_trunc($1, ts) + make_interval(secs => $2) AS bucket_end,
                AVG(accuracy) AS avg_accuracy,
                percentile_cont(0.9) WITHIN GROUP (ORDER BY accuracy) AS p90_accuracy,
                COUNT(*)::int AS sample_count,
                AVG(speed) AS avg_speed,
                MAX(speed) AS max_speed
            FROM (
                SELECT
                    (to_timestamp(timestamp / 1000.0) AT TIME ZONE 'UTC')::timestamptz AS ts,
                    accuracy,
                    speed
                FROM location_logs
                WHERE line_id = $3
                  AND to_timestamp(timestamp / 1000.0) >= $4
                  AND to_timestamp(timestamp / 1000.0) < $5
                  AND accuracy IS NOT NULL
            ) AS raw
            GROUP BY 1,2
            ORDER BY bucket_start
            LIMIT $6
            "#,
        )
        .bind(trunc_unit)
        .bind(bucket_seconds as f64)
        .bind(line_id)
        .bind(from)
        .bind(to)
        .bind(limit)
        .fetch_all(pool)
        .await?;

        Ok(rows)
    }

    /// Fetches persisted log events, newest first.
    pub async fn fetch_log_events(
        &self,
        filter: &EventFilter,
        log_type: Option<&str>,
        level: Option<&str>,
    ) -> anyhow::Result<Vec<LogEventRow>> {
        let pool = self
            .pool
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("database is not configured"))?;

        let rows = sqlx::query_as::<_, LogEventRow>(
            r#"
            SELECT id, session_id, device, app_version, platform, channel,
                   log_type, log_level, message, timestamp, recorded_at
            FROM log_events
            WHERE ($1::text IS NULL OR session_id = $1)
              AND ($2::text IS NULL OR device = $2)
              AND ($3::bigint IS NULL OR timestamp >= $3)
              AND ($4::bigint IS NULL OR timestamp < $4)
              AND ($5::text IS NULL OR log_type = $5)
              AND ($6::text IS NULL OR log_level = $6)
            ORDER BY timestamp DESC
            LIMIT $7
            "#,
        )
        .bind(&filter.session_id)
        .bind(&filter.device)
        .bind(filter.from_ts)
        .bind(filter.to_ts)
        .bind(log_type)
        .bind(level)
        .bind(filter.limit)
        .fetch_all(pool)
        .await
        .context("failed to fetch log events")?;

        Ok(rows)
    }

    /// Fetches persisted interaction events, newest first.
    pub async fn fetch_interaction_events(
        &self,
        filter: &EventFilter,
        event_name: Option<&str>,
    ) -> anyhow::Result<Vec<InteractionEventRow>> {
        let pool = self
            .pool
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("database is not configured"))?;

        let rows = sqlx::query_as::<_, InteractionEventRow>(
            r#"
            SELECT id, session_id, device, app_version, platform, channel,
                   event_name, properties, timestamp, recorded_at
            FROM interaction_events
            WHERE ($1::text IS NULL OR session_id = $1)
              AND ($2::text IS NULL OR device = $2)
              AND ($3::bigint IS NULL OR timestamp >= $3)
              AND ($4::bigint IS NULL OR timestamp < $4)
              AND ($5::text IS NULL OR event_name = $5)
            ORDER BY timestamp DESC
            LIMIT $6
            "#,
        )
        .bind(&filter.session_id)
        .bind(&filter.device)
        .bind(filter.from_ts)
        .bind(filter.to_ts)
        .bind(event_name)
        .bind(filter.limit)
        .fetch_all(pool)
        .await
        .context("failed to fetch interaction events")?;

        Ok(rows)
    }

    /// Fetches persisted location updates, newest first.
    pub async fn fetch_locations(
        &self,
        filter: &EventFilter,
        line_id: Option<i32>,
        state: Option<&str>,
    ) -> anyhow::Result<Vec<LocationEventRow>> {
        let pool = self
            .pool
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("database is not configured"))?;

        let rows = sqlx::query_as::<_, LocationEventRow>(
            r#"
            SELECT id, session_id, device, state, station_id, line_id,
                   segment_id, from_station_id, to_station_id,
                   latitude, longitude, accuracy, speed,
                   timestamp, battery_level, battery_state, recorded_at
            FROM location_logs
            WHERE ($1::text IS NULL OR session_id = $1)
              AND ($2::text IS NULL OR device = $2)
              AND ($3::bigint IS NULL OR timestamp >= $3)
              AND ($4::bigint IS NULL OR timestamp < $4)
              AND ($5::integer IS NULL OR line_id = $5)
              AND ($6::text IS NULL OR state = $6)
            ORDER BY timestamp DESC
            LIMIT $7
            "#,
        )
        .bind(&filter.session_id)
        .bind(&filter.device)
        .bind(filter.from_ts)
        .bind(filter.to_ts)
        .bind(line_id)
        .bind(state)
        .bind(filter.limit)
        .fetch_all(pool)
        .await
        .context("failed to fetch location updates")?;

        Ok(rows)
    }
}

fn movement_state_str(state: &MovementState) -> &'static str {
    state.as_str()
}

fn log_type_str(ty: &LogType) -> &'static str {
    ty.as_str()
}

fn log_level_str(level: &LogLevel) -> &'static str {
    level.as_str()
}

fn battery_state_i16(state: &BatteryState) -> i16 {
    match state {
        BatteryState::Unknown => 0,
        BatteryState::Unplugged => 1,
        BatteryState::Charging => 2,
        BatteryState::Full => 3,
    }
}

fn mask_password(url: &str) -> String {
    if let Some(pos) = url.find("@") {
        if let Some(prefix_end) = url[..pos].find("://") {
            let start = prefix_end + 3;
            let redacted = "***";
            return format!("{}{}{}", &url[..start], redacted, &url[pos..]);
        }
    }
    url.to_string()
}
