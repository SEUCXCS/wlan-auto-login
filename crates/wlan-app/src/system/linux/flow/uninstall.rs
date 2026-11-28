//! Remove only the selected systemd unit's autostart link.
use crate::infra::{
    cleanup,
    error::{Context, Error, Message, Result, ensure},
};
pub(crate) use crate::system::shared::flow::uninstall::run;
use crate::system::{
    define::services::{Manager, Plan},
    shared::flow::{
        hooks::Hooks,
        uninstall::{Step, UninstallContext, UninstallHooks},
    },
};

pub(crate) struct NativeHooks;
impl UninstallHooks for NativeHooks {}
impl Hooks<Step, UninstallContext<'_>> for NativeHooks {
    async fn after(
        &self,
        step: Step,
        context: &UninstallContext<'_>,
        result: std::result::Result<(), &Error>,
    ) -> Result<()> {
        if step == Step::Unregister && result.is_ok() {
            remove_enable_link(context.plan)?;
        }
        Ok(())
    }
}

fn remove_enable_link(plan: &Plan) -> Result<()> {
    if let Manager::Systemd { system } = &plan.manager {
        let parent = plan
            .definition
            .parent()
            .context(Message::ServiceDefinitionHasNoParentDirectory)?;
        let name = plan
            .definition
            .file_name()
            .context(Message::ServiceDefinitionHasNoFileName)?;
        let path = parent
            .join(if *system {
                "multi-user.target.wants"
            } else {
                "default.target.wants"
            })
            .join(name);
        cleanup::validate_directory(
            path.parent()
                .context(Message::ServiceLinkHasNoParentDirectory)?,
        )?;
        match std::fs::read_link(&path) {
            Ok(target) => {
                ensure!(
                    target == plan.definition || target == std::path::Path::new("..").join(name),
                    Message::RefusingToRemoveAnAutostartLinkOwned {
                        value0: (path.display()).to_string()
                    }
                );
                cleanup::remove_file(&path)?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error).context(Message::FailedToRemoveServiceAutostartLink);
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::command::tests::{FakeRunner, STOPPED};
    use crate::system::shared::flow::{
        tests::{fixture, removal_runner},
        uninstall::run_plan,
    };
    use std::fs;

    #[tokio::test]
    async fn purge_deletes_scoped_storage_without_parsing_control_state() -> Result<()> {
        for state in [
            r#"{"paused":false,"session":null,"revision":0}"#,
            "invalid json",
            r#"{"paused":false,"session":null,"revision":0,"credential_generation":18446744073709551615}"#,
        ] {
            for already_unregistered in [false, true] {
                let dir = tempfile::tempdir()?;
                let (paths, plan) = fixture(dir.path(), 1)?;
                fs::write(paths.config.join("control.json"), state)?;
                fs::write(paths.secrets.join("login.json"), "invalid credential data")?;
                let sibling = dir.path().join("SEUCXCS/another-tool");
                fs::create_dir(&sibling)?;
                fs::write(sibling.join("settings.json"), "preserve")?;
                let runner = if already_unregistered {
                    fs::remove_file(&plan.definition)?;
                    FakeRunner::new([
                        (true, "LoadState=not-found\n"),
                        (true, ""),
                        (true, "LoadState=not-found\n"),
                    ])
                } else {
                    removal_runner(1)
                };

                // Exercise the production storage selection with temporary system
                // files; service-manager operations remain entirely simulated.
                run_plan(
                    &paths,
                    &plan,
                    true,
                    &runner,
                    crate::system::shared::credentials::delete(&paths),
                    &NativeHooks,
                )
                .await?;

                assert!(!plan.definition.exists() && !plan.executable.exists());
                crate::system::shared::flow::tests::assert_purged(&paths)?;
                assert_eq!(
                    fs::read_to_string(sibling.join("settings.json"))?,
                    "preserve"
                );
                assert!(dir.path().join("original-cli").exists());
                assert!(runner.outputs.borrow().is_empty());
            }
        }
        Ok(())
    }

    #[tokio::test]
    async fn failed_unregister_never_polls_scoped_credential_deletion() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let (paths, plan) = fixture(dir.path(), 1)?;
        let state = "invalid json";
        fs::write(paths.config.join("control.json"), state)?;
        let runner = FakeRunner::new([(true, STOPPED), (false, "permission denied")]);

        assert!(
            run_plan(
                &paths,
                &plan,
                true,
                &runner,
                crate::system::shared::credentials::delete(&paths),
                &NativeHooks
            )
            .await
            .is_err()
        );
        assert_eq!(
            fs::read_to_string(paths.secrets.join("login.cred"))?,
            "encrypted test secret"
        );
        assert_eq!(
            fs::read_to_string(paths.config.join("control.json"))?,
            state
        );
        assert!(plan.definition.exists() && plan.executable.exists());
        Ok(())
    }

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
            run_plan(
                &paths,
                &plan,
                true,
                &runner,
                async { cleanup::remove_file(&paths.secrets.join("login.cred")) },
                &NativeHooks,
            )
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
}
