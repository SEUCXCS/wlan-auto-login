//! Top-level task dispatch.
use anyhow::Result;

use crate::cli::Command;

mod bundle;
mod check_boundaries;

pub fn run(command: &Command) -> Result<()> {
    match command {
        Command::Bundle(options) => bundle::run(options),
        Command::CheckBoundaries => check_boundaries::run(),
    }
}
