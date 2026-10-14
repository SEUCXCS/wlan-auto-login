//! Command declarations and reusable CLI arguments.
use clap::{Args, Parser, Subcommand};
use std::path::PathBuf;

use crate::support::cargo::BuildOptions;

pub mod bundle;

#[derive(Parser)]
#[command(about = "Developer tasks for the workspace", version)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Compile the application, assemble a bundle, and verify its signature.
    Bundle(bundle::BundleArgs),
}

#[derive(Args)]
pub struct BuildArgs {
    #[arg(long, default_value = "release")]
    pub profile: String,
    /// Rust target triple (defaults to Cargo's configured target).
    #[arg(long)]
    pub target: Option<String>,
    #[arg(long, env = "CARGO_TARGET_DIR")]
    pub target_dir: Option<PathBuf>,
}

impl From<&BuildArgs> for BuildOptions {
    fn from(args: &BuildArgs) -> Self {
        Self {
            profile: args.profile.clone(),
            target: args.target.clone(),
            target_dir: args.target_dir.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::bundle::BundlePlatform;

    #[test]
    fn existing_macos_invocation_keeps_build_and_signing_options() -> anyhow::Result<()> {
        let cli = Cli::try_parse_from([
            "xtask",
            "bundle",
            "macos",
            "--profile",
            "dev",
            "--target",
            "aarch64-apple-darwin",
            "--target-dir",
            "custom build",
            "--codesign-identity=-",
        ])?;
        let Command::Bundle(bundle) = cli.command;
        let BundlePlatform::Macos(options) = bundle.platform;
        let build = BuildOptions::from(&options.build);
        assert_eq!(build.profile, "dev");
        assert_eq!(build.target.as_deref(), Some("aarch64-apple-darwin"));
        assert_eq!(build.target_dir, Some(PathBuf::from("custom build")));
        assert_eq!(options.codesign_identity, "-");
        Ok(())
    }

    #[test]
    fn bundle_requires_a_platform_and_scopes_options_to_it() {
        for arguments in [
            vec!["xtask", "bundle"],
            vec!["xtask", "bundle", "unknown"],
            vec!["xtask", "bundle", "--codesign-identity=-", "macos"],
            vec!["xtask", "bundle", "--profile", "dev", "macos"],
        ] {
            assert!(Cli::try_parse_from(arguments).is_err());
        }
    }

    #[test]
    fn signing_and_output_options_keep_environment_precedence() -> anyhow::Result<()> {
        // Child environments avoid mutating process-wide variables during parallel tests.
        for case in ["environment", "override", "defaults"] {
            let mut child = std::process::Command::new(std::env::current_exe()?);
            child
                .args(["--exact", "cli::tests::environment_child", "--ignored"])
                .env("XTASK_TEST_CLI_CASE", case)
                .env_remove("CODESIGN_IDENTITY")
                .env_remove("CARGO_TARGET_DIR");
            if case != "defaults" {
                child
                    .env("CODESIGN_IDENTITY", "environment identity")
                    .env("CARGO_TARGET_DIR", "environment build");
            }
            let output = child.output()?;
            assert!(
                output.status.success(),
                "CLI environment case {case} failed:\n{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr),
            );
        }
        Ok(())
    }

    #[test]
    #[ignore = "Argument parser helper invoked in an isolated environment by its parent test"]
    fn environment_child() -> anyhow::Result<()> {
        let case = std::env::var("XTASK_TEST_CLI_CASE")?;
        let mut arguments = vec!["xtask", "bundle", "macos"];
        if case == "override" {
            arguments.extend(["--codesign-identity=-", "--target-dir", "explicit build"]);
        }
        let cli = Cli::try_parse_from(arguments)?;
        let Command::Bundle(bundle) = cli.command;
        let BundlePlatform::Macos(options) = bundle.platform;
        assert_eq!(options.build.profile, "release");
        assert_eq!(options.build.target, None);
        let (identity, directory) = match case.as_str() {
            "environment" => ("environment identity", Some("environment build")),
            "override" => ("-", Some("explicit build")),
            "defaults" => ("-", None),
            _ => panic!("Unknown CLI environment case: {case}"),
        };
        assert_eq!(options.codesign_identity, identity);
        assert_eq!(options.build.target_dir, directory.map(PathBuf::from));
        Ok(())
    }

    #[test]
    fn help_is_available_at_each_command_level() {
        for arguments in [
            vec!["xtask", "--help"],
            vec!["xtask", "bundle", "--help"],
            vec!["xtask", "bundle", "macos", "--help"],
        ] {
            let error = match Cli::try_parse_from(arguments) {
                Err(error) => error,
                Ok(_) => panic!("Help did not terminate argument parsing"),
            };
            assert_eq!(error.kind(), clap::error::ErrorKind::DisplayHelp);
        }
    }
}
