#[cfg(target_os = "linux")]
use super::command::SystemRunner;
#[cfg(any(target_os = "linux", test))]
use super::command::{Invocation, Runner};
use super::{
    definition::{Artifacts, Plan, plan},
    manager::Manager,
};
use crate::config::Paths;
#[cfg(any(target_os = "linux", test))]
use anyhow::Context;
use anyhow::{Result, ensure};
#[cfg(any(target_os = "linux", test))]
use std::io::Write;
#[cfg(any(target_os = "macos", windows, test))]
use std::path::PathBuf;

/// Options shared by historical and streaming service-log readers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LogOptions {
    pub lines: u32,
    pub follow: bool,
}

#[derive(Debug)]
enum Source {
    #[cfg(any(target_os = "linux", test))]
    Journal(Invocation),
    #[cfg(any(target_os = "macos", windows, test))]
    File(PathBuf),
}

fn source(paths: &Paths, plan: &Plan, options: LogOptions) -> Result<Source> {
    let files = Artifacts::read(plan)?;
    ensure!(
        files.definition || files.executable,
        "后台服务尚未安装，请先执行 install"
    );
    ensure!(files.complete(), "后台服务安装不完整，请重新执行 install");
    // Reading diagnostics must also work when the service is stopped or its
    // registration is broken. No service manager query or mutation is needed.
    match &plan.manager {
        #[cfg(any(target_os = "linux", test))]
        Manager::Systemd { system } => {
            let _ = paths;
            let mut command = Invocation::new(
                "journalctl",
                [
                    if *system { "--system" } else { "--user" },
                    "--unit",
                    plan.manager.name(),
                    "--no-pager",
                    "--output=cat",
                    "--quiet",
                    "--lines",
                    &options.lines.to_string(),
                ],
            );
            if options.follow {
                command.args.push("--follow".into());
            }
            Ok(Source::Journal(command))
        }
        #[cfg(any(target_os = "macos", test))]
        Manager::Launchd { .. } => {
            let _ = options;
            Ok(Source::File(paths.config.join("daemon.log")))
        }
        #[cfg(any(windows, test))]
        Manager::TaskScheduler { .. } | Manager::WindowsService => {
            let _ = options;
            Ok(Source::File(paths.config.join("daemon.log")))
        }
    }
}

pub async fn logs(paths: &Paths, options: LogOptions) -> Result<()> {
    let source = source(paths, &plan(paths)?, options)?;
    let result = match source {
        #[cfg(target_os = "linux")]
        Source::Journal(command) if options.follow => {
            follow_journal(&command, tokio::signal::ctrl_c()).await
        }
        #[cfg(target_os = "linux")]
        Source::Journal(command) => {
            journal_snapshot(&command, &SystemRunner, &mut std::io::stdout()).await
        }
        #[cfg(any(target_os = "macos", windows))]
        Source::File(path) => file_logs(&path, options).await,
        // Other platform implementations are compiled for unit tests only.
        #[cfg(test)]
        _ => unreachable!("the native service plan selects the native log source"),
    };
    // Closing a downstream pipe is a normal way to stop consuming logs.
    match result {
        Err(error)
            if error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|error| error.kind() == std::io::ErrorKind::BrokenPipe) =>
        {
            Ok(())
        }
        result => result,
    }
}

#[cfg(any(target_os = "linux", test))]
async fn journal_snapshot(
    command: &Invocation,
    runner: &impl Runner,
    output: &mut impl Write,
) -> Result<()> {
    let result = runner.output(command).await?;
    result.check(command)?;
    if result.stdout.is_empty() {
        tracing::info!("暂无可读取的服务日志");
    } else {
        output.write_all(result.stdout.as_bytes())?;
        output.flush()?;
    }
    Ok(())
}

#[cfg(target_os = "linux")]
async fn follow_journal(
    command: &Invocation,
    stop: impl std::future::Future<Output = std::io::Result<()>>,
) -> Result<()> {
    use std::process::Stdio;
    let mut child = tokio::process::Command::new(&command.program)
        .args(&command.args)
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .kill_on_drop(true)
        .spawn()
        .context("无法读取 journal 日志，请确认 journalctl 可用且当前用户有读取权限")?;
    tokio::select! {
        result = child.wait() => {
            let status = result?;
            #[cfg(unix)]
            {
                use std::os::unix::process::ExitStatusExt;
                if matches!(status.signal(), Some(libc::SIGINT | libc::SIGPIPE)) { return Ok(()); }
            }
            ensure!(status.success() || status.code() == Some(130), "journalctl 执行失败：{status}");
            Ok(())
        },
        signal = stop => {
            signal.context("无法监听 Ctrl+C")?;
            child.start_kill()?;
            child.wait().await?;
            Ok(())
        }
    }
}

#[cfg(any(target_os = "macos", windows))]
use file::file_logs;
#[cfg(any(target_os = "macos", windows, test))]
mod file;

#[cfg(test)]
mod tests;
