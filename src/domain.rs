use std::collections::HashMap;

use async_graphql::Enum;
use serde::{Deserialize, Serialize};
use serde_repr::{Deserialize_repr, Serialize_repr};

/// Value type allowed in event properties.
/// TS equivalent: `string | number | boolean | null`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PropertyValue {
    Null,
    Bool(bool),
    Number(serde_json::Number),
    String(String),
}

/// Flat property map. Nested objects and arrays are rejected at
/// deserialization because PropertyValue has no variant for them.
/// TS equivalent: `Record<string, string | number | boolean | null>`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Properties(pub HashMap<String, PropertyValue>);

async_graphql::scalar!(
    Properties,
    "Properties",
    "A flat JSON object whose values are string, number, boolean or null \
     (no nested objects or arrays), i.e. Record<string, string | number | boolean | null>."
);

#[derive(Debug, Clone, Copy, Serialize_repr, Deserialize_repr, PartialEq, Eq, Enum)]
#[graphql(rename_items = "lowercase")]
#[repr(u8)]
pub enum BatteryState {
    Unknown = 0,
    Unplugged = 1,
    Charging = 2,
    Full = 3,
}

impl BatteryState {
    /// Converts a persisted `SMALLINT` value to its corresponding battery state.
    ///
    /// # Examples
    ///
    /// ```
    /// assert_eq!(BatteryState::from_i16(2), Some(BatteryState::Charging));
    /// assert_eq!(BatteryState::from_i16(99), None);
    /// ```
    ///
    /// # Returns
    ///
    /// `Some` with the matching battery state, or `None` for an unknown value.
    ///
    /// # Arguments
    ///
    /// * `value` - The persisted battery state value.
    pub fn from_i16(value: i16) -> Option<Self> {
        match value {
            0 => Some(BatteryState::Unknown),
            1 => Some(BatteryState::Unplugged),
            2 => Some(BatteryState::Charging),
            3 => Some(BatteryState::Full),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Enum)]
#[graphql(rename_items = "lowercase")]
#[serde(rename_all = "snake_case")]
pub enum MovementState {
    Arrived,
    Approaching,
    Passing,
    Moving,
}

impl MovementState {
    /// Converts the movement state to its lowercase string representation.
    ///
    /// # Examples
    ///
    /// ```
    /// assert_eq!(MovementState::Arrived.as_str(), "arrived");
    /// ```
    pub fn as_str(&self) -> &'static str {
        match self {
            MovementState::Arrived => "arrived",
            MovementState::Approaching => "approaching",
            MovementState::Passing => "passing",
            MovementState::Moving => "moving",
        }
    }

    /// Parses a movement state from its lowercase string representation.
    ///
    /// # Examples
    ///
    /// ```
    /// assert_eq!(MovementState::parse("arrived"), Some(MovementState::Arrived));
    /// assert_eq!(MovementState::parse("unknown"), None);
    /// ```
    ///
    /// Returns `None` for unrecognized values.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "arrived" => Some(MovementState::Arrived),
            "approaching" => Some(MovementState::Approaching),
            "passing" => Some(MovementState::Passing),
            "moving" => Some(MovementState::Moving),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Enum)]
#[graphql(rename_items = "lowercase")]
#[serde(rename_all = "snake_case")]
pub enum Platform {
    Ios,
    Android,
    Macos,
    Unknown,
}

impl Platform {
    /// Converts the platform to its lowercase string representation.
    ///
    /// # Examples
    ///
    /// ```
    /// assert_eq!(Platform::Ios.as_str(), "ios");
    /// ```
    ///
    /// # Returns
    ///
    /// The lowercase string corresponding to the platform.
    pub fn as_str(&self) -> &'static str {
        match self {
            Platform::Ios => "ios",
            Platform::Android => "android",
            Platform::Macos => "macos",
            Platform::Unknown => "unknown",
        }
    }

    /// Parses a platform identifier.
    ///
    /// # Returns
    ///
    /// `Some` with the corresponding platform for a recognized identifier, or `None`
    /// for an unrecognized value.
    ///
    /// # Examples
    ///
    /// ```
    /// assert_eq!(Platform::parse("ios"), Some(Platform::Ios));
    /// assert_eq!(Platform::parse("other"), None);
    /// ```
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "ios" => Some(Platform::Ios),
            "android" => Some(Platform::Android),
            "macos" => Some(Platform::Macos),
            "unknown" => Some(Platform::Unknown),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Enum)]
#[graphql(rename_items = "lowercase")]
#[serde(rename_all = "snake_case")]
pub enum Channel {
    Production,
    Canary,
}

impl Channel {
    /// Returns the lowercase string representation of the channel.
    ///
    /// # Examples
    ///
    /// ```
    /// assert_eq!(Channel::Production.as_str(), "production");
    /// assert_eq!(Channel::Canary.as_str(), "canary");
    /// ```
    pub fn as_str(&self) -> &'static str {
        match self {
            Channel::Production => "production",
            Channel::Canary => "canary",
        }
    }

    /// Parses a channel name.
    ///
    /// Returns `Some` channel for `"production"` or `"canary"`, and `None` for unknown values.
    ///
    /// # Examples
    ///
    /// ```
    /// assert_eq!(Channel::parse("production"), Some(Channel::Production));
    /// assert_eq!(Channel::parse("unknown"), None);
    /// ```
    pub fn parse(value: &str) -> Option<Self>
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "production" => Some(Channel::Production),
            "canary" => Some(Channel::Canary),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Enum)]
#[graphql(rename_items = "lowercase")]
#[serde(rename_all = "snake_case")]
pub enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
}

