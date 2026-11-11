//! Build the unchanged CLI, two scoped MSIs, and a single native setup executable.
use anyhow::{Context, Result, bail, ensure};
use std::{fs, process::Command};

use crate::{
    cli::bundle::WindowsArgs,
    support::{cargo, identity, process, workspace},
};

pub fn run(options: &WindowsArgs) -> Result<()> {
    crate::support::platform::require_windows()?;
    let root = workspace::root()?;
    let target = options
        .build
        .target
        .as_deref()
        .unwrap_or(std::env::consts::ARCH);
    let (wix_arch, arch) = architecture(target)?;
    process::run(Command::new(&options.wix).arg("--version"))?;
    let binary = cargo::build(
        root,
        "wlanlogin-cli",
        identity::APP_NAME,
        &(&options.build).into(),
    )?;
    let output = binary
        .parent()
        .context("CLI artifact has no parent")?
        .join("bundle");
    fs::create_dir_all(&output)?;
    let staging = tempfile::Builder::new()
        .prefix(".windows-setup-")
        .tempdir_in(&output)?;
    let icon = root.join("packaging/windows/icon/icon.ico");
    ensure!(icon.is_file(), "The packaged Windows icon is missing");
    let installer_root = root.join("packaging/windows");
    let license_rtf = staging.path().join("license.rtf");
    let license = fs::read_to_string(root.join("LICENSE"))?;
    fs::write(
        &license_rtf,
        format!(
            "{{\\rtf1\\ansi\\deff0{{\\fonttbl{{\\f0 Segoe UI;}}}}\\fs18 {} }}",
            license
                .replace('\\', "\\\\")
                .replace('{', "\\{")
                .replace('}', "\\}")
                .replace('\n', "\\par\n")
        ),
    )?;
    let initializer = staging.path().join("initialize.dll");
    let mut rustc = Command::new(std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into()));
    rustc
        .arg(installer_root.join("initialize.rs"))
        .args(["--crate-type", "cdylib", "--edition", "2024", "-O", "-o"])
        .arg(&initializer);
    if let Some(target) = &options.build.target {
        rustc.args(["--target", target]);
    }
    process::run(&mut rustc)?;
    let build = cargo::BuildOptions {
        profile: options.build.profile.clone(),
        target: options.build.target.clone(),
        target_dir: Some(output.join("installer-build")),
    };
    let action = cargo::build(
        &installer_root,
        "wlanlogin-installer",
        "wlanlogin-installer-action",
        &build,
    )?;
    let verifier = if arch == std::env::consts::ARCH {
        action.clone()
    } else {
        let native = cargo::BuildOptions {
            target: Some(format!("{}-pc-windows-msvc", std::env::consts::ARCH)),
            profile: build.profile.clone(),
            target_dir: build.target_dir.clone(),
        };
        cargo::build(
            &installer_root,
            "wlanlogin-installer",
            "wlanlogin-installer-action",
            &native,
        )?
    };
    for (scope, system, folder, registry, upgrade) in [
        (
            "user",
            "no",
            "LocalAppDataFolder",
            "HKCU",
            "397F9B01-F368-468B-AD7C-23822E646F33",
        ),
        (
            "machine",
            "yes",
            "ProgramFiles64Folder",
            "HKLM",
            "C4F10FA7-8C23-43A7-851C-F0CD091F5CD0",
        ),
    ] {
        let msi = staging.path().join(format!("{scope}.msi"));
        let mut command = Command::new(&options.wix);
        command
            .args([
                "build",
                "-nologo",
                "-ext",
                "WixToolset.UI.wixext",
                "-culture",
                "zh-CN",
                "-arch",
                wix_arch,
                "-o",
            ])
            .arg(&msi)
            .arg(installer_root.join("installer.wxs"));
        for (name, value) in [
            ("Scope", scope),
            ("System", system),
            ("Impersonate", if scope == "user" { "yes" } else { "no" }),
            ("RootFolder", folder),
            ("RegistryRoot", registry),
            ("UpgradeCode", upgrade),
            ("Version", env!("CARGO_PKG_VERSION")),
            (
                "ComponentSuffix",
                match (scope, wix_arch) {
                    ("user", "x64") => "000000000001",
                    ("machine", "x64") => "000000000002",
                    ("user", _) => "000000000003",
                    _ => "000000000004",
                },
            ),
        ] {
            command.arg("-d").arg(format!("{name}={value}"));
        }
        for (name, path) in [
            ("Cli", binary.as_path()),
            ("Action", action.as_path()),
            ("Initializer", initializer.as_path()),
            ("Icon", icon.as_path()),
            ("Readme", root.join("README.md").as_path()),
            ("License", root.join("LICENSE").as_path()),
            ("LicenseRtf", license_rtf.as_path()),
        ] {
            command.arg("-d").arg(format!("{name}={}", path.display()));
        }
        process::run(&mut command)?;
        process::run(
            Command::new(&verifier)
                .arg("--verify-package")
                .arg(&msi)
                .arg(&binary)
                .arg(root.join("README.md"))
                .arg(root.join("LICENSE"))
                .arg(scope),
        )?;
    }
    let setup = cargo::build_with(
        &installer_root,
        "wlanlogin-installer",
        "wlanlogin-setup",
        &build,
        |command| {
            command
                .env("WLANLOGIN_USER_MSI", staging.path().join("user.msi"))
                .env("WLANLOGIN_MACHINE_MSI", staging.path().join("machine.msi"))
                .env("WLANLOGIN_ICON", &icon);
        },
    )?;
    let destination = output.join(format!("wlanlogin-windows-setup-{arch}.exe"));
    let temporary = staging.path().join("setup.exe");
    fs::copy(setup, &temporary)?;
    // Publish only after both MSIs and the embedded bootstrapper build succeed.
    fs::rename(temporary, &destination)?;
    println!("Bundle: {}", destination.display());
    Ok(())
}

fn architecture(target: &str) -> Result<(&'static str, &'static str)> {
    match target {
        "x86_64" | "x86_64-pc-windows-msvc" => Ok(("x64", "x86_64")),
        "aarch64" | "aarch64-pc-windows-msvc" => Ok(("arm64", "aarch64")),
        _ => bail!("Windows setup supports x86_64-pc-windows-msvc and aarch64-pc-windows-msvc"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shared_icon_produces_a_valid_windows_directory() -> Result<()> {
        let icon = include_bytes!("../../../../packaging/windows/icon/icon.ico");
        assert_eq!(&icon[..6], &[0, 0, 1, 0, 4, 0]);
        for index in 0..4 {
            let entry = 6 + index * 16;
            let size = u32::from_le_bytes(icon[entry + 8..entry + 12].try_into()?) as usize;
            let offset = u32::from_le_bytes(icon[entry + 12..entry + 16].try_into()?) as usize;
            assert!(icon[offset..offset + size].starts_with(b"\x89PNG"));
        }
        assert!(architecture("x86_64-unknown-linux-gnu").is_err());
        Ok(())
    }
}
