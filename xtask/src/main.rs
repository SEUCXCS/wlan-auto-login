//! Developer tasks for the workspace.
use anyhow::Result;
use clap::Parser;

mod cli;
mod commands;
mod support;

fn main() -> Result<()> {
    commands::run(&cli::Cli::parse().command)
}
