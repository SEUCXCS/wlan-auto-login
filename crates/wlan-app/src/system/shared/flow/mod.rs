//! Default workflows composed from shared steps and native adapters.
pub(crate) mod control;
pub(crate) mod credentials;
pub(crate) mod logs;
pub(crate) mod permission;
pub(crate) mod restart;
pub(crate) mod run;
pub(crate) mod start;
pub(crate) mod status;
pub(crate) mod stop;
#[cfg(target_os = "linux")]
#[path = "../../linux/flow/uninstall_default.rs"]
pub(crate) mod uninstall;
#[cfg(target_os = "macos")]
#[path = "../../macos/flow/uninstall_default.rs"]
pub(crate) mod uninstall;
#[cfg(target_os = "windows")]
#[path = "../../windows/flow/uninstall_default.rs"]
pub(crate) mod uninstall;
pub(crate) mod wifi;

#[cfg(test)]
pub(crate) mod tests {
    use super::uninstall::uninstall_plan;
    use crate::infra::command::tests::{FakeRunner, RUNNING, STOPPED};
    use crate::infra::{
        cleanup, control,
        error::{Context, Result},
        paths::Paths,
    };
    use crate::system::define::services::{Manager, Plan};
    use crate::system::shared::services::artifacts as definition;
    use std::{cell::Cell, fs, path::Path};

    const MAC_USER_ENABLED: &str =
        r#"{"version":1,"service":"dev.cxcs.tools.wlanlogin","scope":"user","status":"enabled"}"#;
    const MAC_SYSTEM_ENABLED: &str =
        r#"{"version":1,"service":"dev.cxcs.tools.wlanlogin","scope":"system","status":"enabled"}"#;
    const MAC_USER_REMOVED: &str = r#"{"version":1,"service":"dev.cxcs.tools.wlanlogin","scope":"user","status":"not_registered"}"#;
    const MAC_SYSTEM_REMOVED: &str = r#"{"version":1,"service":"dev.cxcs.tools.wlanlogin","scope":"system","status":"not_registered"}"#;

    pub(crate) struct FixtureHooks;
    impl crate::system::shared::services::hooks::UninstallHooks for FixtureHooks {
        fn packaged_definition(&self, plan: &Plan) -> bool {
            matches!(plan.manager, Manager::MacosAppService { .. })
        }
        fn remove_payload(&self, plan: &Plan) -> Result<()> {
            cleanup::remove_file(&plan.executable)?;
            if matches!(plan.manager, Manager::MacosAppService { .. }) {
                cleanup::remove_file(&plan.definition)?;
                cleanup::prune_empty_directories(
                    plan.definition.parent().context("definition parent")?,
                )?;
                cleanup::prune_empty_directories(
                    plan.executable.parent().context("executable parent")?,
                )?;
            }
            Ok(())
        }
    }

