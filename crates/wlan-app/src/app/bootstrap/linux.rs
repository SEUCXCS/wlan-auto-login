//! Start Linux commands on the shared Tokio runtime.
use super::entry;
use crate::app::cli::Cli;
use std::process::ExitCode;

pub(in crate::app) fn boot(args: Cli) -> ExitCode {
    super::run_with_runtime(|runtime| runtime.block_on(entry(args)))
}
