#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Start,
    Status,
    Doctor,
    Help,
    Version,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    pub command: Command,
    pub config_profile: Option<String>,
    pub config_overrides: Vec<String>,
}

impl Options {
    pub fn parse<I, S>(args: I) -> Result<Self, String>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let mut command = Command::Start;
        let mut config_profile = None;
        let mut config_overrides = Vec::new();

        let mut args = args.into_iter().map(Into::into);
        let _program_name = args.next();

        while let Some(arg) = args.next() {
            match arg.as_str() {
                "-h" | "--help" => command = Command::Help,
                "-V" | "--version" => command = Command::Version,
                "--status" => command = Command::Status,
                "--doctor" => command = Command::Doctor,
                "-p" | "--profile" => {
                    let value = args
                        .next()
                        .filter(|value| !value.starts_with('-'))
                        .ok_or_else(|| format!("{arg} requires a profile name"))?;
                    config_profile = Some(parse_profile_name(&value)?);
                }
                value if value.starts_with("--profile=") => {
                    config_profile = Some(parse_profile_name(&value["--profile=".len()..])?);
                }
                "-c" | "--config" => {
                    let value = args
                        .next()
                        .filter(|value| !value.starts_with('-'))
                        .ok_or_else(|| format!("{arg} requires a key=value argument"))?;
                    config_overrides.push(value);
                }
                value if value.starts_with("--config=") => {
                    config_overrides.push(value["--config=".len()..].to_string());
                }
                value if value.starts_with('-') => {
                    return Err(format!("unknown option: {value}"));
                }
                value => {
                    return Err(format!(
                        "unexpected argument: {value}. Run `vifu --help` for usage."
                    ));
                }
            }
        }

        Ok(Self {
            command,
            config_profile,
            config_overrides,
        })
    }
}

fn parse_profile_name(value: &str) -> Result<String, String> {
    if value.is_empty()
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        return Err(format!(
            "invalid --profile value {value:?}; use a plain name such as cloud"
        ));
    }
    Ok(value.to_string())
}

pub fn help_text() -> &'static str {
    "vifu

Run a Vifu Agent Endpoint Runtime.

Usage:
  vifu                   Start the configured Server, Agent Gateway, or both
  vifu --status          Show configured runtime and Agent Gateway state
  vifu --doctor          Diagnose local setup
Configuration:
  ~/.vifu/config.json    Created with local Server and Agent Gateway defaults
  ~/.vifu/providers.json Created empty; add providers in the Dashboard
  -p, --profile <name>   Use ~/.vifu/<name>.config.json instead of the base config
  -c, --config <key=value>
                         Override a configuration value for this run. Use a
                         dotted path such as gateway.serverUrl. Values are
                         parsed as JSON, or used as strings when unquoted.

Options:
  -h, --help             Show help
  -V, --version          Show version
"
}

#[cfg(test)]
mod tests {
    use super::{Command, Options};

    #[test]
    fn defaults_to_start() {
        let options = Options::parse(["vifu"]).unwrap();
        assert_eq!(options.command, Command::Start);
        assert!(options.config_profile.is_none());
        assert!(options.config_overrides.is_empty());
    }

    #[test]
    fn parses_status_flag() {
        let options = Options::parse(["vifu", "--status"]).unwrap();
        assert_eq!(options.command, Command::Status);
    }

    #[test]
    fn collects_repeated_config_overrides() {
        let options = Options::parse([
            "vifu",
            "-c",
            "gateway.serverUrl=https://runtime.example.com/v1?region=jp",
            "--config=server.listen=127.0.0.1:6795",
        ])
        .unwrap();

        assert_eq!(
            options.config_overrides,
            vec![
                "gateway.serverUrl=https://runtime.example.com/v1?region=jp",
                "server.listen=127.0.0.1:6795",
            ]
        );
    }

    #[test]
    fn parses_config_profile() {
        let options = Options::parse(["vifu", "-p", "cloud-dev"]).unwrap();

        assert_eq!(options.config_profile.as_deref(), Some("cloud-dev"));
    }

    #[test]
    fn parses_long_config_profile() {
        let options = Options::parse(["vifu", "--profile=self_hosted"]).unwrap();

        assert_eq!(options.config_profile.as_deref(), Some("self_hosted"));
    }

    #[test]
    fn rejects_config_profile_paths() {
        let error = Options::parse(["vifu", "--profile", "../cloud"]).unwrap_err();

        assert!(error.contains("use a plain name"));
    }

    #[test]
    fn rejects_config_flag_without_a_value() {
        let error = Options::parse(["vifu", "-c", "--status"]).unwrap_err();

        assert_eq!(error, "-c requires a key=value argument");
    }

    #[test]
    fn rejects_role_commands() {
        let error = Options::parse(["vifu", "server"]).unwrap_err();
        assert!(error.contains("unexpected argument"));
    }
}
