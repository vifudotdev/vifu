//! The public Rust SDK for embedding and operating Vifu.

#[cfg(feature = "binary")]
pub mod cli;
#[cfg(feature = "gateway")]
pub mod gateway;
#[cfg(feature = "binary")]
mod launcher;
#[cfg(feature = "binary")]
pub mod runtime_config;

#[cfg(feature = "server")]
pub use vifu_server as server;

#[cfg(feature = "runtime")]
pub mod runtime {
    //! Portable, headless runtime primitives.

    pub use vifu_runtime::*;
}

#[cfg(feature = "binary")]
pub async fn run<I, S>(args: I) -> Result<(), String>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let options = cli::Options::parse(args)?;
    launcher::execute(options).await
}
