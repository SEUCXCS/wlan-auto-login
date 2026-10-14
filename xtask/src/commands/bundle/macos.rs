//! Compile, stage, sign, verify, and publish a macOS application bundle.
use anyhow::{Context, Result, ensure};
use std::fs;

use crate::{
    cli::bundle::MacosArgs,
    support::{cargo, identity, workspace},
};

mod assemble;
mod build_number;
mod publish;
mod sign;

pub fn run(options: &MacosArgs) -> Result<()> {
    // Validate before building or allocating a bundle build number.
    ensure!(
        cfg!(target_os = "macos"),
        "macOS bundling requires a macOS host with Xcode Command Line Tools"
    );
    if let Some(target) = &options.build.target {
        ensure!(
            target.ends_with("-apple-darwin"),
            "macos requires an Apple Darwin target"
        );
    }
    ensure!(
        !options.codesign_identity.trim().is_empty(),
        "Code signing identity must not be empty"
    );

    let root = workspace::root()?;
    let build_options = cargo::BuildOptions::from(&options.build);
    let binary = cargo::build(root, "wlanlogin-cli", identity::APP_NAME, &build_options)?;
    let output = binary
        .parent()
        .context("Build artifact has no parent")?
        .join("bundle");
    fs::create_dir_all(&output)?;
    let staging = tempfile::Builder::new()
        .prefix(".wlanlogin-bundle-")
        .tempdir_in(&output)?;
    let app_name = format!("{}.app", identity::APP_NAME);
    let app = staging.path().join(&app_name);

    // Reserve before signing; failed bundles must not reuse an allocated number.
    let build_number = build_number::reserve(root)?;
    assemble::assemble(root, &binary, &app, build_number)?;
    sign::validate(&app)?;
    sign::sign(&app, &options.codesign_identity)?;
    sign::verify(&app)?;

    // Only a verified bundle may replace the previous output.
    let destination = output.join(app_name);
    if let Err(error) = publish::publish(&app, &destination, staging.path()) {
        let retained = staging.keep();
        return Err(error)
            .with_context(|| format!("Staging files retained at {}", retained.display()));
    }
    println!("Bundle: {}", destination.display());
    println!("Version: {}", env!("CARGO_PKG_VERSION"));
    println!("Build number: {build_number}");
    println!("Signing identity: {}", options.codesign_identity);
    if options.codesign_identity == "-" {
        println!("Ad-hoc builds may need fresh Location and Keychain approval after rebuilding.");
    }
    Ok(())
}
