//! Task Scheduler validation, registration snapshots and idempotent removal.
use crate::infra::{
    command::{Invocation, Runner},
    error::Result,
};
use std::path::Path;
pub(crate) fn powershell(script: &str) -> Invocation {
    Invocation::new(
        "powershell.exe",
        [
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            script,
        ],
    )
}
pub(crate) async fn validate(path: &Path, runner: &impl Runner) -> Result<()> {
    let path = path
        .to_string_lossy()
        .replace(char::from(39), &char::from(39).to_string().repeat(2));
    let script = format!(
        r#"$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = New-Object System.Text.UTF8Encoding -ArgumentList $false
try {{
    $scheduler = New-Object -ComObject 'Schedule.Service'
    $scheduler.Connect()
    $folder = $scheduler.GetFolder([string][char]92)
    $xml = [System.IO.File]::ReadAllText('{path}')
    $null = $folder.RegisterTask($null, $xml, 1, $null, $null, 3, $null)
}} catch {{
    $cause=$_.Exception
    while ($null -ne $cause.InnerException) {{ $cause=$cause.InnerException }}
    [Console]::Error.WriteLine('Task XML validation failed (HRESULT=' + [System.Runtime.InteropServices.Marshal]::GetHRForException($cause) + '): ' + $cause.Message)
    exit 1
}}
"#
    );
    let command = powershell(&script);
    runner.output(&command).await?.check(&command)
}

pub(crate) struct Snapshot {
    state: crate::system::define::services::ServiceState,
    xml: Option<String>,
}
fn quoted(value: &str) -> String {
    let quote = char::from(39);
    format!(
        "{quote}{}{quote}",
        value.replace(quote, &quote.to_string().repeat(2))
    )
}
pub(crate) async fn capture(name: &str, runner: &impl Runner) -> Result<Snapshot> {
    let state = super::manager::query_task(name, runner).await?;
    let xml = if state.registered {
        crate::infra::error::ensure!(
            state.enabled.is_some(),
            "Task Scheduler returned an unknown enabled state"
        );
        let script = format!(
            r#"$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = New-Object System.Text.UTF8Encoding -ArgumentList $false
try {{
 $scheduler=New-Object -ComObject 'Schedule.Service'
 $scheduler.Connect()
 [Console]::Write($scheduler.GetFolder([string][char]92).GetTask({}).Xml)
}} catch {{ [Console]::Error.WriteLine($_.Exception.Message); exit 1 }}
"#,
            quoted(name)
        );
        let command = powershell(&script);
        let result = runner.output(&command).await?;
        result.check(&command)?;
        crate::infra::error::ensure!(
            !result.stdout.trim().is_empty(),
            "Task Scheduler returned an empty registration snapshot"
        );
        Some(result.stdout)
    } else {
        None
    };
    Ok(Snapshot { state, xml })
}
pub(crate) async fn remove(name: &str, runner: &impl Runner) -> Result<()> {
    if !super::manager::query_task(name, runner).await?.registered {
        return Ok(());
    }
    let command = Invocation::new("schtasks", ["/Delete", "/TN", name, "/F"]);
    let result = runner.output(&command).await?;
    if !result.success {
        if !super::manager::query_task(name, runner).await?.registered {
            return Ok(());
        }
        result.check(&command)?;
    }
    use crate::infra::error::Context;
    tokio::time::timeout(std::time::Duration::from_secs(25), async {
        loop {
            if !super::manager::query_task(name, runner).await?.registered {
                return Ok(());
            }
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }
    })
    .await
    .context("Timed out waiting for task removal; recovery files retained")?
}
impl Snapshot {
    pub(crate) async fn restore(
        &self,
        name: &str,
        definition: &Path,
        runner: &impl Runner,
    ) -> Result<()> {
        let Some(_) = &self.xml else {
            return Ok(());
        };
        use crate::infra::error::Context;
        use std::ffi::OsStr;
        let file = self
            .stage(definition)?
            .context("Task snapshot is missing")?;
        let command = Invocation::new(
            "schtasks",
            [
                OsStr::new("/Create"),
                OsStr::new("/TN"),
                OsStr::new(name),
                OsStr::new("/XML"),
                file.path().as_os_str(),
                OsStr::new("/F"),
            ],
        );
        runner.output(&command).await?.check(&command)?;
        let resume = self.state.running || self.state.pending;
        if resume {
            let command = Invocation::new("schtasks", ["/Run", "/TN", name]);
            runner.output(&command).await?.check(&command)?;
        }
        let state = if resume {
            super::wait::wait_task_started(
                &crate::system::define::services::Manager::TaskScheduler { name: name.into() },
                runner,
            )
            .await?
        } else {
            super::manager::query_task(name, runner).await?
        };
        crate::infra::error::ensure!(
            state.registered
                && state.enabled == self.state.enabled
                && (resume || (!state.running && !state.pending)),
            "Task registration state could not be restored"
        );
        Ok(())
    }
}

