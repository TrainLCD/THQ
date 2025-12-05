use std::{
    collections::{HashMap, HashSet},
    fs,
    path::Path,
    sync::Arc,
};

use anyhow::Context;
use serde::Deserialize;
use tokio::sync::RwLock;
use tracing::warn;

use crate::domain::{MovementState, OutgoingLocation};

#[derive(Clone, Default)]
pub struct LineTopology {
    lines: Arc<HashMap<i32, Vec<i32>>>,
}

impl LineTopology {
    pub fn new(lines: HashMap<i32, Vec<i32>>) -> Self {
        Self {
            lines: Arc::new(lines),
        }
    }

    pub fn empty() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    pub fn line_count(&self) -> usize {
        self.lines.len()
    }

    pub fn stations(&self, line_id: i32) -> Option<&[i32]> {
        self.lines.get(&line_id).map(|v| v.as_slice())
    }

    pub fn from_file(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let path_ref = path.as_ref();
        let ext = path_ref
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();

        match ext.as_str() {
            "json" => Self::from_json_file(path_ref),
            "csv" => Self::from_join_csv(path_ref),
            _ => {
                // try json then csv as fallback for convenience
                Self::from_json_file(path_ref).or_else(|_| Self::from_join_csv(path_ref))
            }
        }
    }

    fn from_json_file(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let path_ref = path.as_ref();
        let raw = fs::read_to_string(path_ref).with_context(|| {
            format!(
                "failed to read line topology file at {}",
                path_ref.display()
            )
        })?;

        let parsed: HashMap<String, Vec<i32>> = serde_json::from_str(&raw).with_context(|| {
            format!(
                "failed to parse line topology JSON at {}; expected object: {{ \"<line_id>\": [<station_id>...] }}",
                path_ref.display()
            )
        })?;

        let mut lines = HashMap::new();
        for (k, v) in parsed {
            let line_id: i32 = k.parse().with_context(|| {
                format!(
                    "line id keys must be integers but got '{k}' in {}",
                    path_ref.display()
                )
            })?;
            lines.insert(line_id, v);
        }

        Ok(Self::new(lines))
    }

    fn from_join_csv(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let path_ref = path.as_ref();
        let mut rdr = csv::ReaderBuilder::new()
            .has_headers(true)
            .flexible(true)
            .from_path(path_ref)
            .with_context(|| format!("failed to open topology CSV at {}", path_ref.display()))?;

        #[derive(Deserialize)]
        struct Row {
            line_cd: i32,
            station_cd1: i32,
            station_cd2: i32,
        }

        let mut edges: HashMap<i32, Vec<(i32, i32)>> = HashMap::new();
        for rec in rdr.deserialize() {
            let row: Row = rec.with_context(|| {
                format!(
                    "failed to parse row in {} as (line_cd, station_cd1, station_cd2)",
                    path_ref.display()
                )
            })?;
            edges
                .entry(row.line_cd)
                .or_default()
                .push((row.station_cd1, row.station_cd2));
        }

        let mut lines = HashMap::new();
        for (line_id, pairs) in edges {
            let ordered = order_stations_from_pairs(&pairs).with_context(|| {
                format!(
                    "failed to build station order for line {} from {}",
                    line_id,
                    path_ref.display()
                )
            })?;
            lines.insert(line_id, ordered);
        }

        if lines.is_empty() {
            anyhow::bail!(
                "no rows parsed from {}; did you point THQ_LINE_TOPOLOGY_PATH to join.csv?",
                path_ref.display()
            );
        }

        Ok(Self::new(lines))
    }

    pub fn from_env_var(var: &str) -> anyhow::Result<Option<Self>> {
        if let Ok(path) = std::env::var(var) {
            if path.trim().is_empty() {
                return Ok(None);
            }
            let topo = Self::from_file(path)?;
            Ok(Some(topo))
        } else {
            Ok(None)
        }
    }
}

fn order_stations_from_pairs(pairs: &[(i32, i32)]) -> anyhow::Result<Vec<i32>> {
    let mut adj: HashMap<i32, Vec<i32>> = HashMap::new();
    for (a, b) in pairs {
        adj.entry(*a).or_default().push(*b);
        adj.entry(*b).or_default().push(*a);
    }

    if adj.is_empty() {
        anyhow::bail!("no station pairs found");
    }

    // Pick an end point (degree 1) if available, otherwise smallest id to make deterministic.
    let start = adj
        .iter()
        .filter(|(_, v)| v.len() == 1)
        .map(|(k, _)| *k)
        .min()
        .unwrap_or_else(|| *adj.keys().min().expect("non-empty adj"));

    let mut order = Vec::with_capacity(adj.len());
    let mut visited: HashSet<i32> = HashSet::new();
    let mut prev: Option<i32> = None;
    let mut current = start;

    loop {
        order.push(current);
        visited.insert(current);

        let neighbors = adj.get(&current).map(|v| v.as_slice()).unwrap_or(&[]);
        let next = neighbors
            .iter()
            .copied()
            .filter(|n| Some(*n) != prev && !visited.contains(n))
            .min();

        match next {
            Some(nxt) => {
                prev = Some(current);
                current = nxt;
            }
            None => break,
        }
    }

    Ok(order)
}

