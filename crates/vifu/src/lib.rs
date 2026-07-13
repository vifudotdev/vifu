pub mod agent_gateway;
pub mod cli;
pub mod config;
pub mod openclaw;
pub mod protocol;
pub mod relay;
pub mod session;

pub async fn run<I, S>(args: I) -> Result<(), String>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let options = cli::Options::parse(args)?;
    agent_gateway::execute(options).await
}
