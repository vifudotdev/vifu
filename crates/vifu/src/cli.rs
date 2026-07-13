use crate::config::{DEFAULT_OPENCLAW_URL, DEFAULT_SERVER_URL};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Connect,
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
    pub server_url: String,
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
        let mut server_url =
            std::env::var("VIFU_SERVER_URL").unwrap_or_else(|_| DEFAULT_SERVER_URL.to_string());

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
                "--server-url" => {
                    server_url = args
                        .next()
                        .ok_or_else(|| "--server-url requires a value".to_string())?;
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
            openclaw_url,
            server_url,
        })
    }
}

pub fn help_text() -> &'static str {
    "vifu

Connect local OpenClaw agents to a Vifu Agent Endpoint Runtime.

Usage:
  vifu                   Start the Agent Gateway
  vifu --status          Show Agent Gateway configuration
  vifu --doctor          Diagnose local setup
  vifu --logout          Remove the resumable Agent Gateway session
  vifu --reset           Remove all local Vifu state

Options:
  --openclaw-url URL     Local OpenClaw Gateway URL
  --server-url URL       Vifu server HTTP base URL
  -h, --help             Show help
  -V, --version          Show version
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
    fn parses_server_url() {
        let options = Options::parse(["vifu", "--server-url", "http://127.0.0.1:6790"]).unwrap();
        assert_eq!(options.server_url, "http://127.0.0.1:6790");
    }

    #[test]
    fn rejects_removed_server_command() {
        let error = Options::parse(["vifu", "server"]).unwrap_err();
        assert!(error.contains("unexpected argument"));
    }
}
