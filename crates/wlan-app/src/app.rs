//! Parse arguments and delegate to the selected platform bootstrap.
mod bootstrap;
#[cfg(target_os = "linux")]
#[path = "app/bootstrap/linux/cli.rs"]
pub(crate) mod cli;
#[cfg(target_os = "macos")]
#[path = "app/bootstrap/macos/cli.rs"]
pub(crate) mod cli;
#[cfg(target_os = "windows")]
#[path = "app/bootstrap/windows/cli.rs"]
pub(crate) mod cli;
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
