//! Package the unchanged CLI as an RPM with rpmbuild for Fedora.
use anyhow::{Context, Result, bail, ensure};
use std::{
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use super::linux::{self, stage, write_file};
use crate::{
    cli::bundle::RpmArgs,
    support::{cargo, identity, platform, process, workspace},
};

/// Fedora 43 ships glibc 2.42, the oldest of the supported releases.
const OLDEST_GLIBC: &str = "2.42";

pub fn run(options: &RpmArgs) -> Result<()> {
    platform::require_linux()?;
    let target = options
        .build
        .target
        .as_deref()
        .unwrap_or(std::env::consts::ARCH);
    let architecture = architecture(target)?;
    // Fail before compiling when the host or the sources cannot produce a package.
    linux::require_tools(
        &["rpmbuild", "rpm"],
        "install rpm-build on Fedora, or rpm on Debian and Ubuntu",
    )?;
    let root = workspace::root()?;
    let upstream = env!("CARGO_PKG_VERSION");
    let version = rpm_version(upstream)?;
    let date = linux::package_date(root, "+%a %b %d %Y")?;
    let spec_template = fs::read_to_string(root.join("packaging/fedora/wlanlogin.spec"))?;

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
        .prefix(".rpm-")
        .tempdir_in(&output)?;

    // The spec installs these sources under the names it expects in %{_sourcedir}.
    let sources = staging.path().join("SOURCES");
    stage(&binary, &sources.join(identity::APP_NAME), 0o755)?;
    for document in ["LICENSE", "README.md"] {
        stage(&root.join(document), &sources.join(document), 0o644)?;
    }
    let page = format!("{}.1", identity::APP_NAME);
    let manuals = root.join("packaging/linux/man");
    stage(&manuals.join(&page), &sources.join(&page), 0o644)?;
    stage(
        &manuals.join("zh_CN").join(&page),
        &sources.join(format!("{}.zh_CN.1", identity::APP_NAME)),
        0o644,
    )?;

    let spec = staging
        .path()
        .join("SPECS")
        .join(format!("{}.spec", identity::APP_NAME));
    write_file(
        &spec,
        &spec_template
            .replace("@APP_NAME@", identity::APP_NAME)
            .replace("@VERSION@", &version)
            .replace("@RELEASE@", &options.release.to_string())
            .replace("@UPSTREAM@", upstream)
            .replace("@DATE@", &date),
    )?;
    let mut topdir = OsString::from("_topdir ");
    topdir.push(staging.path());
    process::run(
        Command::new("rpmbuild")
            .args(["-bb", "--quiet", "--target", architecture, "--define"])
            .arg(topdir)
            .arg(&spec),
    )?;

    let built = single_package(&staging.path().join("RPMS").join(architecture))?;
    let requires = query(&built, &["--requires"])?;
    let glibc = require_supported_glibc(&requires)?;
    let label = query(&built, &["--queryformat", "%{VERSION}-%{RELEASE}"])?;
    // Publish only a package that rpmbuild produced and the glibc floor accepted.
    let destination = output.join(built.file_name().context("RPM has no file name")?);
    fs::rename(&built, &destination)?;
    println!("Bundle: {}", destination.display());
    println!("Version: {label}");
    let others = requires
        .lines()
        .filter(|line| !line.contains("(GLIBC_") && !line.starts_with("rpmlib("))
        .collect::<Vec<_>>();
    println!("Requires: glibc >= {glibc}, {}", others.join(", "));
    Ok(())
}

fn architecture(target: &str) -> Result<&'static str> {
    match target {
        "x86_64" | "x86_64-unknown-linux-gnu" => Ok("x86_64"),
        "aarch64" | "aarch64-unknown-linux-gnu" => Ok("aarch64"),
        _ => bail!("RPM packages support x86_64-unknown-linux-gnu and aarch64-unknown-linux-gnu"),
    }
}

/// A Cargo pre-release must sort before its release, which RPM expresses with `~`.
fn rpm_version(upstream: &str) -> Result<String> {
    let version = upstream.replacen('-', "~", 1);
    ensure!(
        !version.contains('-'),
        "RPM versions cannot contain '-': {upstream}"
    );
    Ok(version)
}

fn single_package(directory: &Path) -> Result<PathBuf> {
    let mut packages = fs::read_dir(directory)
        .with_context(|| format!("rpmbuild produced no {}", directory.display()))?
        .map(|entry| Ok(entry?.path()))
        .collect::<Result<Vec<_>>>()?;
    ensure!(
        packages.len() == 1,
        "rpmbuild produced {} packages in {}",
        packages.len(),
        directory.display()
    );
    packages.pop().context("rpmbuild produced no package")
}

