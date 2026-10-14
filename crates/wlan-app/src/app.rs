//! Parse arguments and delegate to the selected platform bootstrap.
mod bootstrap;
mod cli;
mod context;
mod controller;
mod daemon;
mod tui;

use clap::Parser;

#[cfg(target_os = "linux")]
use bootstrap::linux::boot as bootstrap;
#[cfg(target_os = "macos")]
use bootstrap::macos::boot as bootstrap;
#[cfg(target_os = "windows")]
use bootstrap::windows::boot as bootstrap;

pub(crate) fn run() -> std::process::ExitCode {
    let args = cli::Cli::parse();
    bootstrap(args)
}
