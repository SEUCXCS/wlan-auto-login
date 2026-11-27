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
    /// Build a Debian package (.deb) on a Debian or Ubuntu host.
    Deb(DebArgs),
}

#[derive(Args)]
pub struct DebArgs {
    #[command(flatten)]
    pub build: BuildArgs,
    /// Debian revision; increase it to republish the same upstream version.
    #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u32).range(1..))]
    pub revision: u32,
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
