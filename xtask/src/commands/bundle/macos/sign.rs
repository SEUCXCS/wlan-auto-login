//! macOS property-list validation, code signing, and signature verification.
use anyhow::Result;
use std::{path::Path, process::Command};

use crate::support::{identity, process};

pub fn validate(app: &Path) -> Result<()> {
    for system in [false, true] {
        let definition = app.join(identity::macos_service_path(system));
        process::run(
            Command::new("/usr/bin/plutil")
                .arg("-lint")
                .arg(&definition),
        )?;
    }
    process::run(
        Command::new("/usr/bin/plutil")
            .arg("-lint")
            .arg(app.join("Contents/Info.plist")),
    )
}

pub fn sign(app: &Path, codesign_identity: &str) -> Result<()> {
    let mut command = Command::new("/usr/bin/codesign");
    command.args([
        "--force",
        "--sign",
        codesign_identity,
        "--identifier",
        identity::APP_ID,
    ]);
    // Real signing identities support the hardened runtime used for distribution.
    if codesign_identity != "-" {
        command.args(["--options", "runtime", "--timestamp"]);
    }
    process::run(command.arg(app))
}

pub fn verify(app: &Path) -> Result<()> {
    process::run(
        Command::new("/usr/bin/codesign")
            .args(["--verify", "--deep", "--strict", "--verbose=2"])
            .arg(app),
    )
}