#[derive(Clone, Debug)]
pub struct Segment {
    pub line_id: i32,
    pub from_station_id: i32,
    pub to_station_id: i32,
}

impl Segment {
    pub fn segment_id(&self) -> String {
        format!(
            "{}:{}:{}",
            self.line_id, self.from_station_id, self.to_station_id
        )
    }
}

#[derive(Clone, Debug)]
struct StationPoint {
    station_id: i32,
    line_id: i32,
    idx: usize,
}

#[derive(Clone, Debug, Default)]
struct DeviceTrack {
    last_station: Option<StationPoint>,
    prev_station: Option<StationPoint>,
    last_direction: Option<Direction>,
    last_segment: Option<Segment>,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Direction {
    Forward,
    Backward,
}

impl Direction {
    fn step(self) -> isize {
        match self {
            Direction::Forward => 1,
            Direction::Backward => -1,
        }
    }
}

#[derive(Clone, Default)]
pub struct SegmentEstimator {
    topology: LineTopology,
    tracks: Arc<RwLock<HashMap<String, DeviceTrack>>>,
}

impl SegmentEstimator {
    pub fn new(topology: LineTopology) -> Self {
        Self {
            topology,
            tracks: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Annotate the outgoing location with the inferred segment (if available).
    pub async fn annotate(&self, loc: OutgoingLocation) -> OutgoingLocation {
        let segment = self.estimate_segment(&loc).await;
        let mut enriched = loc;

        if let Some(seg) = segment {
            enriched.from_station_id = Some(seg.from_station_id);
            enriched.to_station_id = Some(seg.to_station_id);
            enriched.segment_id = Some(seg.segment_id());
        } else {
            enriched.segment_id = None;
            enriched.from_station_id = None;
            enriched.to_station_id = None;
        }

        enriched
    }

    async fn estimate_segment(&self, loc: &OutgoingLocation) -> Option<Segment> {
        let Some(stations) = self.topology.stations(loc.line_id) else {
            return None;
        };

        let mut tracks = self.tracks.write().await;
        let track = tracks.entry(loc.device.clone()).or_default();

        match loc.state {
            MovementState::Arrived | MovementState::Passing => {
                self.handle_station_event(track, loc, stations)
            }
            MovementState::Approaching | MovementState::Moving => {
                self.handle_continuous(track, loc, stations)
            }
        }
    }

    fn handle_station_event(
        &self,
        track: &mut DeviceTrack,
        loc: &OutgoingLocation,
        stations: &[i32],
    ) -> Option<Segment> {
        let station_id = match loc.station_id {
            Some(v) => v,
            None => {
                warn!(device = %loc.device, line_id = loc.line_id, "station_id missing on station event; cannot infer segment");
                return None;
            }
        };

        let idx = match stations.iter().position(|s| *s == station_id) {
            Some(i) => i,
            None => {
                warn!(device = %loc.device, line_id = loc.line_id, station_id, "station_id not found in topology; skipping segment inference");
                self.reset_track_if_line_changed(track, loc.line_id);
                return None;
            }
        };

        self.reset_track_if_line_changed(track, loc.line_id);

        let prev = track.last_station.take();
        if let Some(prev_station) = prev.clone() {
            track.prev_station = Some(prev_station);
        }

        let current = StationPoint {
            station_id,
            line_id: loc.line_id,
            idx,
        };

        let segment = prev.as_ref().and_then(|prev_station| {
            direction_from_indices(prev_station.idx, idx).map(|dir| {
                track.last_direction = Some(dir);
                Segment {
                    line_id: loc.line_id,
                    from_station_id: prev_station.station_id,
                    to_station_id: station_id,
                }
            })
        });

        if segment.is_none() {
            // Keep prior direction if we revisited the same station, but avoid stale segments.
            track.last_segment = None;
        }

        if let Some(seg) = segment.clone() {
            track.last_segment = Some(seg);
        }

        track.last_station = Some(current);
        segment
    }

    fn handle_continuous(
        &self,
        track: &mut DeviceTrack,
        loc: &OutgoingLocation,
        stations: &[i32],
    ) -> Option<Segment> {
        // If the line changes mid-stream, reset state.
        if self.track_on_different_line(track, loc.line_id) {
            self.reset_track(track);
            return None;
        }

        let Some(last_station) = track.last_station.as_ref() else {
            return None;
        };

        if let Some(seg) = track
            .last_segment
            .as_ref()
            .filter(|s| s.line_id == loc.line_id && s.to_station_id != last_station.station_id)
            .cloned()
        {
            return Some(seg);
        }

        let direction = track.last_direction?;
        let next_idx = last_station.idx as isize + direction.step();
        if next_idx < 0 || next_idx >= stations.len() as isize {
            warn!(
                device = %loc.device,
                line_id = loc.line_id,
                station_id = last_station.station_id,
                "no neighbor station found for continuous state; out of bounds"
            );
            return None;
        }

        let to_station_id = stations[next_idx as usize];
        let seg = Segment {
            line_id: loc.line_id,
            from_station_id: last_station.station_id,
            to_station_id,
        };
        track.last_segment = Some(seg.clone());
        Some(seg)
    }

    fn reset_track_if_line_changed(&self, track: &mut DeviceTrack, new_line_id: i32) {
        if let Some(last) = track.last_station.as_ref() {
            if last.line_id != new_line_id {
                self.reset_track(track);
            }
        }
    }

    fn track_on_different_line(&self, track: &DeviceTrack, line_id: i32) -> bool {
        track
            .last_station
            .as_ref()
            .map(|s| s.line_id != line_id)
            .unwrap_or(false)
    }

    fn reset_track(&self, track: &mut DeviceTrack) {
        track.last_station = None;
        track.prev_station = None;
        track.last_direction = None;
        track.last_segment = None;
    }
}

fn direction_from_indices(prev_idx: usize, curr_idx: usize) -> Option<Direction> {
    match curr_idx.cmp(&prev_idx) {
        std::cmp::Ordering::Greater => Some(Direction::Forward),
        std::cmp::Ordering::Less => Some(Direction::Backward),
        std::cmp::Ordering::Equal => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use uuid::Uuid;

    fn topo() -> LineTopology {
        let mut lines = HashMap::new();
        lines.insert(1, vec![101, 102, 103, 104]);
        LineTopology::new(lines)
    }

    #[tokio::test]
    async fn infers_segment_from_back_to_back_station_events() {
        let estimator = SegmentEstimator::new(topo());

        let first = OutgoingLocation {
            id: "1".into(),
            device: "dev".into(),
            state: MovementState::Arrived,
            station_id: Some(101),
            line_id: 1,
            coords: crate::domain::OutgoingCoords {
                latitude: 0.0,
                longitude: 0.0,
                accuracy: None,
                speed: 0.0,
            },
            timestamp: 1,
            segment_id: None,
            from_station_id: None,
            to_station_id: None,
        };

        let second = OutgoingLocation {
            timestamp: 2,
            station_id: Some(102),
            ..first.clone()
        };

        let annotated1 = estimator.annotate(first).await;
        assert!(annotated1.segment_id.is_none());

        let annotated2 = estimator.annotate(second).await;
        assert_eq!(annotated2.segment_id.as_deref(), Some("1:101:102"));
        assert_eq!(annotated2.from_station_id, Some(101));
        assert_eq!(annotated2.to_station_id, Some(102));
    }

    #[tokio::test]
    async fn uses_direction_for_moving_between_stations() {
        let estimator = SegmentEstimator::new(topo());

        let base = OutgoingLocation {
            id: "1".into(),
            device: "dev".into(),
            state: MovementState::Arrived,
            station_id: Some(101),
            line_id: 1,
            coords: crate::domain::OutgoingCoords {
                latitude: 0.0,
                longitude: 0.0,
                accuracy: None,
                speed: 0.0,
            },
            timestamp: 1,
            segment_id: None,
            from_station_id: None,
            to_station_id: None,
        };

        let second = OutgoingLocation {
            timestamp: 2,
            station_id: Some(102),
            ..base.clone()
        };

        let moving = OutgoingLocation {
            state: MovementState::Moving,
            station_id: None,
            timestamp: 3,
            ..base.clone()
        };

        let _ = estimator.annotate(base).await;
        let _ = estimator.annotate(second).await;
        let annotated_moving = estimator.annotate(moving).await;

        assert_eq!(annotated_moving.segment_id.as_deref(), Some("1:102:103"));
        assert_eq!(annotated_moving.from_station_id, Some(102));
        assert_eq!(annotated_moving.to_station_id, Some(103));
    }

    #[test]
    fn builds_topology_from_join_csv() {
        let path = std::env::temp_dir().join(format!("join_{}.csv", Uuid::new_v4()));
        fs::write(&path, "line_cd,station_cd1,station_cd2\n1,10,11\n1,11,12\n").unwrap();

        let topo = LineTopology::from_file(&path).expect("csv topology loads");
        let stations = topo.stations(1).expect("line exists");
        assert_eq!(stations, &[10, 11, 12]);

        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn returns_none_when_topology_missing() {
        let estimator = SegmentEstimator::new(LineTopology::empty());

        let loc = OutgoingLocation {
            id: "1".into(),
            device: "dev".into(),
            state: MovementState::Arrived,
            station_id: Some(1),
            line_id: 99,
            coords: crate::domain::OutgoingCoords {
                latitude: 0.0,
                longitude: 0.0,
                accuracy: None,
                speed: 0.0,
            },
            timestamp: 1,
            segment_id: None,
            from_station_id: None,
            to_station_id: None,
        };

        let annotated = estimator.annotate(loc).await;
        assert!(annotated.segment_id.is_none());
        assert!(annotated.from_station_id.is_none());
        assert!(annotated.to_station_id.is_none());
    }
}
