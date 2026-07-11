#[tokio::main]
async fn main() {
    match vifu::run(std::env::args()).await {
        Ok(()) => {}
        Err(error) => {
            eprintln!("vifu: {error}");
            std::process::exit(1);
        }
    }
}
