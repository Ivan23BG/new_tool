mod args;
mod commands;

use clap::Parser;
use std::error::Error;

/// Parse command-line arguments and dispatch the selected command.
pub fn run() -> Result<(), Box<dyn Error + Send + Sync>> {
    let cli = args::Cli::parse();
    commands::execute(cli.command)
}
