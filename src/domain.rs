use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MovementState {
    Arrived,
    Approaching,
    Passing,
    Moving,
}

impl MovementState {
    pub fn as_str(&self) -> &'static str {
        match self {
            MovementState::Arrived => "arrived",
            MovementState::Approaching => "approaching",
            MovementState::Passing => "passing",
            MovementState::Moving => "moving",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
}

impl LogLevel {
    pub fn as_str(&self) -> &'static str {
        match self {
            LogLevel::Debug => "debug",
            LogLevel::Info => "info",
            LogLevel::Warn => "warn",
            LogLevel::Error => "error",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LogType {
    System,
    App,
    Client,
}

impl LogType {
    pub fn as_str(&self) -> &'static str {
        match self {
            LogType::System => "system",
            LogType::App => "app",
            LogType::Client => "client",
        }
    }
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
        #[serde(default, rename = "stationId")]
        station_id: Option<i32>,
        #[serde(rename = "lineId")]
        line_id: i32,
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
    pub station_id: Option<i32>,
    pub line_id: i32,
    pub coords: OutgoingCoords,
    pub timestamp: u64,
    pub segment_id: Option<String>,
    pub from_station_id: Option<i32>,
    pub to_station_id: Option<i32>,
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
    _Unknown,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn incoming_subscribe_deserializes() {
        let json = r#"{"type":"subscribe","device":"dev"}"#;
        let v: IncomingMessage = serde_json::from_str(json).unwrap();
        match v {
            IncomingMessage::Subscribe { device } => {
                assert_eq!(device.as_deref(), Some("dev"));
            }
            _ => panic!("expected subscribe variant"),
        }
    }

    #[test]
    fn incoming_location_update_accepts_snake_type_and_camel_fields() {
        let json = r#"{
            "type":"location_update",
            "device":"dev",
            "state":"moving",
            "lineId":7,
            "stationId":42,
            "coords":{"latitude":1.0,"longitude":2.0,"accuracy":null,"speed":3.0},
            "timestamp":123
        }"#;

        let v: IncomingMessage = serde_json::from_str(json).unwrap();
        match v {
            IncomingMessage::LocationUpdate {
                line_id,
                station_id,
                ..
            } => {
                assert_eq!(line_id, 7);
                assert_eq!(station_id, Some(42));
            }
            _ => panic!("expected location update"),
        }
    }

    #[test]
    fn outgoing_location_has_type_field() {
        let msg = OutgoingMessage::LocationUpdate(OutgoingLocation {
            id: "id1".into(),
            device: "dev".into(),
            state: MovementState::Moving,
            station_id: Some(42),
            line_id: 7,
            coords: OutgoingCoords {
                latitude: 1.0,
                longitude: 2.0,
                accuracy: None,
                speed: 3.0,
            },
            timestamp: 42,
            segment_id: None,
            from_station_id: None,
            to_station_id: None,
        });

        let json = serde_json::to_value(&msg).unwrap();
        assert_eq!(json["type"], "location_update");
        assert_eq!(json["device"], "dev");
        assert_eq!(json["coords"]["speed"], 3.0);
    }
}
