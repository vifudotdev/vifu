#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Start,
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
}

impl Options {
    pub fn parse<I, S>(args: I) -> Result<Self, String>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let mut command = Command::Start;

        let mut args = args.into_iter().map(Into::into);
        let _program_name = args.next();

        for arg in args {
            match arg.as_str() {
                "-h" | "--help" => command = Command::Help,
                "-V" | "--version" => command = Command::Version,
                "--status" => command = Command::Status,
                "--doctor" => command = Command::Doctor,
                "--logout" => command = Command::Logout,
                "--reset" => command = Command::Reset,
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

        Ok(Self { command })
    }
}

pub fn help_text() -> &'static str {
    "vifu

Run a Vifu Agent Endpoint Runtime.

Usage:
  vifu                   Start the configured Server, Agent Gateway, or both
  vifu --status          Show configured runtime and Agent Gateway state
  vifu --doctor          Diagnose local setup
  vifu --logout          Remove the resumable Agent Gateway session
  vifu --reset           Replace the local Agent Gateway identity

Configuration:
  ~/.vifu/config.json    Created with local Server and Agent Gateway defaults
  ~/.vifu/providers.json Created empty; add providers in the Dashboard

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
    }

    #[test]
    fn parses_status_flag() {
        let options = Options::parse(["vifu", "--status"]).unwrap();
        assert_eq!(options.command, Command::Status);
    }

    #[test]
    fn rejects_role_commands() {
        let error = Options::parse(["vifu", "server"]).unwrap_err();
        assert!(error.contains("unexpected argument"));
    }
}
