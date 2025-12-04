use std::time::Instant;

use async_graphql::{
    Context, EmptyMutation, EmptySubscription, Enum, Object, Result, Schema, SimpleObject, ID,
};
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use tracing::info;

use crate::storage::{LineAccuracyBucketRow, Storage};

/// Public schema type so the server can hold and share it.
pub type AppSchema = Schema<QueryRoot, EmptyMutation, EmptySubscription>;

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
    pub avg_speed: f64,
    pub max_speed: f64,
}

#[derive(SimpleObject, Clone)]
pub struct LineAccuracyReport {
    pub line_id: ID,
    pub buckets: Vec<LineAccuracyBucket>,
}

pub fn build_schema(storage: Storage) -> AppSchema {
    Schema::build(QueryRoot, EmptyMutation, EmptySubscription)
        .data(storage)
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
