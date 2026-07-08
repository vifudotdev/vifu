use std::path::PathBuf;

use crate::cloud::CloudConfig;

pub const DEFAULT_OPENCLAW_URL: &str = "http://127.0.0.1:18789";
pub const DEFAULT_RELAY_LISTEN_ADDR: &str = "127.0.0.1:48989";

#[derive(Debug, Clone)]
pub struct Config {
    pub home_dir: PathBuf,
    pub openclaw_url: String,
    pub relay_addr: Option<String>,
    pub listen_addr: String,
    pub cloud: CloudConfig,
}

impl Config {
    pub fn load(
        openclaw_url: String,
        relay_addr: Option<String>,
        listen_addr: String,
    ) -> Result<Self, String> {
        let home_dir = match std::env::var_os("VIFU_HOME") {
            Some(value) if !value.is_empty() => PathBuf::from(value),
            _ => default_home_dir()?,
        };

        Ok(Self {
            home_dir,
            openclaw_url,
            relay_addr,
            listen_addr,
            cloud: CloudConfig::from_env()?,
        })
    }

    pub fn session_file(&self) -> PathBuf {
        self.home_dir.join("session")
    }
}

fn default_home_dir() -> Result<PathBuf, String> {
    let home = std::env::var_os("HOME").ok_or_else(|| {
        "HOME is not set. Set VIFU_HOME to choose where vifu stores local state.".to_string()
    })?;
    Ok(PathBuf::from(home).join(".vifu"))
}

#[cfg(test)]
mod tests {
    use super::DEFAULT_OPENCLAW_URL;
    use crate::cli::Options;

    #[test]
    fn default_openclaw_url_is_loopback() {
        let options = Options::parse(["vifu"]).unwrap();
        assert_eq!(options.openclaw_url, DEFAULT_OPENCLAW_URL);
    }
}
