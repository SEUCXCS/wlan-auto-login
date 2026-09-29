//! Developer tooling: compile, package and verify a macOS application.
use anyhow::{Context, Result, bail, ensure};
use clap::{Args, Parser, Subcommand, ValueEnum};
use std::{
    fs,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

#[allow(dead_code)]
#[path = "../../crates/wlan-app/src/infra/identity.rs"]
mod identity;

#[derive(Parser)]
#[command(about = "Build application bundles", version)]
struct Cli {
    #[command(subcommand)]
    command: Task,
}

#[derive(Subcommand)]
enum Task {
    /// Compile the application, assemble a bundle, and verify its signature.
    Bundle(Bundle),
}

#[derive(Clone, Copy, ValueEnum)]
enum Platform {
    Macos,
}

#[derive(Args)]
struct Bundle {
    platform: Platform,
    #[arg(long, default_value = "release")]
    profile: String,
    /// Rust target triple (defaults to Cargo's configured target).
    #[arg(long)]
    target: Option<String>,
    #[arg(long, env = "CARGO_TARGET_DIR")]
    target_dir: Option<PathBuf>,
    /// Signing certificate name or hash; '-' selects ad-hoc signing.
    #[arg(
        long,
        env = "CODESIGN_IDENTITY",
        default_value = "-",
        allow_hyphen_values = true
    )]
    codesign_identity: String,
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Task::Bundle(options) => bundle(&options),
    }
}

fn bundle(options: &Bundle) -> Result<()> {
    ensure!(
        cfg!(target_os = "macos"),
        "macOS bundling requires a macOS host with Xcode Command Line Tools"
    );
    if let Some(target) = &options.target {
        ensure!(
            target.ends_with("-apple-darwin"),
            "macos requires an Apple Darwin target"
        );
    }
    ensure!(
        !options.codesign_identity.trim().is_empty(),
        "Code signing identity must not be empty"
    );
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .context("Workspace root is missing")?;
    let binary = build(root, options)?;
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
    let contents = app.join("Contents");
    fs::create_dir_all(contents.join("MacOS"))?;
    fs::create_dir_all(contents.join("Resources"))?;
    fs::copy(&binary, contents.join("MacOS").join(identity::APP_NAME))?;
    fs::write(contents.join("Info.plist"), plist())?;
    fs::write(contents.join("PkgInfo"), b"APPL????")?;
    run(Command::new("/usr/bin/plutil")
        .arg("-lint")
        .arg(contents.join("Info.plist")))?;
    let mut sign = Command::new("/usr/bin/codesign");
    sign.args([
        "--force",
        "--sign",
        &options.codesign_identity,
        "--identifier",
        identity::APP_ID,
    ]);
    // Real signing identities support the hardened runtime used for distribution.
    if options.codesign_identity != "-" {
        sign.args(["--options", "runtime", "--timestamp"]);
    }
    run(sign.arg(&app))?;
    run(Command::new("/usr/bin/codesign")
        .args(["--verify", "--deep", "--strict", "--verbose=2"])
        .arg(&app))?;
    let destination = output.join(app_name);
    if let Err(error) = publish(&app, &destination, staging.path()) {
        let retained = staging.keep();
        return Err(error)
            .with_context(|| format!("Staging files retained at {}", retained.display()));
    }
    println!("Bundle: {}", destination.display());
    println!("Signing identity: {}", options.codesign_identity);
    if options.codesign_identity == "-" {
        println!("Ad-hoc builds may need fresh Location and Keychain approval after rebuilding.");
    }
    Ok(())
}

fn build(root: &Path, options: &Bundle) -> Result<PathBuf> {
    let mut command = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()));
    command.current_dir(root).args([
        "build",
        "--locked",
        "--package",
        "wlanlogin-cli",
        "--bin",
        identity::APP_NAME,
        "--profile",
        &options.profile,
        "--message-format=json-render-diagnostics",
    ]);
    if let Some(target) = &options.target {
        command.args(["--target", target]);
    }
    if let Some(directory) = &options.target_dir {
        command.arg("--target-dir").arg(directory);
    }
    let mut child = command
        .stdout(Stdio::piped())
        .spawn()
        .context("Failed to start Cargo")?;
    let output = child.stdout.take().context("Cargo stdout is unavailable")?;
    let parsed = read_artifacts(BufReader::new(output));
    // Always reap Cargo, including when its output cannot be parsed.
    let status = child.wait()?;
    ensure!(status.success(), "Cargo build failed: {status}");
    parsed?.context("Cargo did not report the wlanlogin executable")
}

