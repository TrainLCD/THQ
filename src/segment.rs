use std::{
    collections::{HashMap, HashSet},
    fs,
    path::Path,
    sync::Arc,
};

use anyhow::{anyhow, Context};
use serde::Deserialize;
use tokio::sync::RwLock;
use tracing::warn;

use crate::domain::{MovementState, OutgoingLocation};

#[derive(Clone, Default, Debug)]
pub struct LineTopology {
    lines: Arc<HashMap<i32, LineGraph>>, // line_id -> graph
}

#[derive(Clone, Debug)]
struct LineGraph {
    stations: Vec<i32>,                    // sorted unique station ids
    neighbors: HashMap<i32, Vec<i32>>,     // station_id -> sorted neighbors
}

impl LineTopology {

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
        self.lines.get(&line_id).map(|g| g.stations.as_slice())
    }

    pub fn neighbors(&self, line_id: i32, station_id: i32) -> Option<&[i32]> {
        self.lines
            .get(&line_id)
            .and_then(|g| g.neighbors.get(&station_id))
            .map(|v| v.as_slice())
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
                warn!(
                    path = %path_ref.display(),
                    ext = %ext,
                    "unknown topology file extension; trying JSON then CSV fallback"
                );

                let json_err = match Self::from_json_file(path_ref) {
                    Ok(v) => return Ok(v),
                    Err(e) => e,
                };

                let csv_err = match Self::from_join_csv(path_ref) {
                    Ok(v) => return Ok(v),
                    Err(e) => e,
                };

                Err(anyhow!(
                    "failed to load topology at {} (json error: {}; csv error: {})",
                    path_ref.display(),
                    json_err,
                    csv_err
                ))
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
            lines.insert(line_id, LineGraph::from_ordered_path(v));
        }

        Ok(Self {
            lines: Arc::new(lines),
        })
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
            let graph = LineGraph::from_pairs(line_id, &pairs).with_context(|| {
                format!(
                    "failed to build station graph for line {} from {}",
                    line_id,
                    path_ref.display()
                )
            })?;
            lines.insert(line_id, graph);
        }

        if lines.is_empty() {
            anyhow::bail!(
                "no rows parsed from {}; did you point THQ_LINE_TOPOLOGY_PATH to join.csv?",
                path_ref.display()
            );
        }

        Ok(Self {
            lines: Arc::new(lines),
        })
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

impl LineGraph {
    fn from_ordered_path(stations: Vec<i32>) -> Self {
        let mut neighbors: HashMap<i32, Vec<i32>> = HashMap::new();
        for window in stations.windows(2) {
            let a = window[0];
            let b = window[1];
            neighbors.entry(a).or_default().push(b);
            neighbors.entry(b).or_default().push(a);
        }
        Self::finalize(neighbors)
    }

    fn from_pairs(line_id: i32, pairs: &[(i32, i32)]) -> anyhow::Result<Self> {
        if pairs.is_empty() {
            anyhow::bail!("no station pairs found");
        }

        let mut neighbors: HashMap<i32, Vec<i32>> = HashMap::new();
        for (a, b) in pairs {
            neighbors.entry(*a).or_default().push(*b);
            neighbors.entry(*b).or_default().push(*a);
        }

        // Check connectivity; if disconnected, warn but still accept so inter-line connections don't break loading.
        let stations: Vec<i32> = neighbors.keys().copied().collect();
        let mut remaining: HashSet<i32> = stations.iter().copied().collect();
        let mut components = 0;
        while let Some(&start) = remaining.iter().next() {
            components += 1;
            let mut stack = vec![start];
            while let Some(n) = stack.pop() {
                if !remaining.remove(&n) {
                    continue;
                }
                if let Some(ns) = neighbors.get(&n) {
                    for &m in ns {
                        if remaining.contains(&m) {
                            stack.push(m);
                        }
                    }
                }
            }
        }
        if components > 1 {
            warn!(line_id, components, "line graph has multiple components; keeping as-is");
        }

        Ok(Self::finalize(neighbors))
    }

    fn finalize(mut neighbors: HashMap<i32, Vec<i32>>) -> Self {
        for list in neighbors.values_mut() {
            list.sort_unstable();
            list.dedup();
        }
        let mut stations: Vec<i32> = neighbors.keys().copied().collect();
        stations.sort_unstable();
        LineGraph { stations, neighbors }
    }
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
}

#[derive(Clone, Debug, Default)]
struct DeviceTrack {
    last_station: Option<StationPoint>,
    prev_station: Option<StationPoint>,
    last_segment: Option<Segment>,
    // For eviction of idle devices.
    last_seen: u64,
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
        Self::prune_stale_tracks(&mut tracks, loc.timestamp);
        let track = tracks.entry(loc.device.clone()).or_default();
        track.last_seen = loc.timestamp;

