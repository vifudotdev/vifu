mod benchmark;
mod cli;
mod gateway;
mod launcher;
#[cfg(feature = "local-llama")]
mod local_models;
mod monitor;
mod runtime_config;
mod tui;

#[tokio::main]
async fn main() {
    let result = match cli::Options::parse(std::env::args()) {
        Ok(options) => launcher::execute(options).await,
        Err(error) => Err(error),
    };
    match result {
        Ok(()) => {}
        Err(error) => {
            eprintln!("vifu: {error}");
            std::process::exit(1);
        }
    }
}
