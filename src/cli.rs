use crate::config::{DEFAULT_OPENCLAW_URL, DEFAULT_RELAY_LISTEN_ADDR};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Connect,
    Server,
    Status,
    Doctor,
    Logout,
    Reset,
    Help,
    Version,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    pub command: Command,
    pub openclaw_url: String,
    pub relay_addr: Option<String>,
    pub listen_addr: String,
}

impl Options {
    pub fn parse<I, S>(args: I) -> Result<Self, String>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let mut command = Command::Connect;
        let mut openclaw_url =
            std::env::var("VIFU_OPENCLAW_URL").unwrap_or_else(|_| DEFAULT_OPENCLAW_URL.to_string());
        let mut relay_addr = std::env::var("VIFU_RELAY_ADDR")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        let mut listen_addr = std::env::var("VIFU_LISTEN_ADDR")
            .unwrap_or_else(|_| DEFAULT_RELAY_LISTEN_ADDR.to_string());

        let mut args = args.into_iter().map(Into::into);
        let _program_name = args.next();

        while let Some(arg) = args.next() {
            match arg.as_str() {
                "-h" | "--help" => command = Command::Help,
                "-V" | "--version" => command = Command::Version,
                "--status" => command = Command::Status,
                "--doctor" => command = Command::Doctor,
                "--logout" => command = Command::Logout,
                "--reset" => command = Command::Reset,
                "--openclaw-url" => {
                    openclaw_url = args
                        .next()
                        .ok_or_else(|| "--openclaw-url requires a value".to_string())?;
                }
                "--relay" => {
                    relay_addr = Some(
                        args.next()
                            .ok_or_else(|| "--relay requires a value".to_string())?,
                    );
                }
                "--listen" => {
                    listen_addr = args
                        .next()
                        .ok_or_else(|| "--listen requires a value".to_string())?;
                }
                value if value.starts_with('-') => {
                    return Err(format!("unknown option: {value}"));
                }
                "server" => {
                    if command != Command::Connect {
                        return Err("only one vifu command can be used at a time".to_string());
                    }
                    command = Command::Server;
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
            openclaw_url,
            relay_addr,
            listen_addr,
        })
    }
}

pub fn help_text() -> &'static str {
    "vifu

Connect local AI agents to Vifu.

Usage:
  vifu                 Start the local connector
  vifu server          Start a Vifu relay server
  vifu --status        Show local connector status
  vifu --doctor        Diagnose local setup
  vifu --logout        Remove local Vifu session state
  vifu --reset         Remove all local Vifu state

Options:
  --openclaw-url URL   Local OpenClaw Gateway URL
  --relay ADDR         Relay address for the local connector
  --listen ADDR        Listen address for `vifu server`
  -h, --help           Show help
  -V, --version        Show version
"
}

#[cfg(test)]
mod tests {
    use super::{Command, Options};

    #[test]
    fn defaults_to_connect() {
        let options = Options::parse(["vifu"]).unwrap();
        assert_eq!(options.command, Command::Connect);
    }

    #[test]
    fn parses_status_flag() {
        let options = Options::parse(["vifu", "--status"]).unwrap();
        assert_eq!(options.command, Command::Status);
    }

    #[test]
    fn parses_server_command() {
        let options = Options::parse(["vifu", "server", "--listen", "127.0.0.1:48990"]).unwrap();
        assert_eq!(options.command, Command::Server);
        assert_eq!(options.listen_addr, "127.0.0.1:48990");
    }

    #[test]
    fn parses_relay_address() {
        let options = Options::parse(["vifu", "--relay", "127.0.0.1:48989"]).unwrap();
        assert_eq!(options.relay_addr.as_deref(), Some("127.0.0.1:48989"));
    }

    #[test]
    fn rejects_unknown_positional_arguments() {
        let error = Options::parse(["vifu", "endpoint", "add"]).unwrap_err();
        assert!(error.contains("unexpected argument"));
    }
}
