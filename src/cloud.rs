use std::io::Read;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

const USER_AGENT: &str = concat!("vifu/", env!("CARGO_PKG_VERSION"));
const MAX_CLOUD_RESPONSE_BYTES: u64 = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloudConfig {
    pub enabled: bool,
    pub api_base_url: String,
    pub service_login_url: String,
    pub relay_register_url: String,
    pub relay_heartbeat_url: String,
    pub service_id: String,
    pub service_username: String,
    pub service_password: String,
    pub relay_id: String,
    pub relay_endpoint: String,
    pub relay_region: String,
    pub relay_capacity: u32,
    pub heartbeat_interval_seconds: u64,
    pub request_timeout_seconds: u64,
}

#[derive(Debug, Clone)]
struct ServiceToken {
    access_token: String,
    expires_at_unix: u64,
}

impl CloudConfig {
    pub fn from_env() -> Result<Self, String> {
        Self::from_lookup(|key| std::env::var(key).ok())
    }

    fn disabled() -> Self {
        Self {
            enabled: false,
            api_base_url: String::new(),
            service_login_url: String::new(),
            relay_register_url: String::new(),
            relay_heartbeat_url: String::new(),
            service_id: String::new(),
            service_username: String::new(),
            service_password: String::new(),
            relay_id: String::new(),
            relay_endpoint: String::new(),
            relay_region: "local".to_string(),
            relay_capacity: 32,
            heartbeat_interval_seconds: 30,
            request_timeout_seconds: 10,
        }
    }

    fn from_lookup(mut lookup: impl FnMut(&str) -> Option<String>) -> Result<Self, String> {
        if !truthy(lookup("VIFU_CLOUD_ENABLED").as_deref()) {
            return Ok(Self::disabled());
        }

        let api_base_url = required(&mut lookup, "VIFU_API_BASE_URL")?;
        let service_login_url = lookup("VIFU_SERVICE_LOGIN_URL")
            .map(normalize_url)
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| join_url(&api_base_url, "/v1/auth/service/login"));
        let relay_register_url = lookup("VIFU_RELAY_REGISTER_URL")
            .map(normalize_url)
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| join_url(&api_base_url, "/v1/relays/register"));
        let relay_heartbeat_url = lookup("VIFU_RELAY_HEARTBEAT_URL")
            .map(normalize_url)
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| join_url(&api_base_url, "/v1/relays/heartbeat"));

        Ok(Self {
            enabled: true,
            api_base_url: normalize_url(api_base_url),
            service_login_url,
            relay_register_url,
            relay_heartbeat_url,
            service_id: required(&mut lookup, "VIFU_SERVICE_ID")?,
            service_username: required(&mut lookup, "VIFU_SERVICE_USERNAME")?,
            service_password: required(&mut lookup, "VIFU_SERVICE_PASSWORD")?,
            relay_id: required(&mut lookup, "VIFU_RELAY_ID")?,
            relay_endpoint: required(&mut lookup, "VIFU_RELAY_ENDPOINT")?,
            relay_region: lookup("VIFU_RELAY_REGION")
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| "local".to_string()),
            relay_capacity: lookup("VIFU_RELAY_CAPACITY")
                .as_deref()
                .map(parse_u32)
                .transpose()?
                .unwrap_or(32),
            heartbeat_interval_seconds: lookup("VIFU_HEARTBEAT_INTERVAL_SECONDS")
                .as_deref()
                .map(parse_u64)
                .transpose()?
                .map(|value| value.clamp(5, 3600))
                .unwrap_or(30),
            request_timeout_seconds: lookup("VIFU_REQUEST_TIMEOUT_SECONDS")
                .as_deref()
                .map(parse_u64)
                .transpose()?
                .map(|value| value.clamp(1, 120))
                .unwrap_or(10),
        })
    }
}

pub fn start_relay_control(config: &CloudConfig) -> Result<Option<thread::JoinHandle<()>>, String> {
    if !config.enabled {
        println!("Cloud: disabled");
        return Ok(None);
    }

    let mut client = CloudClient::new(config.clone());
    let token = client.login()?;
    client.register(&token)?;
    println!(
        "Cloud: registered relay {} in {}",
        config.relay_id, config.relay_region
    );

    let thread_config = config.clone();
    let handle = thread::spawn(move || {
        let mut client = CloudClient::new(thread_config);
        let mut token = token;
        loop {
            thread::sleep(Duration::from_secs(
                client.config.heartbeat_interval_seconds,
            ));
            if token.expires_at_unix <= now_unix_seconds().saturating_add(60) {
                match client.login() {
                    Ok(next) => token = next,
                    Err(error) => {
                        eprintln!(
                            "vifu cloud: service login failed: {}",
                            sanitize_error(&error)
                        );
                        continue;
                    }
                }
            }
            if let Err(error) = client.heartbeat(&token) {
                eprintln!("vifu cloud: heartbeat failed: {}", sanitize_error(&error));
            }
        }
    });

    Ok(Some(handle))
}

struct CloudClient {
    config: CloudConfig,
    agent: ureq::Agent,
}

