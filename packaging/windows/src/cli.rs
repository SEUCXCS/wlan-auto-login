//! Invoke only the existing public CLI, with explicit scope and private stdin.
use crate::{Form, Scope};
use anyhow::{Context, Result, ensure};
use std::{
    io::Write,
    os::windows::process::CommandExt,
    path::Path,
    process::{Command, Stdio},
};

pub fn run(binary: &Path, scope: Scope, arguments: &[&str], form: Option<&Form>) -> Result<()> {
    let mut command = Command::new(binary);
    command
        .arg(scope.cli_flag())
        .args(arguments)
        .creation_flags(0x08000000)
        .env_remove("WLANLOGIN_PASSWORD")
        .env_remove("WLANLOGIN_USERNAME")
        .env_remove("WLANLOGIN_LINE")
        .stdin(if form.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Some(form) = form {
        form.validate()?;
        command
            .arg("--password-stdin")
            .arg("--username")
            .arg(&form.username)
            .arg("--line")
            .arg(&form.line);
    }
    let mut child = command.spawn().context("Could not start wlanlogin")?;
    let sent = if let Some(form) = form {
        let mut input = child.stdin.take().context("CLI stdin unavailable")?;
        input
            .write_all(form.password.as_bytes())
            .and_then(|_| input.write_all(b"\n"))
    } else {
        Ok(())
    };
    let status = child.wait().context("Could not wait for wlanlogin")?;
    sent.context("Could not provide credentials to wlanlogin")?;
    ensure!(
        status.success(),
        "wlanlogin operation failed (exit {:?})",
        status.code()
    );
    Ok(())
}
