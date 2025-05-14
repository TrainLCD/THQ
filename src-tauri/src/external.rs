use chrono::{DateTime, Utc};
use jsonwebtoken::{encode, EncodingKey, Header};
use once_cell::sync::Lazy;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::time::{Duration, Instant};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::RwLock;

static TOKEN_CACHE: Lazy<RwLock<Option<CachedToken>>> = Lazy::new(|| RwLock::new(None));

struct CachedToken {
    token: String,
    expires_at: Instant,
}
#[derive(Deserialize)]
#[allow(dead_code)]
struct ServiceAccount {
    r#type: String,
    project_id: String,
    private_key_id: String,
    private_key: String,
    client_email: String,
    client_id: String,
    auth_uri: String,
    token_uri: String,
    auth_provider_x509_cert_url: String,
    client_x509_cert_url: String,
}

#[derive(Serialize)]
struct Claims<'a> {
    iss: &'a str,
    scope: &'a str,
    aud: &'a str,
    exp: usize,
    iat: usize,
}

#[derive(Deserialize)]
pub struct LocationValue {
    pub latitude: f64,
    pub longitude: f64,
    pub accuracy: Option<f64>,
    pub speed: f64,
    pub state: Option<String>,
    pub device: String,
    pub timestamp: u64,
}

#[derive(Deserialize)]
pub struct LogValue {
    pub level: String,
    pub message: String,
    pub device: String,
    pub timestamp: u64,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct FirebaseError {
    error: FirebaseErrorBody,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct FirebaseErrorBody {
    code: u16,
    message: String,
    status: String,
}

async fn get_access_token() -> Result<String, Box<dyn std::error::Error>> {
    let json_path = std::env::var("GOOGLE_APPLICATION_CREDENTIALS")
        .unwrap_or_else(|_| "service-account.json".to_string());
    let json = std::fs::read_to_string(json_path)?;
    let service_account: ServiceAccount = serde_json::from_str(&json)?;

    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() as usize;
    let claims = Claims {
        iss: &service_account.client_email,
        scope: "https://www.googleapis.com/auth/cloud-platform",
        aud: "https://oauth2.googleapis.com/token",
        exp: now + 3600,
        iat: now,
    };

    let jwt = encode(
        &Header::new(jsonwebtoken::Algorithm::RS256),
        &claims,
        &EncodingKey::from_rsa_pem(service_account.private_key.as_bytes())?,
    )?;

    let client = reqwest::Client::new();
    let params = [
        ("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer"),
        ("assertion", &jwt),
    ];

    let resp: serde_json::Value = client
        .post("https://oauth2.googleapis.com/token")
        .form(&params)
        .send()
        .await?
        .json()
        .await?;

    Ok(resp["access_token"].as_str().unwrap().to_string())
}

async fn get_cached_access_token() -> Result<String, Box<dyn std::error::Error>> {
    {
        let cache = TOKEN_CACHE.read().await;
        if let Some(cached) = &*cache {
            if cached.expires_at > Instant::now() {
                return Ok(cached.token.clone());
            }
        }
    }

    let new_token = get_access_token().await?;
    let mut cache = TOKEN_CACHE.write().await;
    *cache = Some(CachedToken {
        token: new_token.clone(),
        expires_at: Instant::now() + Duration::from_secs(3500), // 安全のため少し短めに
    });

    Ok(new_token)
}

pub async fn send_location_to_firebase(
    location: &LocationValue,
) -> Result<(), Box<dyn std::error::Error>> {
    let project_id = std::env::var("FIREBASE_PROJECT_ID")
        .expect("Could not found FIREBASE_PROJECT_ID in environment variable.");
    let collection = "telemetryLocations";

    let client = Client::new();

    let access_token = get_access_token().await?; // Google OAuth 2.0トークン

    let url = format!(
        "https://firestore.googleapis.com/v1/projects/{}/databases/(default)/documents/{}",
        project_id, collection
    );

    let timestamp_str: String = DateTime::<Utc>::from(
        std::time::UNIX_EPOCH + std::time::Duration::from_millis(location.timestamp),
    )
    .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);

    let payload = json!({
        "fields": {
            "latitude": { "doubleValue": location.latitude.to_string() },
            "longitude": { "doubleValue": location.longitude.to_string() },
            "accuracy": { "doubleValue": location.accuracy.unwrap_or(0.0).to_string() },
            "speed": { "doubleValue": location.speed.to_string() },
            "device": {"stringValue":location.device},
            "state": {"stringValue":location.state},
            "timestamp": { "timestampValue": timestamp_str }
        }
    });

    let res = client
        .post(url)
        .bearer_auth(access_token)
        .json(&payload)
        .send()
        .await?;

    let status = res.status();
    if !status.is_success() {
        let text = res.text().await?;
        log::error!("Firestore error {}: {}", status, text);
    }

    Ok(())
}

pub async fn send_log_to_firebase(log_value: &LogValue) -> Result<(), Box<dyn std::error::Error>> {
    let project_id = std::env::var("FIREBASE_PROJECT_ID")
        .expect("Could not found FIREBASE_PROJECT_ID in environment variable.");
    let collection = "telemetryLogs";

    let client = Client::new();

    let access_token = get_cached_access_token().await?; // Google OAuth 2.0トークン

    let url = format!(
        "https://firestore.googleapis.com/v1/projects/{}/databases/(default)/documents/{}",
        project_id, collection
    );

    let timestamp_str: String = DateTime::<Utc>::from(
        std::time::UNIX_EPOCH + std::time::Duration::from_millis(log_value.timestamp),
    )
    .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);

    let payload = json!({
        "fields": {
            "level": { "stringValue": log_value.level },
            "message": { "stringValue": log_value.message },
            "device": {"stringValue":log_value.device},
            "timestamp": { "timestampValue": timestamp_str }
        }
    });

    let res = client
        .post(url)
        .bearer_auth(access_token)
        .json(&payload)
        .send()
        .await?;

    let status = res.status();
    if !status.is_success() {
        let text = res.text().await?;
        log::error!("Firestore error {}: {}", status, text);
    }

    Ok(())
}
