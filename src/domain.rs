use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MovementState {
    Arrived,
    Approaching,
    Passing,
    Moving,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LogType {
    System,
    App,
    Client,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Coords {
    pub latitude: f64,
    pub longitude: f64,
    pub accuracy: Option<f64>,
    pub speed: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogBody {
    pub r#type: LogType,
    pub level: LogLevel,
    pub message: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum IncomingMessage {
    Subscribe {
        #[serde(default)]
        device: Option<String>,
    },
    LocationUpdate {
        #[serde(default)]
        id: Option<String>,
        device: String,
        state: MovementState,
        coords: Coords,
        timestamp: u64,
    },
    Log {
        #[serde(default)]
        id: Option<String>,
        device: String,
        timestamp: u64,
        log: LogBody,
    },
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OutgoingMessage {
    LocationUpdate(OutgoingLocation),
    Log(OutgoingLog),
    Error(OutgoingError),
}

#[derive(Debug, Clone, Serialize)]
pub struct OutgoingLocation {
    pub id: String,
    pub device: String,
    pub state: MovementState,
    pub coords: OutgoingCoords,
    pub timestamp: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct OutgoingCoords {
    pub latitude: f64,
    pub longitude: f64,
    pub accuracy: Option<f64>,
    pub speed: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct OutgoingLog {
    pub id: String,
    pub device: String,
    pub timestamp: u64,
    pub log: LogBody,
}

#[derive(Debug, Clone, Serialize)]
pub struct OutgoingError {
    pub error: ErrorBody,
}

#[derive(Debug, Clone, Serialize)]
pub struct ErrorBody {
    pub r#type: ErrorType,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorType {
    WebsocketMessageError,
    JsonParseError,
    PayloadParseError,
    AccuracyLow,
    InvalidCoords,
    Unknown,
}
