//! Build explicit binary targets and locate their reported Cargo artifacts.
use anyhow::{Context, Result, ensure};
use std::{
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

pub struct BuildOptions {
    pub profile: String,
    pub target: Option<String>,
    pub target_dir: Option<PathBuf>,
}

pub fn build(root: &Path, package: &str, binary: &str, options: &BuildOptions) -> Result<PathBuf> {
    let mut command = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()));
    command.current_dir(root).args([
        "build",
        "--locked",
        "--package",
        package,
        "--bin",
        binary,
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
    let parsed = read_artifacts(BufReader::new(output), binary);
    // Always reap Cargo, including when its output cannot be parsed.
    let status = child.wait()?;
    ensure!(status.success(), "Cargo build failed: {status}");
    parsed?.with_context(|| format!("Cargo did not report the {binary} executable"))
}

fn read_artifacts(input: impl BufRead, binary: &str) -> Result<Option<PathBuf>> {
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
            && value["target"]["name"] == binary
            && let Some(path) = value["executable"].as_str()
        {
            executable = Some(PathBuf::from(path));
        }
    }
    Ok(executable)
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
            read_artifacts(input.as_bytes(), "wlanlogin")?,
            Some(PathBuf::from(
                "/custom dir/aarch64-apple-darwin/debug/wlanlogin"
            ))
        );
        assert_eq!(
            read_artifacts(input.as_bytes(), "other")?,
            Some(PathBuf::from("/wrong"))
        );
        assert_eq!(read_artifacts(input.as_bytes(), "missing")?, None);
        assert!(read_artifacts(b"not json".as_slice(), "wlanlogin").is_err());
        Ok(())
    }
}
