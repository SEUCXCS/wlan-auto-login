use crate::infra::{
    command::{Invocation, Runner},
    error::{Message, Result},
    paths::Paths,
};
use crate::system::define::services::{LogOptions, LogSource, Plan, ServiceManager};
use std::io::Write;
pub(crate) fn source(paths: &Paths, plan: &Plan, options: LogOptions) -> Result<LogSource> {
    let system = plan.scope == crate::infra::scope::Scope::System;

    // Inspect the installed definition, not the newly generated plan:
    // older installations still send their application output to journal.
    if std::fs::read_to_string(&plan.definition)?
        .lines()
        .any(|line| line.trim() == "# ApplicationLog=daemon.log")
    {
        return Ok(LogSource::File(paths.config.join("daemon.log")));
    }
    let mut command = Invocation::new(
        "journalctl",
        [
            if system { "--system" } else { "--user" },
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
    Ok(LogSource::Journal(command))
}
pub(crate) async fn journal_snapshot(
    command: &Invocation,
    runner: &impl Runner,
    output: &mut impl Write,
) -> Result<()> {
    let result = runner.output(command).await?;
    result.check(command)?;
    if result.stdout.is_empty() {
        tracing::info!("{}", Message::NoServiceLogEntriesAvailable);
    } else {
        output.write_all(result.stdout.as_bytes())?;
        output.flush()?;
    }
    Ok(())
}
#[cfg(target_os = "linux")]
pub(crate) async fn follow_journal(
    command: &Invocation,
    output: &mut impl Write,
    stop: impl std::future::Future<Output = std::io::Result<()>>,
) -> Result<()> {
    use crate::infra::error::{Context, ensure};
    use std::process::Stdio;
    use tokio::io::AsyncReadExt;
    let mut child = tokio::process::Command::new(&command.program)
        .args(&command.args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .context(Message::FailedToReadJournalCheckJournalctlAvailabilityAnd)?;
    let mut stdout = child.stdout.take().context(Message::LogOutputUnavailable)?;
    let mut stderr = child.stderr.take().context(Message::LogOutputUnavailable)?;
    // Drain diagnostic output concurrently; never let a full pipe block the child.
    // JoinSet cancels this helper if output consumption fails or the caller cancels.
    let mut diagnostics = tokio::task::JoinSet::new();
    diagnostics.spawn(async move { tokio::io::copy(&mut stderr, &mut tokio::io::sink()).await });
    tokio::pin!(stop);
    let mut buffer = [0; 8192];
    loop {
        tokio::select! {
            signal = &mut stop => {
                signal.context(Message::FailedToRegisterCtrlCListener)?;
                child.start_kill()?;
                child.wait().await?;
                return Ok(());
            },
            read = stdout.read(&mut buffer) => {
                let count = read?;
                if count == 0 { break; }
                output.write_all(&buffer[..count])?;
                output.flush()?;
            },
        }
    }
    let status = tokio::select! {
        signal = &mut stop => {
            signal.context(Message::FailedToRegisterCtrlCListener)?;
            child.start_kill()?;
            child.wait().await?;
            return Ok(());
        },
        status = child.wait() => status?,
    };
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if matches!(status.signal(), Some(libc::SIGINT | libc::SIGPIPE)) {
            return Ok(());
        }
    }
    ensure!(
        status.success() || status.code() == Some(130),
        Message::JournalctlFailedStatus {
            status: status.to_string()
        }
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::command::tests::FakeRunner;
    #[tokio::test]
    #[cfg(target_os = "linux")]
    async fn journal_follow_cancels_the_child_and_reports_failures() -> Result<()> {
        let command = Invocation::new("sleep", ["30"]);
        tokio::time::timeout(
            std::time::Duration::from_secs(3),
            follow_journal(&command, &mut Vec::new(), std::future::ready(Ok(()))),
        )
        .await??;
        let command = Invocation::new("false", std::iter::empty::<&str>());
        assert!(
            follow_journal(&command, &mut Vec::new(), std::future::pending())
                .await
                .is_err()
        );
        Ok(())
    }

    #[tokio::test]
    async fn journal_keeps_original_messages_and_reports_access_failures() -> Result<()> {
        let command = Invocation::new("journalctl", ["--user"]);
        let runner = FakeRunner::new([(true, "[INFO 2026-09-22 10:30:00] 日志\n")]);
        let mut output = Vec::new();
        journal_snapshot(&command, &runner, &mut output).await?;
        assert_eq!(output, "[INFO 2026-09-22 10:30:00] 日志\n".as_bytes());
        let runner = FakeRunner::new([(false, "access denied")]);
        assert!(
            journal_snapshot(&command, &runner, &mut Vec::new())
                .await
                .is_err()
        );
        let runner = FakeRunner::new([(true, "")]);
        let mut output = Vec::new();
        journal_snapshot(&command, &runner, &mut output).await?;
        assert!(output.is_empty());
        Ok(())
    }

    #[tokio::test]
    #[cfg(target_os = "linux")]
    async fn followed_output_is_delivered_to_the_caller() -> Result<()> {
        let command = Invocation::new("printf", ["first line\nsecond line\n"]);
        let mut output = Vec::new();
        follow_journal(&command, &mut output, std::future::pending()).await?;
        assert_eq!(output, b"first line\nsecond line\n");
        Ok(())
    }
}
