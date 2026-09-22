use super::*;
use crate::service::testing::{FakeRunner, RUNNING, STOPPED};

#[test]
fn systemd_distinguishes_running_stopped_failed_and_missing() -> Result<()> {
    let running = parse_systemd(RUNNING)?;
    assert!(running.running);
    assert_eq!(running.pid, Some(123));
    let stopped = parse_systemd(STOPPED)?;
    assert!(stopped.registered);
    assert!(!stopped.running);
    let failed = parse_systemd(
        "LoadState=loaded\nActiveState=failed\nSubState=failed\nMainPID=0\nResult=exit-code",
    )?;
    assert!(!failed.running);
    assert!(
        failed
            .detail
            .is_some_and(|detail| detail.contains("exit-code"))
    );
    assert!(!parse_systemd("LoadState=not-found")?.registered);
    assert!(parse_systemd("unrecognized output").is_err());
    Ok(())
}

#[test]
fn launchd_reads_only_top_level_job_properties() -> Result<()> {
    let state = parse_launchd(
        "gui/501/example = {\nstate = running\npid = 987\nenvironment = {\nstate = stopped\npid = 0\n}\nlast exit code = 0\n}",
    )?;
    assert!(state.running);
    assert_eq!(state.pid, Some(987));
    assert!(!parse_launchd("job = {\nstate = not running\nlast exit code = 1\n}")?.running);
    assert!(parse_launchd("unrecognized output").is_err());
    Ok(())
}

#[test]
fn windows_numeric_states_do_not_depend_on_system_language() -> Result<()> {
    for number in 1..=4 {
        let state = parse_windows(&format!(
            "Installed=1\r\nState={number}\r\nEnabled=1\r\nLastTaskResult=0\r\n"
        ))?;
        assert!(state.registered);
        assert_eq!(state.running, number == 4);
    }
    assert!(!parse_windows("Installed=0")?.registered);
    assert!(parse_windows("Installed=1\nState=0").is_err());
    assert!(parse_windows("Installed=1\nState=5").is_err());
    assert!(parse_windows("access denied").is_err());
    let command = windows_query("task'with-quote");
    assert_eq!(command.program, "powershell.exe");
    let script = command
        .args
        .last()
        .context("missing script")?
        .to_string_lossy();
    assert!(script.contains("task''with-quote"));
    assert!(!script.contains("ConvertTo-Json"));
    Ok(())
}

#[test]
fn scm_numeric_states_are_independent_of_locale() -> Result<()> {
    for number in 1..=7 {
        let state = parse_windows_service(&format!("Installed=1\nState={number}\nEnabled=1"))?;
        assert!(state.registered);
        assert_eq!(state.running, number == 4);
        assert_eq!(state.pending, matches!(number, 2 | 3 | 5 | 6));
    }
    assert!(!parse_windows_service("Installed=0")?.registered);
    assert!(parse_windows_service("Installed=1\nState=unknown").is_err());
    assert!(parse_windows_service("access denied").is_err());
    Ok(())
}

#[tokio::test]
async fn launchd_unloaded_job_requires_an_accessible_domain() -> Result<()> {
    let manager = Manager::Launchd {
        domain: "gui/501".into(),
        target: "gui/501/org.seucxcs.cxxy-wlanlogin".into(),
    };
    let runner = FakeRunner::new([
        (false, "missing job"),
        (true, "gui/501 = {\nservices = {\n}\n}"),
    ]);
    let state = manager.query(&runner).await?;
    assert!(!state.running);
    assert!(!state.registered);
    assert!(
        manager.installed(&state),
        "an unloaded plist remains installed"
    );
    let runner = FakeRunner::new([(false, "missing job"), (false, "missing domain")]);
    assert!(manager.query(&runner).await.is_err());
    Ok(())
}
