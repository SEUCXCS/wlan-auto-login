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
    /// Build a Windows Installer setup executable.
    Windows(WindowsArgs),
}

#[derive(Args)]
pub struct WindowsArgs {
    #[command(flatten)]
    pub build: BuildArgs,
    /// Path to the WiX v6 command (defaults to wix on PATH).
    #[arg(long, env = "WIX", default_value = "wix")]
    pub wix: std::path::PathBuf,
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
