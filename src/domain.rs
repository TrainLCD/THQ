use serde::{Deserialize, Serialize};
use serde_repr::{Deserialize_repr, Serialize_repr};

#[derive(Debug, Clone, Serialize_repr, Deserialize_repr, PartialEq, Eq)]
#[repr(u8)]
pub enum BatteryState {
    Unknown = 0,
    Unplugged = 1,
    Charging = 2,
    Full = 3,
}

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
}

/// REST API用の位置情報リクエスト
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocationUpdateRequest {
    #[serde(default)]
    pub id: Option<String>,
    pub device: String,
    pub state: MovementState,
    #[serde(default)]
    pub station_id: Option<i32>,
    pub line_id: i32,
    pub coords: Coords,
    pub timestamp: u64,
    #[serde(default)]
    pub battery_level: Option<f64>,
    #[serde(default)]
    pub battery_state: Option<BatteryState>,
}

/// REST API用のログリクエスト
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LogRequest {
    #[serde(default)]
    pub id: Option<String>,
    pub device: String,
    pub timestamp: u64,
    pub log: LogBody,
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
    pub battery_level: Option<f64>,
    pub battery_state: Option<BatteryState>,
}

#[derive(Debug, Clone, Serialize)]
pub struct OutgoingCoords {
    pub latitude: f64,
    pub longitude: f64,
    pub accuracy: Option<f64>,
    pub speed: Option<f64>,
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
        }
    }

    #[test]
    fn location_update_request_deserializes() {
        let json = r#"{
            "device":"dev",
            "state":"moving",
            "lineId":7,
            "stationId":42,
            "coords":{"latitude":1.0,"longitude":2.0,"accuracy":null,"speed":3.0},
            "timestamp":123
        }"#;

        let req: LocationUpdateRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.line_id, 7);
        assert_eq!(req.station_id, Some(42));
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
                speed: Some(3.0),
            },
            timestamp: 42,
            segment_id: None,
            from_station_id: None,
            to_station_id: None,
            battery_level: None,
            battery_state: None,
        });

        let json = serde_json::to_value(&msg).unwrap();
        assert_eq!(json["type"], "location_update");
        assert_eq!(json["device"], "dev");
        assert_eq!(json["coords"]["speed"], 3.0); // Some(3.0) serializes as 3.0
    }
}
