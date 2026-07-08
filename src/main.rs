fn main() {
    match vifu::run(std::env::args()) {
        Ok(()) => {}
        Err(error) => {
            eprintln!("vifu: {error}");
            std::process::exit(1);
        }
    }
}