impl Snapshot {
    fn stage(&self, definition: &Path) -> Result<Option<tempfile::NamedTempFile>> {
        let Some(xml) = &self.xml else {
            return Ok(None);
        };
        use crate::infra::error::Context;
        use std::io::Write;
        let mut file = tempfile::NamedTempFile::new_in(
            definition
                .parent()
                .context("Task definition has no parent")?,
        )?;
        // Normalize only the declaration; path and argument text must remain byte-for-byte equivalent.
        let xml = if xml.starts_with("<?xml") {
            if let Some(end) = xml.find("?>") {
                format!("{}{}", xml[..end].replace("UTF-8", "UTF-16"), &xml[end..])
            } else {
                xml.clone()
            }
        } else {
            xml.clone()
        };
        file.write_all(&super::artifacts::encode_task_xml(&xml))?;
        file.as_file().sync_all()?;
        Ok(Some(file))
    }
    pub(crate) fn retain(&self, definition: &Path) -> Result<Option<std::path::PathBuf>> {
        self.stage(definition)?
            .map(|file| {
                file.keep()
                    .map(|(_, path)| path)
                    .map_err(|error| error.error.into())
            })
            .transpose()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(windows)]
    #[tokio::test]
    async fn native_validation_accepts_utf16_and_reproduces_the_utf8_failure() -> Result<()> {
        use crate::system::windows::services::{artifacts, definition};
        let dir = tempfile::tempdir()?;
        let sid = crate::infra::scope::current_sid()?;
        let plan = definition::windows_plan(
            dir.path().join("用户 & Test/daemon.exe"),
            dir.path().join("task.xml"),
            &sid,
        )?;
        std::fs::write(&plan.definition, artifacts::definition_bytes(&plan))?;
        validate(&plan.definition, &crate::infra::command::SystemRunner).await?;
        std::fs::write(&plan.definition, plan.content.replace("UTF-16", "UTF-8"))?;
        assert!(
            validate(&plan.definition, &crate::infra::command::SystemRunner)
                .await
                .is_err()
        );
        Ok(())
    }
    use crate::infra::error::Context;
    #[tokio::test]
    async fn missing_and_concurrently_removed_tasks_are_idempotent_but_denials_fail() -> Result<()>
    {
        use crate::infra::command::tests::FakeRunner;
        let absent = FakeRunner::new([(true, "Installed=0")]);
        remove("missing", &absent).await?;
        assert_eq!(absent.calls.borrow().len(), 1);
        let removed = FakeRunner::new([
            (
                true,
                "Installed=1
State=3
Enabled=1",
            ),
            (false, "file missing"),
            (true, "Installed=0"),
        ]);
        remove("raced", &removed).await?;
        let denied = FakeRunner::new([
            (
                true,
                "Installed=1
State=3
Enabled=1",
            ),
            (false, "access denied"),
            (
                true,
                "Installed=1
State=3
Enabled=1",
            ),
        ]);
        assert!(remove("denied", &denied).await.is_err());
        let unavailable = FakeRunner::new([(false, "query denied")]);
        assert!(remove("unknown", &unavailable).await.is_err());
        assert_eq!(unavailable.calls.borrow().len(), 1);
        Ok(())
    }
    #[tokio::test]
    async fn snapshots_preserve_disabled_state_and_resume_only_running_tasks() -> Result<()> {
        use crate::infra::command::tests::FakeRunner;
        use crate::system::define::services::ServiceState;
        for running in [false, true] {
            let dir = tempfile::tempdir()?;
            let definition = dir.path().join("task.xml");
            let snapshot=Snapshot{state:ServiceState{registered:true,running,pending:false,enabled:Some(running),state:"old".into(),pid:None,detail:None},xml:Some("<?xml version=\"1.0\" encoding=\"UTF-16\"?><Task><Command>C:/UTF-8/path.exe</Command></Task>".into())};
            let runner = if running {
                FakeRunner::new([
                    (true, ""),
                    (true, ""),
                    (
                        true,
                        "Installed=1
State=4
Enabled=1",
                    ),
                ])
            } else {
                FakeRunner::new([
                    (true, ""),
                    (
                        true,
                        "Installed=1
State=1
Enabled=0",
                    ),
                ])
            };
            snapshot.restore("test", &definition, &runner).await?;
            assert_eq!(
                runner
                    .calls
                    .borrow()
                    .iter()
                    .filter(|command| command.args.first().is_some_and(|arg| arg == "/Run"))
                    .count(),
                usize::from(running)
            );
            let recovery = snapshot
                .retain(&definition)?
                .context("snapshot recovery file")?;
            let xml = super::super::artifacts::read_definition(&recovery)?;
            assert!(xml.contains("C:/UTF-8/path.exe"));
            std::fs::remove_file(recovery)?;
        }
        Ok(())
    }
}
