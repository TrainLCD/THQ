use std::time::Duration;

use anyhow::Context;
use sqlx::{postgres::PgPoolOptions, PgPool};
use tracing::info;

use crate::domain::{LogLevel, LogType, MovementState, OutgoingLocation, OutgoingLog};

#[derive(Clone, sqlx::FromRow)]
pub struct LineAccuracyBucketRow {
    pub bucket_start: sqlx::types::chrono::DateTime<sqlx::types::chrono::Utc>,
    pub bucket_end: sqlx::types::chrono::DateTime<sqlx::types::chrono::Utc>,
    pub avg_accuracy: f64,
    pub p90_accuracy: f64,
    pub sample_count: i32,
    pub avg_speed: f64,
    pub max_speed: f64,
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
                speed DOUBLE PRECISION NOT NULL,
                timestamp BIGINT NOT NULL,
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
        sqlx::query(
            "ALTER TABLE location_logs ADD COLUMN IF NOT EXISTS from_station_id INTEGER;",
        )
        .execute(pool)
        .await?;
        sqlx::query("ALTER TABLE location_logs ADD COLUMN IF NOT EXISTS to_station_id INTEGER;")
            .execute(pool)
            .await?;

        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS log_events (
                id TEXT PRIMARY KEY,
                device TEXT NOT NULL,
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
            "CREATE INDEX IF NOT EXISTS idx_location_logs_device ON location_logs (device);",
        )
        .execute(pool)
        .await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_location_logs_segment ON location_logs (segment_id);",
        )
        .execute(pool)
        .await?;


        sqlx::query("CREATE INDEX IF NOT EXISTS idx_log_events_device ON log_events (device);")
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
            "INSERT INTO location_logs (id, device, state, station_id, line_id, segment_id, from_station_id, to_station_id, latitude, longitude, accuracy, speed, timestamp) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13) ON CONFLICT (id) DO NOTHING",
        )
        .bind(&loc.id)
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
            "INSERT INTO log_events (id, device, log_type, log_level, message, timestamp) VALUES ($1, $2, $3, $4, $5, $6) ON CONFLICT (id) DO NOTHING",
        )
        .bind(&log.id)
        .bind(&log.device)
        .bind(log_type_str(&log.log.r#type))
        .bind(log_level_str(&log.log.level))
        .bind(&log.log.message)
        .bind(ts)
        .execute(pool)
        .await
        .context("failed to insert log event")?;

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
