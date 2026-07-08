pub mod cli;
pub mod config;
pub mod connector;
pub mod openclaw;
pub mod session;

pub fn run<I, S>(args: I) -> Result<(), String>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let options = cli::Options::parse(args)?;
    connector::execute(options)
}
