pub mod agent_gateway;
pub mod cli;

pub use vifu_core::{config, openclaw, protocol, relay, session};

pub async fn run<I, S>(args: I) -> Result<(), String>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let options = cli::Options::parse(args)?;
    agent_gateway::execute(options).await
}
