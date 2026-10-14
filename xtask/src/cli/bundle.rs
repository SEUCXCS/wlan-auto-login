//! Platform subcommands and their packaging options.
use clap::{Args, Subcommand};

use super::BuildArgs;

#[derive(Args)]
pub struct BundleArgs {
    #[command(subcommand)]
    pub platform: BundlePlatform,
}

#[derive(Subcommand)]
pub enum BundlePlatform {
    /// Assemble and sign a macOS application bundle.
    Macos(MacosArgs),
}

#[derive(Args)]
pub struct MacosArgs {
    #[command(flatten)]
    pub build: BuildArgs,
    /// Signing certificate name or hash; '-' selects ad-hoc signing.
    #[arg(
        long,
        env = "CODESIGN_IDENTITY",
        default_value = "-",
        allow_hyphen_values = true
    )]
    pub codesign_identity: String,
}
