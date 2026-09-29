use crate::infra::{
    command::{Invocation, Runner},
    error::{Context, Message, Result, ensure},
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
pub(crate) async fn follow_journal(
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
        .context(Message::FailedToReadJournalCheckJournalctlAvailabilityAnd)?;
    tokio::select! {
        result = child.wait() => {
            let status = result?;
            #[cfg(unix)]
            {
                use std::os::unix::process::ExitStatusExt;
                if matches!(status.signal(), Some(libc::SIGINT | libc::SIGPIPE)) { return Ok(()); }
            }
            ensure!(status.success() || status.code() == Some(130), Message::JournalctlFailedStatus { status: (status).to_string() });
            Ok(())
        },
        signal = stop => {
            signal.context(Message::FailedToRegisterCtrlCListener)?;
            child.start_kill()?;
            child.wait().await?;
            Ok(())
        }
    }
}
