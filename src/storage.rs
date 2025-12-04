use std::time::Duration;

use anyhow::Context;
use sqlx::{postgres::PgPoolOptions, PgPool};
use tracing::info;

use crate::domain::{LogLevel, LogType, MovementState, OutgoingLocation, OutgoingLog};

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
            CREATE TABLE IF NOT EXISTS location_events (
                id TEXT PRIMARY KEY,
                device TEXT NOT NULL,
                state TEXT NOT NULL,
                station_id INTEGER,
                line_id INTEGER NOT NULL,
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
        sqlx::query("ALTER TABLE location_events ADD COLUMN IF NOT EXISTS station_id INTEGER;")
            .execute(pool)
            .await?;
        sqlx::query("ALTER TABLE location_events ADD COLUMN IF NOT EXISTS line_id INTEGER;")
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
            "CREATE INDEX IF NOT EXISTS idx_location_events_device ON location_events (device);",
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
            "INSERT INTO location_events (id, device, state, station_id, line_id, latitude, longitude, accuracy, speed, timestamp) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10) ON CONFLICT (id) DO NOTHING",
        )
        .bind(&loc.id)
        .bind(&loc.device)
        .bind(movement_state_str(&loc.state))
        .bind(loc.station_id)
        .bind(loc.line_id)
        .bind(loc.coords.latitude)
        .bind(loc.coords.longitude)
        .bind(loc.coords.accuracy)
        .bind(loc.coords.speed)
        .bind(ts)
        .execute(pool)
        .await
        .context("failed to insert location event")?;

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
