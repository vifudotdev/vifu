mod cli;
mod gateway;
mod launcher;
mod runtime_config;

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
