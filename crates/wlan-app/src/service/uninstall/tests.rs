use super::*;
use crate::service::{
    definition,
    testing::{FakeRunner, RUNNING, STOPPED},
};
use std::{cell::Cell, fs, path::Path};

fn fixture(root: &Path, kind: usize) -> Result<(Paths, Plan)> {
    let base = root.join("cxxy-wlanlogin");
    let mut paths = Paths::test(base.join("state"));
    paths.runtime = base.join("run");
    paths.secrets = base.join("secrets");
    let executable = base.join("bin/daemon");
    let definition = root.join("shared-services/service");
    let log = paths.config.join("daemon.log");
    let plan = match kind {
        0 => definition::linux_plan(executable, definition)?,
        1 => definition::linux_system_plan(executable, definition)?,
        2 => definition::macos_plan(executable, definition, log, 501)?,
        3 => definition::macos_system_plan(executable, definition, log)?,
        4 => definition::windows_plan(executable, definition, "S-1-5-21-123")?,
        _ => definition::windows_system_plan(executable, definition)?,
    };
    paths.scope = plan.scope;
    for path in [
        &paths.config,
        &paths.runtime,
        &paths.secrets,
        plan.executable.parent().context("binary parent")?,
        plan.definition.parent().context("service parent")?,
    ] {
        fs::create_dir_all(path)?;
    }
    fs::write(&plan.definition, &plan.content)?;
    fs::write(&plan.executable, "installed copy, never executed")?;
    for name in ["preferences.json", "control.json", "daemon.log"] {
        fs::write(paths.config.join(name), "test data")?;
    }
    for name in ["daemon-endpoint.json", "daemon.sock"] {
        fs::write(paths.runtime.join(name), "test data")?;
    }
    fs::write(paths.secrets.join("login.cred"), "encrypted test secret")?;
    fs::write(root.join("shared-services/another-service"), "preserve")?;
    fs::write(root.join("original-cli"), "preserve")?;
    Ok((paths, plan))
}

fn removal_runner(kind: usize) -> FakeRunner {
    FakeRunner::new(match kind {
        0 | 1 => vec![
            (true, STOPPED),
            (true, ""),
            (true, ""),
            (true, "LoadState=not-found\n"),
        ],
        2 | 3 => vec![
            (true, "job = {\nstate = not running\n}\n"),
            (true, ""),
            (false, "missing"),
            (true, "domain = {}"),
        ],
        4 => vec![
            (true, "Installed=1\nState=3\n"),
            (true, ""),
            (true, ""),
            (true, "Installed=0\n"),
        ],
        _ => vec![
            (true, "Installed=1\nState=1\n"),
            (true, ""),
            (true, ""),
            (true, "Installed=0\n"),
        ],
    })
}

#[tokio::test]
async fn purge_cleans_each_platform_and_scope_without_removing_shared_files() -> Result<()> {
    for kind in 0..6 {
        let dir = tempfile::tempdir()?;
        let (paths, plan) = fixture(dir.path(), kind)?;
        let runner = removal_runner(kind);
        let deleted = Cell::new(false);
        uninstall_plan(&paths, &plan, true, &runner, async {
            deleted.set(true);
            cleanup::remove_file(&paths.secrets.join("login.cred"))
        })
        .await?;
        assert!(deleted.get());
        assert!(!plan.definition.exists() && !plan.executable.exists());
        assert!(!paths.config.exists() && !paths.runtime.exists() && !paths.secrets.exists());
        assert!(!dir.path().join("cxxy-wlanlogin").exists());
        assert_eq!(
            fs::read_to_string(dir.path().join("shared-services/another-service"))?,
            "preserve"
        );
        assert!(dir.path().join("original-cli").exists());
        assert!(
            runner.outputs.borrow().is_empty(),
            "all service checks must run"
        );
    }
    Ok(())
}

#[tokio::test]
async fn ordinary_uninstall_preserves_credentials_logs_and_preferences() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let (paths, plan) = fixture(dir.path(), 0)?;
    uninstall_plan(&paths, &plan, false, &removal_runner(0), async {
        panic!("ordinary uninstall must not delete credentials")
    })
    .await?;
    assert!(!plan.definition.exists() && !plan.executable.exists());
    assert!(paths.secrets.join("login.cred").exists());
    for name in ["preferences.json", "control.json", "daemon.log"] {
        assert!(paths.config.join(name).exists());
    }
    Ok(())
}

#[tokio::test]
async fn stale_registration_is_removed_even_without_a_definition_file() -> Result<()> {
    for kind in 0..6 {
        let dir = tempfile::tempdir()?;
        let (paths, plan) = fixture(dir.path(), kind)?;
        fs::remove_file(&plan.definition)?;
        uninstall_plan(&paths, &plan, false, &removal_runner(kind), async {
            Ok(())
        })
        .await?;
        assert!(!plan.executable.exists());
    }
    Ok(())
}

