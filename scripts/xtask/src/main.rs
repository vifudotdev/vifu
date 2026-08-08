use std::env;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

const HELP: &str = "Run Vifu with freshly built Console assets.\n\nUsage: cargo vifu [CARGO_RUN_OPTIONS] [-- VIFU_OPTIONS]\n\nExamples:\n  cargo vifu\n  cargo vifu --release\n  cargo vifu -- -c server.address=https://192.0.2.10:6790";

fn main() -> ExitCode {
    let arguments = env::args_os().skip(1).collect::<Vec<_>>();
    if arguments.len() == 1 && matches!(arguments[0].to_str(), Some("-h" | "--help")) {
        println!("{HELP}");
        return ExitCode::SUCCESS;
    }

    match run_vifu(&arguments) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("cargo vifu failed: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run_vifu(arguments: &[OsString]) -> Result<(), String> {
    let repository_root = repository_root();
    run(&repository_root, "bun", ["run", "build:console"])?;

    let cargo = env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo"));
    run(&repository_root, cargo, cargo_run_arguments(arguments))
}

fn cargo_run_arguments(arguments: &[OsString]) -> Vec<OsString> {
    let mut cargo_arguments = vec![
        OsString::from("run"),
        OsString::from("--package"),
        OsString::from("vifu"),
    ];
    cargo_arguments.extend_from_slice(arguments);
    cargo_arguments
}

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")))
}

fn run<I, S>(
    working_directory: &Path,
    program: impl AsRef<OsStr>,
    arguments: I,
) -> Result<(), String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let program = program.as_ref();
    let arguments = arguments.into_iter().collect::<Vec<_>>();
    let rendered_arguments = arguments
        .iter()
        .map(|argument| argument.as_ref().to_string_lossy())
        .collect::<Vec<_>>()
        .join(" ");
    eprintln!("$ {} {rendered_arguments}", program.to_string_lossy());

    let status = Command::new(program)
        .args(arguments.iter().map(AsRef::as_ref))
        .current_dir(working_directory)
        .status()
        .map_err(|error| format!("could not start {}: {error}", program.to_string_lossy()))?;

    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "{} exited with {status}",
            program.to_string_lossy()
        ))
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use super::{cargo_run_arguments, repository_root};

    #[test]
    fn cargo_run_arguments_should_forward_release_options() {
        let arguments = [OsString::from("--release"), OsString::from("--locked")];
        let result = cargo_run_arguments(&arguments);

        assert_eq!(
            result,
            ["run", "--package", "vifu", "--release", "--locked"].map(OsString::from)
        );
    }

    #[test]
    fn cargo_run_arguments_should_forward_vifu_options_after_separator() {
        let arguments = [
            OsString::from("--"),
            OsString::from("-c"),
            OsString::from("server.address=https://192.0.2.10:6790"),
        ];
        let result = cargo_run_arguments(&arguments);

        assert_eq!(
            result,
            [
                "run",
                "--package",
                "vifu",
                "--",
                "-c",
                "server.address=https://192.0.2.10:6790",
            ]
            .map(OsString::from)
        );
    }

    #[test]
    fn repository_root_should_contain_workspace_manifest() {
        assert!(repository_root().join("Cargo.toml").is_file());
    }
}
