use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum TelemetryEvent {
    #[serde(rename = "location_update")]
    LocationUpdate(LocationData),
    #[serde(rename = "error")]
    Error(ErrorData),
    #[serde(rename = "log")]
    LogUpdate(LogData),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LocationData {
    pub id: String,
    pub lat: f64,
    pub lon: f64,
    pub accuracy: Option<f64>,
    pub speed: f64,
    pub device: String,
    pub state: String,
    pub timestamp: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ErrorData {
    pub r#type: String,
    pub reason: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LogData {
    pub r#type: String,
    pub id: String,
    pub timestamp: u64,
    pub level: String,
    pub message: String,
    pub device: String,
}
