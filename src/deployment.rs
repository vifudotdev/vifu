use std::thread;
use std::time::Duration;

use serde_json::{json, Value};
use std::io::Read;

const DEFAULT_RELAY_REGION: &str = "local";
const DEFAULT_RELAY_CAPACITY: u32 = 32;
const DEFAULT_HEARTBEAT_INTERVAL_SECONDS: u64 = 30;
const DEFAULT_REQUEST_TIMEOUT_SECONDS: u64 = 10;

#[derive(Debug, Clone)]
pub struct DeploymentConfig {
    pub target: DeploymentTarget,
    api_base_url: String,
    deploy_key: String,
    deployment_name: String,
    deployment_endpoint: String,
    relay_region: String,
    relay_capacity: u32,
    heartbeat_interval_seconds: u64,
    request_timeout_seconds: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeploymentTarget {
    Local,
    SelfHost,
    Prod(String),
}

impl DeploymentTarget {
    pub fn label(&self) -> String {
        match self {
            Self::Local => "local".to_string(),
            Self::SelfHost => "self-host".to_string(),
            Self::Prod(name) => format!("prod:{name}"),
        }
    }

    fn requires_api(&self) -> bool {
        matches!(self, Self::Prod(_))
    }
}

impl DeploymentConfig {
    pub fn from_env() -> Result<Self, String> {
        Self::from_lookup(|key| std::env::var(key).ok())
    }

    fn from_lookup<F>(mut lookup: F) -> Result<Self, String>
    where
        F: FnMut(&str) -> Option<String>,
    {
        let target = parse_deployment(lookup("VIFU_DEPLOYMENT").as_deref())?;

        if !target.requires_api() {
            return Ok(Self::local(target));
        }

        let deployment_name = match &target {
            DeploymentTarget::Prod(name) => name.clone(),
            DeploymentTarget::Local | DeploymentTarget::SelfHost => String::new(),
        };

        Ok(Self {
            target,
            api_base_url: trim_trailing_slash(required(&mut lookup, "VIFU_DEPLOYMENT_API_URL")?),
            deploy_key: required(&mut lookup, "VIFU_DEPLOY_KEY")?,
            deployment_name,
            deployment_endpoint: required(&mut lookup, "VIFU_DEPLOYMENT_ENDPOINT")?,
            relay_region: lookup("VIFU_DEPLOYMENT_REGION")
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| DEFAULT_RELAY_REGION.to_string()),
            relay_capacity: lookup("VIFU_DEPLOYMENT_CAPACITY")
                .map(|value| parse_u32(&value, "VIFU_DEPLOYMENT_CAPACITY"))
                .transpose()?
                .unwrap_or(DEFAULT_RELAY_CAPACITY),
            heartbeat_interval_seconds: lookup("VIFU_DEPLOYMENT_HEARTBEAT_INTERVAL_SECONDS")
                .map(|value| parse_u64(&value, "VIFU_DEPLOYMENT_HEARTBEAT_INTERVAL_SECONDS"))
                .transpose()?
                .unwrap_or(DEFAULT_HEARTBEAT_INTERVAL_SECONDS),
            request_timeout_seconds: lookup("VIFU_DEPLOYMENT_REQUEST_TIMEOUT_SECONDS")
                .map(|value| parse_u64(&value, "VIFU_DEPLOYMENT_REQUEST_TIMEOUT_SECONDS"))
                .transpose()?
                .unwrap_or(DEFAULT_REQUEST_TIMEOUT_SECONDS),
        })
    }

    fn local(target: DeploymentTarget) -> Self {
        Self {
            target,
            api_base_url: String::new(),
            deploy_key: String::new(),
            deployment_name: String::new(),
            deployment_endpoint: String::new(),
            relay_region: DEFAULT_RELAY_REGION.to_string(),
            relay_capacity: DEFAULT_RELAY_CAPACITY,
            heartbeat_interval_seconds: DEFAULT_HEARTBEAT_INTERVAL_SECONDS,
            request_timeout_seconds: DEFAULT_REQUEST_TIMEOUT_SECONDS,
        }
    }

    fn register_url(&self) -> String {
        format!("{}/v1/relays/register", self.api_base_url)
    }

    fn heartbeat_url(&self) -> String {
        format!("{}/v1/relays/heartbeat", self.api_base_url)
    }

    fn payload(&self) -> Value {
        json!({
            "relayId": self.deployment_name,
            "endpoint": self.deployment_endpoint,
            "region": self.relay_region,
            "capacity": self.relay_capacity,
            "status": "online",
            "version": env!("CARGO_PKG_VERSION"),
        })
    }
}

pub fn start(config: &DeploymentConfig) -> Result<Option<thread::JoinHandle<()>>, String> {
    if !config.target.requires_api() {
        return Ok(None);
    }

    let client = DeploymentClient::new(config.clone());
    client.register()?;
    println!("Deployment: connected");

    let thread_config = config.clone();
    let handle = thread::spawn(move || {
        let client = DeploymentClient::new(thread_config.clone());
        loop {
            thread::sleep(Duration::from_secs(
                thread_config.heartbeat_interval_seconds,
            ));
            if let Err(error) = client.heartbeat() {
                eprintln!(
                    "vifu deployment: heartbeat failed: {}",
                    sanitize_error(&error)
                );
            }
        }
    });

    Ok(Some(handle))
}

struct DeploymentClient {
    config: DeploymentConfig,
    agent: ureq::Agent,
}