impl CloudClient {
    fn new(config: CloudConfig) -> Self {
        let agent = ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(config.request_timeout_seconds))
            .build();
        Self { config, agent }
    }

    fn login(&mut self) -> Result<ServiceToken, String> {
        let payload = json!({
            "serviceId": self.config.service_id,
            "username": self.config.service_username,
            "password": self.config.service_password,
        });
        let response = self.post_json(&self.config.service_login_url, payload, None)?;
        let access_token = response
            .get("accessToken")
            .and_then(Value::as_str)
            .ok_or_else(|| "service login response missing accessToken".to_string())?
            .to_string();
        let expires_in = response
            .get("expiresIn")
            .and_then(Value::as_u64)
            .unwrap_or(3600)
            .max(60);

        Ok(ServiceToken {
            access_token,
            expires_at_unix: now_unix_seconds().saturating_add(expires_in),
        })
    }

    fn register(&mut self, token: &ServiceToken) -> Result<(), String> {
        let payload = self.relay_payload("online");
        self.post_json(
            &self.config.relay_register_url,
            payload,
            Some(&token.access_token),
        )?;
        Ok(())
    }

    fn heartbeat(&mut self, token: &ServiceToken) -> Result<(), String> {
        let payload = self.relay_payload("online");
        self.post_json(
            &self.config.relay_heartbeat_url,
            payload,
            Some(&token.access_token),
        )?;
        Ok(())
    }

    fn relay_payload(&self, status: &str) -> Value {
        json!({
            "relayId": self.config.relay_id,
            "endpoint": self.config.relay_endpoint,
            "region": self.config.relay_region,
            "capacity": self.config.relay_capacity,
            "status": status,
            "version": env!("CARGO_PKG_VERSION"),
        })
    }

    fn post_json(
        &self,
        url: &str,
        payload: Value,
        bearer_token: Option<&str>,
    ) -> Result<Value, String> {
        let mut request = self
            .agent
            .post(url)
            .set("Content-Type", "application/json")
            .set("Accept", "application/json")
            .set("User-Agent", USER_AGENT)
            .set("X-Request-ID", &request_id());

        if let Some(token) = bearer_token {
            request = request.set("Authorization", &format!("Bearer {token}"));
        }

        match request.send_json(payload) {
            Ok(response) => read_json_response(response),
            Err(ureq::Error::Status(status, response)) => {
                let body = read_response_text(response).unwrap_or_else(|_| String::new());
                Err(format!(
                    "request failed with status {status}: {}",
                    sanitize_error(&body)
                ))
            }
            Err(ureq::Error::Transport(error)) => Err(error.to_string()),
        }
    }
}

fn read_json_response(response: ureq::Response) -> Result<Value, String> {
    let body = read_response_text(response)?;
    if body.trim().is_empty() {
        return Ok(json!({}));
    }
    serde_json::from_str(&body).map_err(|error| format!("invalid json response: {error}"))
}

fn read_response_text(response: ureq::Response) -> Result<String, String> {
    let mut reader = response.into_reader().take(MAX_CLOUD_RESPONSE_BYTES);
    let mut body = String::new();
    reader
        .read_to_string(&mut body)
        .map_err(|error| error.to_string())?;
    Ok(body)
}

fn required(lookup: &mut impl FnMut(&str) -> Option<String>, key: &str) -> Result<String, String> {
    lookup(key)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("{key} is required when VIFU_CLOUD_ENABLED=1"))
}

fn truthy(value: Option<&str>) -> bool {
    matches!(
        value.map(|value| value.trim().to_ascii_lowercase()),
        Some(value) if matches!(value.as_str(), "1" | "true" | "yes" | "on")
    )
}

fn normalize_url(value: String) -> String {
    value.trim().trim_end_matches('/').to_string()
}

fn join_url(base: &str, path: &str) -> String {
    format!("{}{}", normalize_url(base.to_string()), path)
}

fn parse_u32(value: &str) -> Result<u32, String> {
    value
        .trim()
        .parse::<u32>()
        .map_err(|_| format!("invalid integer value: {value}"))
}

fn parse_u64(value: &str) -> Result<u64, String> {
    value
        .trim()
        .parse::<u64>()
        .map_err(|_| format!("invalid integer value: {value}"))
}

fn request_id() -> String {
    format!("vifu-{}-{}", now_unix_seconds(), std::process::id())
}

fn now_unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn sanitize_error(value: &str) -> String {
    let mut output = String::with_capacity(value.len().min(512));
    for ch in value.chars().take(512) {
        if ch.is_control() && ch != '\n' {
            output.push(' ');
        } else {
            output.push(ch);
        }
    }
    if output.is_empty() {
        "unknown error".to_string()
    } else {
        output
    }
}

#[cfg(test)]
mod tests {
    use super::CloudConfig;

    #[test]
    fn cloud_config_is_disabled_by_default() {
        let config = CloudConfig::from_lookup(|_| None).unwrap();
        assert!(!config.enabled);
    }

    #[test]
    fn cloud_config_requires_service_credentials_when_enabled() {
        let error = CloudConfig::from_lookup(|key| match key {
            "VIFU_CLOUD_ENABLED" => Some("1".to_string()),
            _ => None,
        })
        .unwrap_err();
        assert!(error.contains("VIFU_API_BASE_URL"));
    }

    #[test]
    fn cloud_config_builds_default_control_urls() {
        let config = CloudConfig::from_lookup(|key| match key {
            "VIFU_CLOUD_ENABLED" => Some("true".to_string()),
            "VIFU_API_BASE_URL" => Some("https://api.example.test/".to_string()),
            "VIFU_SERVICE_ID" => Some("vifu-relay-dev".to_string()),
            "VIFU_SERVICE_USERNAME" => Some("service@example.test".to_string()),
            "VIFU_SERVICE_PASSWORD" => Some("test-password".to_string()),
            "VIFU_RELAY_ID" => Some("relay-dev".to_string()),
            "VIFU_RELAY_ENDPOINT" => Some("wss://relay.example.test".to_string()),
            _ => None,
        })
        .unwrap();

        assert!(config.enabled);
        assert_eq!(
            config.service_login_url,
            "https://api.example.test/v1/auth/service/login"
        );
        assert_eq!(
            config.relay_register_url,
            "https://api.example.test/v1/relays/register"
        );
        assert_eq!(
            config.relay_heartbeat_url,
            "https://api.example.test/v1/relays/heartbeat"
        );
    }
}