fn read_artifacts(input: impl BufRead) -> Result<Option<PathBuf>> {
    let mut executable = None;
    for line in input.lines() {
        let line = line?;
        let value: serde_json::Value =
            serde_json::from_str(&line).context("Invalid Cargo build output")?;
        if value["reason"] == "compiler-message" {
            if let Some(message) = value["message"]["rendered"].as_str() {
                eprint!("{message}");
            }
        } else if value["reason"] == "compiler-artifact"
            && value["target"]["name"] == identity::APP_NAME
            && let Some(path) = value["executable"].as_str()
        {
            executable = Some(PathBuf::from(path));
        }
    }
    Ok(executable)
}

fn plist() -> String {
    include_str!("../../packaging/macos/Info.plist")
        .replace("@APP_ID@", identity::APP_ID)
        .replace("@APP_NAME@", identity::APP_NAME)
        .replace("@VERSION@", env!("CARGO_PKG_VERSION"))
}

fn run(command: &mut Command) -> Result<()> {
    let status = command
        .status()
        .with_context(|| format!("Failed to run {}", command.get_program().to_string_lossy()))?;
    ensure!(
        status.success(),
        "{} failed: {status}",
        command.get_program().to_string_lossy()
    );
    Ok(())
}

fn publish(app: &Path, destination: &Path, staging: &Path) -> Result<()> {
    let backup = staging.join("previous.app");
    let existed = match fs::symlink_metadata(destination) {
        Ok(metadata) => {
            ensure!(
                metadata.is_dir() && !metadata.file_type().is_symlink(),
                "Refusing to replace a redirected or non-directory bundle"
            );
            let previous = fs::read_to_string(destination.join("Contents/Info.plist"))?;
            ensure!(
                previous.contains(&format!("<string>{}</string>", identity::APP_ID)),
                "Refusing to replace an unrelated bundle"
            );
            fs::rename(destination, &backup)?;
            true
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => return Err(error.into()),
    };
    if let Err(error) = fs::rename(app, destination) {
        if existed && let Err(rollback) = fs::rename(&backup, destination) {
            // The caller must retain the temporary directory if rollback ever fails.
            let recovery = destination.with_extension("app.recovery");
            fs::rename(&backup, &recovery)
                .context("Failed to preserve previous bundle during rollback")?;
            bail!(
                "Bundle publication failed: {error}; rollback failed: {rollback}; previous bundle: {}",
                recovery.display()
            );
        }
        return Err(error).context("Failed to publish bundle; previous bundle restored");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn artifacts_use_cargos_actual_path_and_ignore_other_binaries() -> Result<()> {
        let input = concat!(
            "{\"reason\":\"compiler-artifact\",\"target\":{\"name\":\"other\"},\"executable\":\"/wrong\"}\n",
            "{\"reason\":\"compiler-artifact\",\"target\":{\"name\":\"wlanlogin\"},\"executable\":\"/custom dir/aarch64-apple-darwin/debug/wlanlogin\"}\n"
        );
        assert_eq!(
            read_artifacts(input.as_bytes())?,
            Some(PathBuf::from(
                "/custom dir/aarch64-apple-darwin/debug/wlanlogin"
            ))
        );
        assert!(read_artifacts(b"not json".as_slice()).is_err());
        Ok(())
    }

    #[test]
    fn failed_publication_restores_the_previous_bundle() -> Result<()> {
        let root = tempfile::tempdir()?;
        let destination = root.path().join("wlanlogin.app");
        fs::create_dir_all(destination.join("Contents"))?;
        fs::write(destination.join("Contents/Info.plist"), plist())?;
        fs::write(destination.join("previous-version"), "keep")?;
        assert!(publish(&root.path().join("missing.app"), &destination, root.path()).is_err());
        assert_eq!(
            fs::read_to_string(destination.join("previous-version"))?,
            "keep"
        );
        Ok(())
    }
}
