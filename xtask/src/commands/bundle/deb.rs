//! Package the unchanged CLI as a Debian binary package with the native dpkg tools.
use anyhow::{Context, Result, bail, ensure};
use std::{
    ffi::OsString,
    fs,
    path::Path,
    process::{Command, Stdio},
};

use super::linux::{self, stage, write_file};
use crate::{
    cli::bundle::DebArgs,
    support::{cargo, identity, platform, process, workspace},
};

/// Debian 12 ships glibc 2.36, the oldest of the supported releases (Ubuntu 24.04 ships 2.39).
const OLDEST_GLIBC: &str = "2.36";

pub fn run(options: &DebArgs) -> Result<()> {
    platform::require_linux()?;
    let target = options
        .build
        .target
        .as_deref()
        .unwrap_or(std::env::consts::ARCH);
    let architecture = architecture(target)?;
    // Fail before compiling when the host or the sources cannot produce a package.
    linux::require_tools(
        &["dpkg", "dpkg-deb", "dpkg-shlibdeps", "gzip"],
        "install dpkg-dev and gzip on a Debian or Ubuntu host",
    )?;
    let root = workspace::root()?;
    let upstream = env!("CARGO_PKG_VERSION");
    let version = debian_version(upstream, options.revision);
    let date = linux::package_date(root, "+%a, %d %b %Y %H:%M:%S %z")?;
    let control_template = fs::read_to_string(root.join("packaging/linux/distro/debian/control"))?;
    let maintainer = control_field(&control_template, "Maintainer")?;

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
        .prefix(".deb-")
        .tempdir_in(&output)?;
    let package = staging.path().join("package");

    let executable = package.join("usr/bin").join(identity::APP_NAME);
    stage(&binary, &executable, 0o755)?;

    let documents = package.join("usr/share/doc").join(identity::APP_NAME);
    write_file(
        &documents.join("copyright"),
        &fs::read_to_string(root.join("packaging/linux/distro/debian/copyright"))?.replace(
            "@LICENSE@",
            fs::read_to_string(root.join("LICENSE"))?.trim_end(),
        ),
    )?;
    stage_document(&root.join("README.md"), &documents.join("README.md"))?;
    let debian_changelog = documents.join("changelog.Debian");
    write_file(
        &debian_changelog,
        &debian_changelog_entry(&version, upstream, maintainer, &date),
    )?;
    compress(&debian_changelog)?;

    let page = format!("{}.1", identity::APP_NAME);
    let manuals = root.join("packaging/linux/man");
    let installed_manuals = package.join("usr/share/man");
    stage_document(
        &manuals.join(&page),
        &installed_manuals.join("man1").join(&page),
    )?;
    stage_document(
        &manuals.join("zh_CN").join(&page),
        &installed_manuals.join("zh_CN/man1").join(&page),
    )?;

    let installed_size = normalize(&package)?;
    // dpkg-shlibdeps locates the package root of the analysed binary by its DEBIAN directory.
    let metadata = package.join("DEBIAN");
    fs::create_dir(&metadata)?;
    platform::set_mode(&metadata, 0o755)?;
    let depends = shared_library_depends(staging.path(), &executable)?;
    require_supported_glibc(&depends)?;
    write_file(
        &metadata.join("control"),
        &control_template
            .replace("@APP_NAME@", identity::APP_NAME)
            .replace("@VERSION@", &version)
            .replace("@ARCHITECTURE@", architecture)
            .replace("@INSTALLED_SIZE@", &installed_size.to_string())
            .replace("@DEPENDS@", &depends),
    )?;

    let name = format!("{}_{version}_{architecture}.deb", identity::APP_NAME);
    let built = staging.path().join(&name);
    process::run(
        Command::new("dpkg-deb")
            .args(["--root-owner-group", "-Zxz", "--build"])
            .arg(&package)
            .arg(&built),
    )?;
    // Publish only a package that dpkg-deb accepted.
    let destination = output.join(name);
    fs::rename(built, &destination)?;
    println!("Bundle: {}", destination.display());
    println!("Version: {version}");
    println!("Depends: {depends}");
    Ok(())
}

fn architecture(target: &str) -> Result<&'static str> {
    match target {
        "x86_64" | "x86_64-unknown-linux-gnu" => Ok("amd64"),
        "aarch64" | "aarch64-unknown-linux-gnu" => Ok("arm64"),
        _ => {
            bail!("Debian packages support x86_64-unknown-linux-gnu and aarch64-unknown-linux-gnu")
        }
    }
}

/// A Cargo pre-release must sort before its release, which Debian expresses with `~`.
fn debian_version(upstream: &str, revision: u32) -> String {
    format!("{}-{revision}", upstream.replacen('-', "~", 1))
}

fn control_field<'a>(control: &'a str, name: &str) -> Result<&'a str> {
    control
        .lines()
        .find_map(|line| line.strip_prefix(name)?.strip_prefix(": "))
        .with_context(|| format!("packaging/linux/distro/debian/control has no {name} field"))
}