impl DeploymentClient {
    fn new(config: DeploymentConfig) -> Self {
        let agent = ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(config.request_timeout_seconds))
            .build();
        Self { config, agent }
    }

    fn register(&self) -> Result<(), String> {
        self.post_json(&self.config.register_url(), self.config.payload())
    }

    fn heartbeat(&self) -> Result<(), String> {
        self.post_json(&self.config.heartbeat_url(), self.config.payload())
    }

    fn post_json(&self, url: &str, payload: Value) -> Result<(), String> {
        let response = self
            .agent
            .post(url)
            .set("Content-Type", "application/json")
            .set("User-Agent", concat!("vifu/", env!("CARGO_PKG_VERSION")))
            .set(
                "Authorization",
                &format!("Bearer {}", self.config.deploy_key),
            )
            .send_json(payload);

        match response {
            Ok(response) if response.status() >= 200 && response.status() < 300 => Ok(()),
            Ok(response) => Err(format!("request failed with status {}", response.status())),
            Err(ureq::Error::Status(status, response)) => {
                let body = read_response_text(response).unwrap_or_default();
                if body.is_empty() {
                    Err(format!("request failed with status {status}"))
                } else {
                    Err(format!(
                        "request failed with status {status}: {}",
                        sanitize_error(&body)
                    ))
                }
            }
            Err(ureq::Error::Transport(error)) => Err(error.to_string()),
        }
    }
}

fn required<F>(lookup: &mut F, key: &str) -> Result<String, String>
where
    F: FnMut(&str) -> Option<String>,
{
    lookup(key)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("{key} is required when VIFU_DEPLOYMENT=prod:<name>"))
}

fn parse_deployment(value: Option<&str>) -> Result<DeploymentTarget, String> {
    let value = value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("local");

    if value == "local" {
        return Ok(DeploymentTarget::Local);
    }
    if value == "self-host" {
        return Ok(DeploymentTarget::SelfHost);
    }

    let Some(name) = value.strip_prefix("prod:") else {
        return Err("VIFU_DEPLOYMENT must be local, self-host, or prod:<name>".to_string());
    };

    let name = deployment_text(name.trim().to_string(), "VIFU_DEPLOYMENT")?;
    if name.is_empty() {
        return Err("VIFU_DEPLOYMENT prod target requires a deployment name".to_string());
    }
    Ok(DeploymentTarget::Prod(name))
}

fn deployment_text(value: String, field: &str) -> Result<String, String> {
    if value.len() > 128
        || !value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
    {
        return Err(format!("{field} contains unsupported characters"));
    }
    Ok(value)
}

fn trim_trailing_slash(value: String) -> String {
    value.trim_end_matches('/').to_string()
}

fn parse_u32(value: &str, field: &str) -> Result<u32, String> {
    value
        .trim()
        .parse::<u32>()
        .map_err(|_| format!("{field} must be a positive integer"))
        .and_then(|value| {
            if value == 0 {
                Err(format!("{field} must be greater than zero"))
            } else {
                Ok(value)
            }
        })
}

fn parse_u64(value: &str, field: &str) -> Result<u64, String> {
    value
        .trim()
        .parse::<u64>()
        .map_err(|_| format!("{field} must be a positive integer"))
        .and_then(|value| {
            if value == 0 {
                Err(format!("{field} must be greater than zero"))
            } else {
                Ok(value)
            }
        })
}

fn read_response_text(response: ureq::Response) -> Result<String, String> {
    let mut reader = response.into_reader().take(4096);
    let mut body = String::new();
    std::io::Read::read_to_string(&mut reader, &mut body).map_err(|error| error.to_string())?;
    Ok(body)
}

fn sanitize_error(value: &str) -> String {
    let mut output = String::with_capacity(value.len().min(256));
    for ch in value.chars().take(256) {
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
    use super::{DeploymentConfig, DeploymentTarget};

    #[test]
    fn defaults_to_local_deployment() {
        let config = DeploymentConfig::from_lookup(|_| None).unwrap();
        assert_eq!(config.target, DeploymentTarget::Local);
    }

    #[test]
    fn parses_self_host_deployment() {
        let config = DeploymentConfig::from_lookup(|key| match key {
            "VIFU_DEPLOYMENT" => Some("self-host".to_string()),
            _ => None,
        })
        .unwrap();
        assert_eq!(config.target, DeploymentTarget::SelfHost);
    }

    #[test]
    fn requires_prod_deployment_fields() {
        let error = DeploymentConfig::from_lookup(|key| match key {
            "VIFU_DEPLOYMENT" => Some("prod:relay-local".to_string()),
            _ => None,
        })
        .unwrap_err();
        assert!(error.contains("VIFU_DEPLOYMENT_API_URL"));
    }

    #[test]
    fn builds_prod_deployment_register_and_heartbeat_urls() {
        let config = DeploymentConfig::from_lookup(|key| match key {
            "VIFU_DEPLOYMENT" => Some("prod:relay-local".to_string()),
            "VIFU_DEPLOYMENT_API_URL" => Some("https://api.example.test/".to_string()),
            "VIFU_DEPLOY_KEY" => Some("vifu_rk_test.secret".to_string()),
            "VIFU_DEPLOYMENT_ENDPOINT" => Some("tcp://127.0.0.1:48989".to_string()),
            _ => None,
        })
        .unwrap();

        assert_eq!(
            config.register_url(),
            "https://api.example.test/v1/relays/register"
        );
        assert_eq!(
            config.heartbeat_url(),
            "https://api.example.test/v1/relays/heartbeat"
        );
        assert_eq!(config.relay_region, "local");
        assert_eq!(config.relay_capacity, 32);
    }

    #[test]
    fn rejects_unknown_deployment_target() {
        let error = DeploymentConfig::from_lookup(|key| match key {
            "VIFU_DEPLOYMENT" => Some("remote:relay-local".to_string()),
            _ => None,
        })
        .unwrap_err();
        assert!(error.contains("VIFU_DEPLOYMENT must be local, self-host, or prod:<name>"));
    }
}
