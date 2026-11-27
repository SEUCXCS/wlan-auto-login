//! Package the unchanged CLI as a Debian binary package with the native dpkg tools.
use anyhow::{Context, Result, bail, ensure};
use std::{
    ffi::OsString,
    fs,
    path::Path,
    process::{Command, Stdio},
};

use crate::{
    cli::bundle::DebArgs,
    support::{cargo, identity, platform, process, workspace},
};

pub fn run(options: &DebArgs) -> Result<()> {
    platform::require_linux()?;
    let target = options
        .build
        .target
        .as_deref()
        .unwrap_or(std::env::consts::ARCH);
    let architecture = architecture(target)?;
    // Fail before compiling when the host cannot produce a Debian package.
    for tool in ["dpkg-deb", "dpkg-shlibdeps"] {
        let available = Command::new(tool)
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success());
        ensure!(
            available,
            "{tool} is required; install dpkg-dev on a Debian or Ubuntu host"
        );
    }

    let root = workspace::root()?;
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
    create_parent(&executable)?;
    fs::copy(&binary, &executable)?;
    platform::set_mode(&executable, 0o755)?;

    let documents = package.join("usr/share/doc").join(identity::APP_NAME);
    fs::create_dir_all(&documents)?;
    let copyright = documents.join("copyright");
    fs::write(
        &copyright,
        fs::read_to_string(root.join("packaging/debian/copyright"))?.replace(
            "@LICENSE@",
            fs::read_to_string(root.join("LICENSE"))?.trim_end(),
        ),
    )?;
    platform::set_mode(&copyright, 0o644)?;
    let readme = documents.join("README.md");
    fs::copy(root.join("README.md"), &readme)?;
    platform::set_mode(&readme, 0o644)?;
    // Debian policy expects larger documentation to be compressed reproducibly.
    process::run(Command::new("gzip").arg("-9n").arg(&readme))?;

    let installed_size = normalize(&package)?;
    // dpkg-shlibdeps locates the package root of the analysed binary by its DEBIAN directory.
    let metadata = package.join("DEBIAN");
    fs::create_dir(&metadata)?;
    platform::set_mode(&metadata, 0o755)?;
    let depends = shared_library_depends(staging.path(), &executable)?;
    let version = debian_version(env!("CARGO_PKG_VERSION"));
    let control = metadata.join("control");
    fs::write(
        &control,
        fs::read_to_string(root.join("packaging/debian/control"))?
            .replace("@APP_NAME@", identity::APP_NAME)
            .replace("@VERSION@", &version)
            .replace("@ARCHITECTURE@", architecture)
            .replace("@INSTALLED_SIZE@", &installed_size.to_string())
            .replace("@DEPENDS@", &depends),
    )?;
    platform::set_mode(&control, 0o644)?;

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
fn debian_version(version: &str) -> String {
    version.replacen('-', "~", 1)
}

fn create_parent(path: &Path) -> Result<()> {
    fs::create_dir_all(path.parent().context("Package path has no parent")?)?;
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prerelease_versions_sort_before_their_release() {
        assert_eq!(debian_version("0.2.0"), "0.2.0");
        assert_eq!(debian_version("0.3.0-alpha.1"), "0.3.0~alpha.1");
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
}
