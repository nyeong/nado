use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use clap::Parser;

#[derive(Parser, Debug)]
#[command(
    name = "nado",
    version,
    about = "Local differential tester for algorithm solutions"
)]
pub struct Cli {
    /// Optional path to nado TOML config (defaults to ./nado.toml)
    pub config: Option<PathBuf>,
}

pub fn resolve_config_path(cli_config: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(config_path) = cli_config {
        if !config_path.exists() {
            bail!("config not found: {}", config_path.display());
        }
        return Ok(config_path);
    }

    let cwd = std::env::current_dir().context("failed to get current directory")?;
    let config_path = cwd.join("nado.toml");
    if config_path.exists() {
        return Ok(config_path);
    }

    bail!(
        "nado.toml not found in current directory: {}",
        cwd.display()
    );
}