        match loc.state {
            MovementState::Arrived | MovementState::Passing => {
                self.handle_station_event(track, loc, stations)
            }
            MovementState::Approaching | MovementState::Moving => {
                self.handle_continuous(track, loc)
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

        if !stations.contains(&station_id) {
            warn!(device = %loc.device, line_id = loc.line_id, station_id, "station_id not found in topology; skipping segment inference");
            self.reset_track_if_line_changed(track, loc.line_id);
            return None;
        }

        self.reset_track_if_line_changed(track, loc.line_id);

        let prev = track.last_station.take();
        if let Some(prev_station) = prev.clone() {
            track.prev_station = Some(prev_station);
        }

        let current = StationPoint {
            station_id,
            line_id: loc.line_id,
        };

        let segment = prev.as_ref().and_then(|prev_station| {
            let neighbors = self.topology.neighbors(loc.line_id, prev_station.station_id);
            if neighbors.map_or(false, |n| n.contains(&station_id)) {
                Some(Segment {
                    line_id: loc.line_id,
                    from_station_id: prev_station.station_id,
                    to_station_id: station_id,
                })
            } else {
                warn!(device = %loc.device, line_id = loc.line_id, from = prev_station.station_id, to = station_id, "stations not adjacent in topology; segment not inferred");
                None
            }
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
    ) -> Option<Segment> {
        // If the line changes mid-stream, reset state.
        if self.track_on_different_line(track, loc.line_id) {
            self.reset_track(track);
            return None;
        }

        let Some(last_station) = track.last_station.as_ref() else {
            return None;
        };

        let neighbors = match self.topology.neighbors(loc.line_id, last_station.station_id) {
            Some(n) if !n.is_empty() => n,
            _ => {
                warn!(
                    device = %loc.device,
                    line_id = loc.line_id,
                    station_id = last_station.station_id,
                    "no neighbor station found for continuous state"
                );
                return None;
            }
        };

        // Prefer continuing away from the previous station to maintain direction.
        let preferred_avoid = track.prev_station.as_ref().map(|p| p.station_id);
        let mut candidates: Vec<i32> = neighbors
            .iter()
            .copied()
            .filter(|n| Some(*n) != preferred_avoid)
            .collect();

        if candidates.is_empty() {
            // If only the previous station exists, we can't infer forward motion.
            return None;
        }

        // Deterministic pick: smallest station id.
        candidates.sort_unstable();
        let to_station_id = candidates[0];
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
        track.last_segment = None;
    }

    fn prune_stale_tracks(tracks: &mut HashMap<String, DeviceTrack>, now: u64) {
        const TRACK_TTL_SECS: u64 = 6 * 60 * 60; // 6 hours
        tracks.retain(|_, t| now.saturating_sub(t.last_seen) <= TRACK_TTL_SECS);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use uuid::Uuid;

    fn topo() -> LineTopology {
        let mut graphs = HashMap::new();
        graphs.insert(1, LineGraph::from_ordered_path(vec![101, 102, 103, 104]));
        LineTopology {
            lines: Arc::new(graphs),
        }
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

    #[test]
    fn builds_topology_with_branching_graph() {
        let path = std::env::temp_dir().join(format!("join_branch_{}.csv", Uuid::new_v4()));
        fs::write(&path, "line_cd,station_cd1,station_cd2\n1,1,2\n1,2,3\n1,2,4\n").unwrap();

        let topo = LineTopology::from_file(&path).expect("csv topology loads");
        let stations = topo.stations(1).expect("line exists");
        assert_eq!(stations, &[1, 2, 3, 4]);
        let neighbors = topo.neighbors(1, 2).expect("has neighbors");
        assert_eq!(neighbors, &[1, 3, 4]);

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn builds_topology_with_disconnected_components() {
        let path = std::env::temp_dir().join(format!("join_disconnected_{}.csv", Uuid::new_v4()));
        // two components: 1-2 and 10-11
        fs::write(&path, "line_cd,station_cd1,station_cd2\n1,1,2\n1,10,11\n").unwrap();

        let topo = LineTopology::from_file(&path).expect("csv topology loads");
        let stations = topo.stations(1).expect("line exists");
        assert_eq!(stations, &[1, 2, 10, 11]);
        assert_eq!(topo.neighbors(1, 1).unwrap(), &[2]);
        assert_eq!(topo.neighbors(1, 10).unwrap(), &[11]);

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn unknown_extension_reports_both_errors() {
        let path = std::env::temp_dir().join(format!("topo_{}.foo", Uuid::new_v4()));
        fs::write(&path, "not json or csv").unwrap();

        let err = LineTopology::from_file(&path).expect_err("should fail with both errors");
        let msg = format!("{err:#}");
        assert!(msg.contains("json error"));
        assert!(msg.contains("csv error"));

        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn prunes_stale_device_tracks() {
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

        // first annotate stores track
        let _ = estimator.annotate(first.clone()).await;

        // far future update should remove old track before adding new device
        let future = OutgoingLocation {
            timestamp: 6 * 60 * 60 + 2, // past TTL
            device: "new_dev".into(),
            ..first
        };

        let _ = estimator.annotate(future).await;

        let tracks = estimator.tracks.read().await;
        assert_eq!(tracks.len(), 1);
        assert!(tracks.contains_key("new_dev"));
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
