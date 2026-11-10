//! Task Scheduler validation uses Unicode COM strings and never registers a task.
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
}