fn debian_changelog_entry(version: &str, upstream: &str, maintainer: &str, date: &str) -> String {
    format!(
        "{} ({version}) unstable; urgency=medium\n\n  * Package upstream release {upstream}.\n    See README.md.gz for the release notes link.\n\n -- {maintainer}  {date}\n",
        identity::APP_NAME
    )
}

fn stage_document(source: &Path, destination: &Path) -> Result<()> {
    stage(source, destination, 0o644)?;
    compress(destination)
}

/// Debian policy expects documentation and manual pages compressed reproducibly.
fn compress(path: &Path) -> Result<()> {
    process::run(Command::new("gzip").arg("-9n").arg(path))
}

/// Give every payload directory mode 0755 and return the installed size in KiB.
fn normalize(directory: &Path) -> Result<u64> {
    platform::set_mode(directory, 0o755)?;
    let mut kibibytes = 0;
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        kibibytes += if entry.file_type()?.is_dir() {
            normalize(&entry.path())?
        } else {
            entry.metadata()?.len().div_ceil(1024)
        };
    }
    Ok(kibibytes)
}

fn shared_library_depends(staging: &Path, executable: &Path) -> Result<String> {
    // dpkg-shlibdeps reads package names from debian/control in its working directory.
    fs::create_dir(staging.join("debian"))?;
    fs::write(
        staging.join("debian/control"),
        format!(
            "Source: {0}\n\nPackage: {0}\nArchitecture: any\n",
            identity::APP_NAME
        ),
    )?;
    let mut executable_argument = OsString::from("-e");
    executable_argument.push(executable);
    let output = Command::new("dpkg-shlibdeps")
        .current_dir(staging)
        .arg("-O")
        .arg(executable_argument)
        .stderr(Stdio::inherit())
        .output()
        .context("Failed to run dpkg-shlibdeps")?;
    ensure!(
        output.status.success(),
        "dpkg-shlibdeps failed: {}",
        output.status
    );
    Ok(substvar(&String::from_utf8(output.stdout)?)?.to_owned())
}

fn substvar(output: &str) -> Result<&str> {
    output
        .lines()
        .find_map(|line| line.strip_prefix("shlibs:Depends="))
        .filter(|depends| !depends.is_empty())
        .context("dpkg-shlibdeps reported no shared library dependencies")
}

/// Refuse packages that would not install on the oldest supported release.
fn require_supported_glibc(depends: &str) -> Result<()> {
    let required = glibc_requirement(depends).context(
        "dpkg-shlibdeps reported no versioned libc6 dependency; cannot verify Debian 12 glibc compatibility",
    )?;
    let status = Command::new("dpkg")
        .args(["--compare-versions", required, "le", OLDEST_GLIBC])
        .status()
        .context("Failed to run dpkg")?;
    ensure!(
        status.success(),
        "The binary requires glibc {required}, but Debian 12 provides {OLDEST_GLIBC}; build the package on Debian 12"
    );
    Ok(())
}

fn glibc_requirement(depends: &str) -> Option<&str> {
    depends.split(", ").find_map(|dependency| {
        dependency
            .strip_prefix("libc6 (>= ")?
            .strip_suffix(')')
            .filter(|version| !version.is_empty())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prerelease_versions_sort_before_their_release() {
        assert_eq!(debian_version("0.2.0", 1), "0.2.0-1");
        assert_eq!(debian_version("0.3.0-alpha.1", 2), "0.3.0~alpha.1-2");
    }

    #[test]
    fn only_linux_gnu_targets_map_to_debian_architectures() -> Result<()> {
        assert_eq!(architecture("x86_64")?, "amd64");
        assert_eq!(architecture("aarch64-unknown-linux-gnu")?, "arm64");
        for target in [
            "x86_64-pc-windows-msvc",
            "aarch64-apple-darwin",
            "x86_64-unknown-linux-musl",
        ] {
            assert!(architecture(target).is_err());
        }
        Ok(())
    }

    #[test]
    fn shared_library_depends_must_be_reported() -> Result<()> {
        assert_eq!(
            substvar("shlibs:Depends=libc6 (>= 2.34), libdbus-1-3 (>= 1.9.14)\n")?,
            "libc6 (>= 2.34), libdbus-1-3 (>= 1.9.14)"
        );
        assert!(substvar("").is_err());
        assert!(substvar("shlibs:Depends=\n").is_err());
        Ok(())
    }

    #[test]
    fn glibc_requirement_ignores_other_libraries() {
        assert_eq!(
            glibc_requirement("libc6 (>= 2.39), libdbus-1-3 (>= 1.10), libgcc-s1 (>= 4.2)"),
            Some("2.39")
        );
        assert_eq!(glibc_requirement("libdbus-1-3 (>= 1.10)"), None);
        assert_eq!(glibc_requirement("libc6, libdbus-1-3 (>= 1.10)"), None);
        assert_eq!(glibc_requirement("libc6 (>= )"), None);
    }

    #[test]
    fn glibc_floor_rejects_missing_or_unversioned_libc6() {
        for depends in [
            "libdbus-1-3 (>= 1.10)",
            "libc6, libdbus-1-3 (>= 1.10)",
            "libc6 (>= )",
        ] {
            assert!(require_supported_glibc(depends).is_err());
        }
    }
}