impl LogLevel {
    /// Provides the lowercase string representation of the log level.
    ///
    /// # Examples
    ///
    /// ```
    /// assert_eq!(LogLevel::Info.as_str(), "info");
    /// ```
    pub fn as_str(&self) -> &'static str {
        match self {
            LogLevel::Debug => "debug",
            LogLevel::Info => "info",
            LogLevel::Warn => "warn",
            LogLevel::Error => "error",
        }
    }

    /// Parses a lowercase string into a log level.
    ///
    /// # Examples
    ///
    /// ```
    /// assert_eq!(LogLevel::parse("info"), Some(LogLevel::Info));
    /// ```
    ///
    /// Returns `None` for unrecognized values.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "debug" => Some(LogLevel::Debug),
            "info" => Some(LogLevel::Info),
            "warn" => Some(LogLevel::Warn),
            "error" => Some(LogLevel::Error),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Enum)]
#[graphql(rename_items = "lowercase")]
#[serde(rename_all = "snake_case")]
pub enum LogType {
    System,
    App,
    Client,
}

impl LogType {
    /// Converts the log type to its lowercase string representation.
    ///
    /// # Returns
    ///
    /// The corresponding string: `"system"`, `"app"`, or `"client"`.
    ///
    /// # Examples
    ///
    /// ```
    /// assert_eq!(LogType::System.as_str(), "system");
    /// ```
    pub fn as_str(&self) -> &'static str {
        match self {
            LogType::System => "system",
            LogType::App => "app",
            LogType::Client => "client",
        }
    }

    /// Parses a log type from its lowercase string representation.
    ///
    /// # Examples
    ///
    /// ```
    /// assert_eq!(LogType::parse("app"), Some(LogType::App));
    /// ```
    ///
    /// Returns `None` for unrecognized values.
    pub fn parse(value: &str) -> Option<Self>
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "system" => Some(LogType::System),
            "app" => Some(LogType::App),
            "client" => Some(LogType::Client),
            _ => None,
        }
    }
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

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OutgoingMessage {
    LocationUpdate(OutgoingLocation),
    Log(OutgoingLog),
    Interaction(OutgoingInteraction),
    Error(OutgoingError),
}

#[derive(Debug, Clone, Serialize)]
pub struct OutgoingLocation {
    pub id: String,
    /// Client-generated unique session identifier.
    pub session_id: String,
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
    /// Client-generated unique session identifier.
    pub session_id: String,
    /// None when the sender chose to stay anonymous.
    pub device: Option<String>,
    pub app_version: String,
    pub platform: Platform,
    pub channel: Channel,
    pub timestamp: u64,
    pub log: LogBody,
}

/// A user-driven interaction (e.g. app launch, tab change, TTS request).
/// Unlike logs, which carry console.* output, this records a named action.
#[derive(Debug, Clone, Serialize)]
pub struct OutgoingInteraction {
    pub id: String,
    /// Client-generated unique session identifier.
    pub session_id: String,
    /// None when the sender chose to stay anonymous.
    pub device: Option<String>,
    pub app_version: String,
    pub platform: Platform,
    pub channel: Channel,
    pub timestamp: u64,
    pub event_name: String,
    /// Optional flat map of extra attributes describing the interaction.
    pub properties: Option<Properties>,
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
    fn outgoing_log_has_type_field() {
        let msg = OutgoingMessage::Log(OutgoingLog {
            id: "id1".into(),
            session_id: "sess-1".into(),
            device: Some("dev".into()),
            app_version: "1.2.3".into(),
            platform: Platform::Ios,
            channel: Channel::Production,
            timestamp: 42,
            log: LogBody {
                r#type: LogType::App,
                level: LogLevel::Info,
                message: "hello".into(),
            },
        });

        let json = serde_json::to_value(&msg).unwrap();
        assert_eq!(json["type"], "log");
        assert_eq!(json["device"], "dev");
        assert_eq!(json["log"]["level"], "info");
        assert_eq!(json["log"]["message"], "hello");
    }

    #[test]
    fn outgoing_interaction_has_type_field() {
        let msg = OutgoingMessage::Interaction(OutgoingInteraction {
            id: "id1".into(),
            session_id: "sess-1".into(),
            device: None,
            app_version: "1.2.3".into(),
            platform: Platform::Android,
            channel: Channel::Canary,
            timestamp: 42,
            event_name: "app_launch".into(),
            properties: Some(Properties(HashMap::from([
                ("tab".to_string(), PropertyValue::String("map".into())),
                ("count".to_string(), PropertyValue::Number(3.into())),
                ("active".to_string(), PropertyValue::Bool(true)),
                ("note".to_string(), PropertyValue::Null),
            ]))),
        });

        let json = serde_json::to_value(&msg).unwrap();
        assert_eq!(json["type"], "interaction");
        assert_eq!(json["event_name"], "app_launch");
        assert!(json["device"].is_null());
        assert_eq!(json["app_version"], "1.2.3");
        assert_eq!(json["platform"], "android");
        assert_eq!(json["channel"], "canary");
        assert_eq!(json["properties"]["tab"], "map");
        assert_eq!(json["properties"]["count"], 3);
        assert_eq!(json["properties"]["active"], true);
        assert!(json["properties"]["note"].is_null());
    }

    #[test]
    fn property_value_rejects_nested_structures() {
        assert!(serde_json::from_str::<Properties>(r#"{"tab":"map","count":1}"#).is_ok());
        assert!(serde_json::from_str::<Properties>(r#"{"nested":{"a":1}}"#).is_err());
        assert!(serde_json::from_str::<Properties>(r#"{"list":[1,2]}"#).is_err());
    }

    #[test]
    fn outgoing_location_has_type_field() {
        let msg = OutgoingMessage::LocationUpdate(OutgoingLocation {
            id: "id1".into(),
            session_id: "sess-1".into(),
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
        assert_eq!(json["coords"]["speed"], 3.0);
    }
}