    pub(crate) fn fixture(root: &Path, kind: usize) -> Result<(Paths, Plan)> {
        let base = root.join("SEUCXCS/wlanlogin");
        let mut paths = Paths::test(base.join("state"));
        paths.runtime = base.join("run");
        paths.secrets = base.join("secrets");
        let executable = base.join("bin/daemon");
        let definition = root.join("shared-services/service");
        let plan = match kind {
            0 => definition::linux_plan(executable, definition)?,
            1 => definition::linux_system_plan(executable, definition)?,
            2 | 3 => definition::app_service_plan(
                base.join("wlanlogin.app/Contents/MacOS/wlanlogin"),
                kind == 3,
                501,
            )?,
            4 => definition::windows_plan(executable, definition, "S-1-5-21-123")?,
            _ => definition::windows_system_plan(executable, definition)?,
        };
        paths.scope = plan.scope;
        for path in [
            paths.config.as_path(),
            paths.runtime.as_path(),
            paths.secrets.as_path(),
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
        fs::create_dir_all(root.join("shared-services"))?;
        fs::write(root.join("shared-services/another-service"), "preserve")?;
        fs::write(root.join("original-cli"), "preserve")?;
        Ok((paths, plan))
    }

    pub(crate) fn removal_runner(kind: usize) -> FakeRunner {
        FakeRunner::new(match kind {
            0 | 1 => vec![
                (true, STOPPED),
                (true, ""),
                (true, ""),
                (true, "LoadState=not-found\n"),
            ],
            2 | 3 => vec![
                (
                    true,
                    if kind == 3 {
                        MAC_SYSTEM_ENABLED
                    } else {
                        MAC_USER_ENABLED
                    },
                ),
                (true, "job = {\nstate = not running\n}\n"),
                (true, ""),
                (
                    true,
                    if kind == 3 {
                        MAC_SYSTEM_REMOVED
                    } else {
                        MAC_USER_REMOVED
                    },
                ),
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
            let sibling = dir.path().join("SEUCXCS/another-tool");
            fs::create_dir(&sibling)?;
            fs::write(sibling.join("settings.json"), "preserve")?;
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
            assert!(!dir.path().join("SEUCXCS/wlanlogin").exists());
            assert_eq!(
                fs::read_to_string(sibling.join("settings.json"))?,
                "preserve"
            );
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
        for kind in [0, 1, 4, 5] {
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
        fs::write(paths.config.join("control.json"), "invalid control state")?;
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
                .contains("Timed out waiting for service stop")
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
    async fn credential_failure_retains_data_and_can_be_retried_after_service_removal() -> Result<()>
    {
        let dir = tempfile::tempdir()?;
        let (paths, plan) = fixture(dir.path(), 0)?;
        let result = uninstall_plan(&paths, &plan, true, &removal_runner(0), async {
            crate::infra::error::bail!("keyring unavailable")
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
        assert!(!dir.path().join("SEUCXCS/wlanlogin").exists());
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

    #[tokio::test]
    async fn purge_ignores_corrupt_control_state_and_deletes_only_after_unregistration()
    -> Result<()> {
        for state in [
            r#"{"paused":false,"session":null,"revision":0}"#,
            "invalid json",
            r#"{"paused":false,"session":null,"revision":0,"credential_generation":18446744073709551615}"#,
        ] {
            for already_unregistered in [false, true] {
                let dir = tempfile::tempdir()?;
                let (paths, plan) = fixture(dir.path(), 0)?;
                fs::write(paths.config.join("control.json"), state)?;
                let runner = if already_unregistered {
                    fs::remove_file(&plan.definition)?;
                    FakeRunner::new([
                        (true, "LoadState=not-found\n"),
                        (true, ""),
                        (true, "LoadState=not-found\n"),
                    ])
                } else {
                    removal_runner(0)
                };
                let deleted = Cell::new(false);
                uninstall_plan(&paths, &plan, true, &runner, async {
                    assert!(
                        runner.outputs.borrow().is_empty(),
                        "unregistration must finish before deletion"
                    );
                    assert!(
                        control::singleton(&paths).is_err(),
                        "daemon lock must protect deletion"
                    );
                    assert_eq!(
                        fs::read_to_string(paths.config.join("control.json"))?,
                        state
                    );
                    deleted.set(true);
                    cleanup::remove_file(&paths.secrets.join("login.cred"))
                })
                .await?;
                assert!(deleted.get());
                assert!(
                    !paths.config.exists() && !paths.runtime.exists() && !paths.secrets.exists()
                );
            }
        }
        Ok(())
    }

    #[tokio::test]
    async fn purge_rejects_redirected_data_roots_before_touching_services() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let (mut paths, plan) = fixture(dir.path(), 0)?;
        let outside = dir.path().join("unrelated");
        fs::create_dir(&outside)?;
        fs::write(outside.join("daemon.log"), "preserve")?;
        let link = dir.path().join("redirect");
        crate::infra::filesystem::tests::redirect_directory(&outside, &link)?;
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
}
