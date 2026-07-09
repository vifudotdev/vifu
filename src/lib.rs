pub mod cli;
pub mod config;
pub mod connector;
pub mod deployment;
pub mod openclaw;
pub mod protocol;
pub mod relay;
pub mod session;

pub fn run<I, S>(args: I) -> Result<(), String>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let options = cli::Options::parse(args)?;
    connector::execute(options)
}
