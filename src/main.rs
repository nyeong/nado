mod cli;
mod config;
mod engine;
mod generator;
mod runner;

use anyhow::Result;
use clap::Parser;

use crate::cli::Cli;

fn main() -> Result<()> {
    let cli = Cli::parse();
    let config_path = cli::resolve_config_path(cli.config)?;

    let exit_code = engine::run(&config_path)?;
    if exit_code != 0 {
        std::process::exit(exit_code);
    }

    Ok(())
}
