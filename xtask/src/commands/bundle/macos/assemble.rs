//! Assemble the exact bundle inventory consumed by the application installer.
use anyhow::{Context, Result};
use std::{fs, path::Path};

use crate::support::identity;

pub fn assemble(root: &Path, binary: &Path, app: &Path, build_number: u64) -> Result<()> {
    for directory in identity::MACOS_BUNDLE_DIRECTORIES {
        fs::create_dir_all(app.join(directory))?;
    }
    let contents = app.join("Contents");
    fs::copy(binary, contents.join("MacOS").join(identity::APP_NAME))?;
    fs::copy(
        root.join("packaging/macos/icon/icon.icns"),
        contents.join("Resources/icon.icns"),
    )
    .context("Failed to copy the macOS application icon")?;
    fs::write(contents.join("Info.plist"), plist(build_number))?;
    fs::write(contents.join("PkgInfo"), b"APPL????")?;
    for system in [false, true] {
        fs::write(
            app.join(identity::macos_service_path(system)),
            identity::macos_service_plist(system),
        )?;
    }
    Ok(())
}

fn plist(build_number: u64) -> String {
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../packaging/macos/Info.plist"
    ))
    .replace("@APP_ID@", identity::APP_ID)
    .replace("@APP_NAME@", identity::APP_NAME)
    .replace("@DISPLAY_NAME@", identity::DISPLAY_NAME)
    .replace("@VERSION@", env!("CARGO_PKG_VERSION"))
    .replace("@BUILD_NUMBER@", &build_number.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::support::workspace;

    #[test]
    fn native_jobs_are_in_the_signed_inventory_and_have_no_installation_paths() {
        for system in [false, true] {
            assert!(identity::MACOS_BUNDLE_FILES.contains(&identity::macos_service_path(system)));
            let job = identity::macos_service_plist(system);
            assert!(!job.contains('@'));
            assert!(job.contains("<key>BundleProgram</key>"));
            assert!(job.contains(identity::APP_ID));
            assert!(!job.contains("/usr/bin/open") && !job.contains("/Users/"));
            assert!(job.contains(if system { "--system" } else { "--user" }));
        }
        assert!(plist(1).contains("<key>CFBundleName</key><string>CXCS WLAN Login</string>"));
    }

    #[test]
    fn assembly_includes_the_icon_and_display_name_without_renaming_the_executable() -> Result<()> {
        let root = workspace::root()?;
        let staging = tempfile::tempdir()?;
        let binary = staging.path().join("wlanlogin");
        fs::write(&binary, b"test executable")?;
        let app = staging.path().join("wlanlogin.app");
        assemble(root, &binary, &app, 42)?;
        let icon = fs::read(app.join("Contents/Resources/icon.icns"))?;
        assert_eq!(icon, fs::read(root.join("packaging/macos/icon/icon.icns"))?);
        assert!(icon.starts_with(b"icns"));
        assert!(identity::MACOS_BUNDLE_FILES.contains(&"Contents/Resources/icon.icns"));
        let metadata = fs::read_to_string(app.join("Contents/Info.plist"))?;
        assert!(!metadata.contains('@'));
        assert!(metadata.contains("<key>CFBundleIconFile</key><string>icon.icns</string>"));
        assert!(
            metadata.contains("<key>CFBundleDisplayName</key><string>CXCS WLAN Login</string>")
        );
        assert!(metadata.contains("<key>CFBundleExecutable</key><string>wlanlogin</string>"));
        assert!(metadata.contains(&format!(
            "<key>CFBundleShortVersionString</key><string>{}</string>",
            env!("CARGO_PKG_VERSION")
        )));
        assert!(metadata.contains("<key>CFBundleVersion</key><string>42</string>"));
        assert_eq!(
            fs::read(app.join("Contents/MacOS/wlanlogin"))?,
            b"test executable"
        );
        Ok(())
    }
}
