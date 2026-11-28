//! Validate Windows task XML and preserve native SCM or task registration.
use crate::infra::{
    command::Runner,
    error::{Context, Error, Message, Result},
    filesystem::private_dir,
};
pub(crate) use crate::system::shared::flow::install::{run, validate_scope};
use crate::system::{
    define::services::Manager,
    shared::{
        flow::{
            hooks::Hooks,
            install::{Backend, FilePayload, InstallContext, Step},
        },
        services::steps::execute_optional,
    },
    windows::services::{artifacts, manager::ScmRegistration, task, wait},
};
#[cfg(test)]
use crate::{infra::paths::Paths, system::define::services::Plan};
use std::{fs, path::PathBuf};

pub(crate) struct NativeHooks;
pub(crate) enum Registration {
    Task(task::Snapshot),
    Service(ScmRegistration),
}
impl<R: Runner> Hooks<Step, InstallContext<'_, R>> for NativeHooks {
    async fn after(
        &self,
        step: Step,
        context: &InstallContext<'_, R>,
        result: std::result::Result<(), &Error>,
    ) -> Result<()> {
        if result.is_ok() && matches!(step, Step::StopPrevious | Step::StopFailed) {
            let plan = if step == Step::StopPrevious {
                context.previous
            } else {
                context.plan
            };
            wait::wait_task_stopped(&plan.manager, context.runner).await?;
        }
        Ok(())
    }
}

impl<R: Runner> Backend<R> for NativeHooks {
    type Staged = FilePayload;
    type Registration = Registration;

    async fn stage(&self, context: &InstallContext<'_, R>) -> Result<FilePayload> {
        let plan = context.plan;
        let bin = plan
            .executable
            .parent()
            .context(Message::InstallationPathHasNoParentDirectory)?;
        let definition = plan
            .definition
            .parent()
            .context(Message::ServiceDefinitionHasNoParentDirectory)?;
        private_dir(bin)?;
        fs::create_dir_all(definition)?;
        if context.paths.scope == crate::infra::scope::Scope::System {
            crate::infra::filesystem::restrict_windows(bin, true)?;
            crate::infra::filesystem::restrict_windows(definition, true)?;
        }
        let staged = FilePayload::stage(plan, context.source, &artifacts::definition_bytes(plan))?;
        if matches!(plan.manager, Manager::TaskScheduler { .. }) {
            task::validate(staged.definition()?, context.runner).await?;
        }
        Ok(staged)
    }

    async fn snapshot(&self, context: &InstallContext<'_, R>) -> Result<Registration> {
        match &context.previous.manager {
            Manager::TaskScheduler { name } => Ok(Registration::Task(
                task::capture(name, context.runner).await?,
            )),
            _ => Ok(Registration::Service(
                ScmRegistration::capture(context.runner).await?,
            )),
        }
    }

    async fn remove_failed(&self, context: &InstallContext<'_, R>) -> Result<()> {
        if let Manager::TaskScheduler { name } = &context.plan.manager {
            task::remove(name, context.runner).await?;
        } else {
            for command in &context.plan.remove {
                execute_optional(context.runner, command).await?;
            }
        }
        Ok(())
    }

    async fn verify(&self, context: &InstallContext<'_, R>) -> Result<bool> {
        if matches!(context.plan.manager, Manager::TaskScheduler { .. }) {
            wait::wait_task_started(&context.plan.manager, context.runner).await?;
        }
        Ok(true)
    }

    async fn restore_registration(
        &self,
        context: &InstallContext<'_, R>,
        registration: &Registration,
    ) -> Result<()> {
        match (registration, &context.previous.manager) {
            (Registration::Task(snapshot), Manager::TaskScheduler { name }) => {
                snapshot
                    .restore(name, &context.previous.definition, context.runner)
                    .await
            }
            (Registration::Service(snapshot), _) => {
                snapshot.restore(context.previous, context.runner).await
            }
            _ => crate::infra::error::bail!(
                "Task registration snapshot does not match the service plan"
            ),
        }
    }

