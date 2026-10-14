//! Top-level task dispatch.
use anyhow::Result;

use crate::cli::Command;

mod bundle;

pub fn run(command: &Command) -> Result<()> {
    match command {
        Command::Bundle(options) => bundle::run(options),
    }
}