fn query(package: &Path, arguments: &[&str]) -> Result<String> {
    let output = Command::new("rpm")
        .args(["--query", "--package"])
        .args(arguments)
        .arg(package)
        .stderr(Stdio::inherit())
        .output()
        .context("Failed to run rpm")?;
    ensure!(
        output.status.success(),
        "rpm query failed: {}",
        output.status
    );
    Ok(String::from_utf8(output.stdout)?.trim_end().to_owned())
}

/// Refuse packages that would not install on the oldest supported release.
fn require_supported_glibc(requires: &str) -> Result<&str> {
    ensure!(
        !requires
            .lines()
            .any(|line| line.contains("(GLIBC_PRIVATE)")),
        "The binary depends on GLIBC_PRIVATE internal symbols and cannot be packaged for Fedora"
    );
    let required = glibc_requirement(requires).context(
        "rpmbuild generated no glibc requirement; is rpm's ELF dependency generator installed?",
    )?;
    let required_parts =
        version_parts(required).context("Failed to parse generated glibc symbol version")?;
    let oldest_parts = version_parts(OLDEST_GLIBC).context("Invalid OLDEST_GLIBC constant")?;
    ensure!(
        required_parts <= oldest_parts,
        "The binary requires glibc {required}, but Fedora 43 provides {OLDEST_GLIBC}; build the package on an older host"
    );
    Ok(required)
}

/// The highest numeric `GLIBC_x.y` symbol version across the generated glibc library requirements.
fn glibc_requirement(requires: &str) -> Option<&str> {
    requires
        .lines()
        .filter_map(|line| {
            let (_, version) = line.split_once("(GLIBC_")?;
            let (version, _) = version.split_once(')')?;
            let parts = version_parts(version)?;
            Some((parts, version))
        })
        .max_by(|(left, _), (right, _)| left.cmp(right))
        .map(|(_, version)| version)
}

fn version_parts(version: &str) -> Option<Vec<u32>> {
    if version.is_empty() {
        return None;
    }
    version
        .split('.')
        .map(|part| part.parse::<u32>().ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prerelease_versions_sort_before_their_release() -> Result<()> {
        assert_eq!(rpm_version("2.0.0")?, "2.0.0");
        assert_eq!(rpm_version("2.1.0-alpha.1")?, "2.1.0~alpha.1");
        assert!(rpm_version("2.1.0-alpha-1").is_err());
        Ok(())
    }

    #[test]
    fn only_linux_gnu_targets_map_to_rpm_architectures() -> Result<()> {
        assert_eq!(architecture("x86_64-unknown-linux-gnu")?, "x86_64");
        assert_eq!(architecture("aarch64")?, "aarch64");
        for target in ["x86_64-pc-windows-msvc", "x86_64-unknown-linux-musl"] {
            assert!(architecture(target).is_err());
        }
        Ok(())
    }

    #[test]
    fn glibc_requirement_is_the_highest_symbol_version() {
        let requires = "NetworkManager\nlibc.so.6()(64bit)\nlibc.so.6(GLIBC_2.2.5)(64bit)\nlibc.so.6(GLIBC_2.34)(64bit)\nlibc.so.6(GLIBC_2.9)(64bit)\nlibc.so.6(GLIBC_ABI_DT_RELR)(64bit)\nlibdbus-1.so.3(LIBDBUS_1_3)(64bit)\nlibm.so.6(GLIBC_2.38)(64bit)\nsystemd";
        assert_eq!(glibc_requirement(requires), Some("2.38"));
        assert_eq!(
            glibc_requirement("NetworkManager\nlibc.so.6()(64bit)"),
            None
        );
        assert_eq!(
            glibc_requirement("libc.so.6(GLIBC_ABI_DT_RELR)(64bit)"),
            None
        );
    }

    #[test]
    fn glibc_floor_rejects_newer_symbol_versions() {
        assert!(require_supported_glibc("libc.so.6(GLIBC_2.42)(64bit)").is_ok());
        assert!(require_supported_glibc("libc.so.6(GLIBC_2.43)(64bit)").is_err());
        assert!(require_supported_glibc("systemd").is_err());
        assert!(require_supported_glibc("libc.so.6(GLIBC_ABI_DT_RELR)(64bit)").is_err());
        assert!(require_supported_glibc("libc.so.6(GLIBC_PRIVATE)(64bit)").is_err());
        assert!(
            require_supported_glibc(
                "libc.so.6(GLIBC_2.34)(64bit)\nlibc.so.6(GLIBC_PRIVATE)(64bit)"
            )
            .is_err()
        );
    }
}