    fn retain_registration(
        &self,
        context: &InstallContext<'_, R>,
        registration: &Registration,
    ) -> Result<Option<PathBuf>> {
        match registration {
            Registration::Task(snapshot) => snapshot.retain(&context.previous.definition),
            Registration::Service(_) => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::command::{Invocation, SystemRunner};

    async fn install_plan(
        paths: &Paths,
        plan: Plan,
        source: &std::path::Path,
        runner: &impl Runner,
    ) -> Result<()> {
        crate::system::shared::flow::install::run_plan(
            &InstallContext {
                paths,
                plan: &plan,
                previous: &plan,
                source,
                runner,
            },
            &NativeHooks,
        )
        .await
    }
    use crate::infra::command::{CommandOutput, tests::FakeRunner};
    const ABSENT: &str = "Installed=0";
    const READY: &str = "Installed=1
State=3
Enabled=1
LastTaskResult=0";
    fn runner(outputs: &[(bool, &str)]) -> FakeRunner {
        let runner = FakeRunner::default();
        runner
            .outputs
            .borrow_mut()
            .extend(outputs.iter().map(|(success, text)| CommandOutput {
                success: *success,
                stdout: (*text).into(),
                stderr: String::new(),
            }));
        runner
    }
    fn fixture(residue: bool) -> Result<(tempfile::TempDir, Paths, Plan, std::path::PathBuf)> {
        let dir = tempfile::tempdir()?;
        let paths = Paths::test(dir.path().join("config"));
        std::fs::create_dir_all(&paths.config)?;
        let binary = dir.path().join("bin/daemon.exe");
        std::fs::create_dir_all(binary.parent().context("binary parent")?)?;
        let plan = crate::system::windows::services::definition::windows_plan(
            binary,
            paths.config.join("task.xml"),
            "S-1-5-21-123",
        )?;
        if residue {
            std::fs::write(&plan.executable, b"old binary")?;
            std::fs::write(
                &plan.definition,
                format!("{} stale disk definition", crate::infra::identity::APP_ID),
            )?;
        }
        let source = dir.path().join("source.exe");
        std::fs::write(&source, b"new binary")?;
        Ok((dir, paths, plan, source))
    }
    fn command_count(runner: &FakeRunner, verb: &str) -> usize {
        runner
            .calls
            .borrow()
            .iter()
            .filter(|command| command.args.first().is_some_and(|arg| arg == verb))
            .count()
    }
    #[tokio::test]
    async fn invalid_xml_or_snapshot_failure_never_stops_the_old_task() -> Result<()> {
        for outputs in [
            vec![(false, "invalid XML")],
            vec![(true, ""), (false, "query denied")],
        ] {
            let (_dir, paths, plan, source) = fixture(true)?;
            let binary = plan.executable.clone();
            let definition = plan.definition.clone();
            let before = std::fs::read(&definition)?;
            let runner = runner(&outputs);
            assert!(install_plan(&paths, plan, &source, &runner).await.is_err());
            assert_eq!(std::fs::read(binary)?, b"old binary");
            assert_eq!(std::fs::read(definition)?, before);
            assert_eq!(command_count(&runner, "/End"), 0);
        }
        Ok(())
    }
    #[tokio::test]
    async fn failed_registration_with_no_task_rolls_back_even_with_stale_files() -> Result<()> {
        for residue in [false, true] {
            let (_dir, paths, plan, source) = fixture(residue)?;
            let binary = plan.executable.clone();
            let definition = plan.definition.clone();
            let before = std::fs::read(&definition).ok();
            let runner = runner(&[
                (true, ""),
                (true, ABSENT),
                (true, ""),
                (true, ABSENT),
                (false, "registration failed"),
                (true, ""),
                (true, ABSENT),
                (true, ABSENT),
            ]);
            let error = install_plan(&paths, plan, &source, &runner)
                .await
                .err()
                .context("expected failure")?;
            assert!(!error.to_string().contains("rollback was incomplete"));
            assert!(error.to_string().contains("registration failed"));
            assert_eq!(std::fs::read(&definition).ok(), before);
            assert_eq!(binary.exists(), residue);
            assert_eq!(command_count(&runner, "/Delete"), 0);
            assert_eq!(command_count(&runner, "/Create"), 1);
            assert_eq!(command_count(&runner, "/Run"), 0);
        }
        Ok(())
    }
    #[tokio::test]
    async fn failed_start_removes_new_registration_before_restoring_files() -> Result<()> {
        let (_dir, paths, plan, source) = fixture(false)?;
        let binary = plan.executable.clone();
        let definition = plan.definition.clone();
        let runner = runner(&[
            (true, ""),
            (true, ABSENT),
            (true, ""),
            (true, ABSENT),
            (true, ""),
            (false, "start failed"),
            (true, ""),
            (true, READY),
            (true, READY),
            (true, ""),
            (true, ABSENT),
        ]);
        assert!(install_plan(&paths, plan, &source, &runner).await.is_err());
        assert!(!binary.exists());
        assert!(!definition.exists());
        assert_eq!(command_count(&runner, "/Delete"), 1);
        Ok(())
    }
    #[tokio::test]
    async fn failed_unregister_retains_published_files_and_old_backups() -> Result<()> {
        let (dir, paths, plan, source) = fixture(true)?;
        let binary = plan.executable.clone();
        let runner = runner(&[
            (true, ""),
            (true, ABSENT),
            (true, ""),
            (true, ABSENT),
            (true, ""),
            (false, "start failed"),
            (true, ""),
            (true, READY),
            (true, READY),
            (false, "delete denied"),
            (true, READY),
        ]);
        let error = install_plan(&paths, plan, &source, &runner)
            .await
            .err()
            .context("expected failure")?;
        assert!(error.to_string().contains("recovery files"));
        assert!(error.to_string().contains("delete denied"));
        assert_eq!(std::fs::read(binary)?, b"new binary");
        let backups = std::fs::read_dir(dir.path().join("bin"))?
            .filter_map(std::result::Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().starts_with(".tmp"))
            .map(|entry| std::fs::read(entry.path()))
            .collect::<std::io::Result<Vec<_>>>()?;
        assert!(backups.iter().any(|bytes| bytes == b"old binary"));
        Ok(())
    }
    #[tokio::test]
    async fn stopped_registered_task_is_restored_without_starting_it() -> Result<()> {
        let (_dir, paths, plan, source) = fixture(true)?;
        let binary = plan.executable.clone();
        let definition = plan.definition.clone();
        let before = std::fs::read(&definition)?;
        let native = plan.content.clone();
        let runner = runner(&[
            (true, ""),
            (true, READY),
            (true, &native),
            (true, ""),
            (true, READY),
            (false, "registration failed"),
            (true, ""),
            (true, READY),
            (true, READY),
            (true, ""),
            (true, ABSENT),
            (true, ""),
            (true, READY),
        ]);
        let error = install_plan(&paths, plan, &source, &runner)
            .await
            .err()
            .context("expected failure")?;
        assert!(!error.to_string().contains("rollback was incomplete"));
        assert_eq!(std::fs::read(binary)?, b"old binary");
        assert_eq!(std::fs::read(definition)?, before);
        assert_eq!(command_count(&runner, "/Run"), 0);
        assert_eq!(command_count(&runner, "/Create"), 2);
        Ok(())
    }
    #[tokio::test]
    async fn successful_install_checks_liveness_and_publishes_utf16() -> Result<()> {
        let (_dir, paths, plan, source) = fixture(false)?;
        let binary = plan.executable.clone();
        let definition = plan.definition.clone();
        let runner = runner(&[
            (true, ""),
            (true, ABSENT),
            (true, ""),
            (true, ABSENT),
            (true, ""),
            (true, ""),
            (
                true,
                "Installed=1
State=4
Enabled=1",
            ),
        ]);
        install_plan(&paths, plan, &source, &runner).await?;
        assert_eq!(std::fs::read(binary)?, b"new binary");
        assert!(std::fs::read(definition)?.starts_with(&[0xff, 0xfe]));
        assert_eq!(command_count(&runner, "/Delete"), 0);
        assert!(runner.outputs.borrow().is_empty());
        Ok(())
    }
    #[tokio::test]
    async fn failed_native_restore_retains_the_registration_xml_for_recovery() -> Result<()> {
        let (_dir, paths, plan, source) = fixture(true)?;
        let native = plan
            .content
            .replace("CXCS WLAN Login", "original registered task");
        let runner = runner(&[
            (true, ""),
            (true, READY),
            (true, &native),
            (true, ""),
            (true, READY),
            (false, "registration failed"),
            (true, ""),
            (true, READY),
            (true, READY),
            (true, ""),
            (true, ABSENT),
            (false, "restore denied"),
        ]);
        let error = install_plan(&paths, plan, &source, &runner)
            .await
            .err()
            .context("expected failure")?;
        assert!(error.to_string().contains("restore denied"));
        let recovered = std::fs::read_dir(&paths.config)?
            .filter_map(std::result::Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().starts_with(".tmp"))
            .filter_map(|entry| {
                crate::system::windows::services::artifacts::read_definition(&entry.path()).ok()
            })
            .collect::<Vec<_>>();
        assert!(recovered.iter().any(|xml| xml == &native));
        Ok(())
    }
    #[tokio::test]
    async fn real_preflight_can_read_the_staged_file_before_any_native_mutation() -> Result<()> {
        struct ValidateOnly {
            commands: FakeRunner,
        }
        impl Runner for ValidateOnly {
            async fn output(&self, command: &Invocation) -> Result<CommandOutput> {
                if command.program == "powershell.exe"
                    && command.args.last().is_some_and(|script| {
                        script.to_string_lossy().contains("RegisterTask($null")
                    })
                {
                    SystemRunner.output(command).await
                } else {
                    self.commands.output(command).await
                }
            }
        }
        let (_dir, paths, mut plan, source) = fixture(true)?;
        plan.content = plan
            .content
            .replace("S-1-5-21-123", &crate::infra::scope::current_sid()?);
        let binary = plan.executable.clone();
        let runner = ValidateOnly {
            commands: runner(&[(false, "snapshot sentinel")]),
        };
        let error = install_plan(&paths, plan, &source, &runner)
            .await
            .err()
            .context("expected snapshot sentinel")?;
        assert!(error.to_string().contains("snapshot sentinel"), "{error}");
        assert_eq!(std::fs::read(binary)?, b"old binary");
        assert_eq!(command_count(&runner.commands, "/End"), 0);
        Ok(())
    }
}