#[tokio::test]
async fn failed_unregister_preserves_files_and_never_deletes_credentials() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let (paths, plan) = fixture(dir.path(), 0)?;
    let runner = FakeRunner::new([(true, STOPPED), (false, "permission denied")]);
    assert!(
        uninstall_plan(&paths, &plan, true, &runner, async {
            panic!("credentials cannot be deleted after failed unregister")
        })
        .await
        .is_err()
    );
    assert!(plan.definition.exists() && plan.executable.exists());
    assert!(paths.secrets.join("login.cred").exists());
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn purge_waits_for_running_service_before_unregistering_or_deleting() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let (paths, plan) = fixture(dir.path(), 0)?;
    let runner = FakeRunner::new([
        (true, RUNNING),
        (true, ""), // stop
        (true, RUNNING),
        (true, STOPPED),
        (true, ""), // disable
        (true, ""), // reload
        (true, "LoadState=not-found\n"),
    ]);
    uninstall_plan(&paths, &plan, true, &runner, async {
        assert_eq!(runner.calls.borrow().len(), 7);
        assert!(!plan.definition.exists());
        assert!(plan.executable.exists());
        cleanup::remove_file(&paths.secrets.join("login.cred"))
    })
    .await?;
    let calls = runner.calls.borrow();
    assert_eq!(calls[1].args[1], "stop");
    assert_eq!(calls[4].args[1], "disable");
    assert!(!paths.config.exists());
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn failed_stop_never_deletes_service_files_or_credentials() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let (paths, plan) = fixture(dir.path(), 0)?;
    // An optional stop command can fail, but its outcome must be verified.
    let runner = FakeRunner::new(
        [(true, RUNNING), (false, "stop denied")]
            .into_iter()
            .chain(std::iter::repeat_n((true, RUNNING), 130)),
    );
    let result = uninstall_plan(&paths, &plan, true, &runner, async {
        panic!("a running service must retain its credentials")
    })
    .await;
    assert!(
        format!("{:#}", result.err().context("expected stop timeout")?)
            .contains("等待服务停止超时")
    );
    assert!(plan.definition.exists() && plan.executable.exists());
    assert!(paths.secrets.join("login.cred").exists());
    assert!(paths.config.join("daemon.log").exists());
    assert!(
        runner
            .calls
            .borrow()
            .iter()
            .all(|call| { !call.args.iter().any(|argument| argument == "disable") })
    );
    Ok(())
}

#[tokio::test]
async fn credential_failure_retains_data_and_can_be_retried_after_service_removal() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let (paths, plan) = fixture(dir.path(), 0)?;
    let result = uninstall_plan(&paths, &plan, true, &removal_runner(0), async {
        anyhow::bail!("keyring unavailable")
    })
    .await;
    assert!(
        format!("{:#}", result.err().context("expected credential failure")?)
            .contains("keyring unavailable")
    );
    assert!(!plan.definition.exists());
    assert!(plan.executable.exists());
    assert!(paths.config.join("daemon.log").exists());
    let runner = FakeRunner::new([
        (true, "LoadState=not-found\n"),
        (true, ""),
        (true, "LoadState=not-found\n"),
    ]);
    uninstall_plan(&paths, &plan, true, &runner, async {
        cleanup::remove_file(&paths.secrets.join("login.cred"))
    })
    .await?;
    assert!(!plan.executable.exists());
    assert!(!dir.path().join("cxxy-wlanlogin").exists());
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn purge_removes_only_owned_dangling_systemd_enable_links() -> Result<()> {
    for (kind, directory) in [(0, "default.target.wants"), (1, "multi-user.target.wants")] {
        let dir = tempfile::tempdir()?;
        let (paths, plan) = fixture(dir.path(), kind)?;
        let wants = plan
            .definition
            .parent()
            .context("definition parent")?
            .join(directory);
        fs::create_dir(&wants)?;
        let link = wants.join("service");
        fs::remove_file(&plan.definition)?;
        std::os::unix::fs::symlink("../service", &link)?;
        let runner = FakeRunner::new([
            (true, "LoadState=not-found\n"),
            (true, ""),
            (true, "LoadState=not-found\n"),
        ]);
        uninstall_plan(&paths, &plan, true, &runner, async {
            cleanup::remove_file(&paths.secrets.join("login.cred"))
        })
        .await?;
        assert!(fs::symlink_metadata(&link).is_err());

        std::os::unix::fs::symlink("../another-service", &link)?;
        assert!(remove_enable_link(&plan).is_err());
        assert!(fs::symlink_metadata(&link).is_ok());
        assert_eq!(
            fs::read_to_string(wants.join("../another-service"))?,
            "preserve"
        );
    }
    Ok(())
}

#[tokio::test]
async fn purge_is_repeatable_and_preserves_unknown_files_in_app_directories() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let (paths, plan) = fixture(dir.path(), 0)?;
    fs::write(paths.config.join("personal-note"), "preserve")?;
    uninstall_plan(&paths, &plan, true, &removal_runner(0), async {
        cleanup::remove_file(&paths.secrets.join("login.cred"))
    })
    .await?;
    let runner = FakeRunner::new([
        (true, "LoadState=not-found\n"),
        (true, ""),
        (true, "LoadState=not-found\n"),
    ]);
    uninstall_plan(&paths, &plan, true, &runner, async { Ok(()) }).await?;
    assert_eq!(
        fs::read_to_string(paths.config.join("personal-note"))?,
        "preserve"
    );
    assert!(!paths.config.join("daemon.log").exists());
    assert!(!paths.runtime.exists());
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn purge_rejects_redirected_data_roots_before_touching_services() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let (mut paths, plan) = fixture(dir.path(), 0)?;
    let outside = dir.path().join("unrelated");
    fs::create_dir(&outside)?;
    fs::write(outside.join("daemon.log"), "preserve")?;
    let link = dir.path().join("redirect");
    std::os::unix::fs::symlink(&outside, &link)?;
    paths.config = link;
    let runner = FakeRunner::default();
    assert!(
        uninstall_plan(&paths, &plan, true, &runner, async { Ok(()) })
            .await
            .is_err()
    );
    assert!(runner.calls.borrow().is_empty());
    assert_eq!(fs::read_to_string(outside.join("daemon.log"))?, "preserve");
    assert!(plan.definition.exists());
    Ok(())
}
