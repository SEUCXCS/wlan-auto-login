//! systemd registration and Unix installation permissions.
use crate::infra::{
    command::Runner,
    error::{Context, Message, Result},
    filesystem::private_dir,
    paths::Paths,
    scope::Scope,
};
use crate::system::{
    define::services::Plan,
    linux::services::manager::Registration,
    shared::{
        flow::{
            hooks::Hooks,
            install::{Backend, FilePayload, InstallContext, Step},
        },
        services::artifacts::plan,
    },
};
use std::{fs, os::unix::fs::PermissionsExt};

pub(crate) struct NativeHooks;
impl<R: Runner> Hooks<Step, InstallContext<'_, R>> for NativeHooks {}
impl<R: Runner> Backend<R> for NativeHooks {
    type Staged = FilePayload;
    type Registration = Registration;

    fn target_plan(&self, paths: &Paths) -> Result<Plan> {
        if paths.scope == Scope::System {
            crate::infra::error::ensure!(
                paths.secrets.join("login.cred").try_exists()?
                    || paths.secrets.join("login.json").try_exists()?,
                Message::SystemCredentialsAreMissingRunLoginSystemRemember
            );
        }
        plan(paths)
    }

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
        let mode = if context.paths.scope == Scope::System {
            0o755
        } else {
            0o700
        };
        if mode == 0o755 {
            fs::set_permissions(bin, fs::Permissions::from_mode(mode))?;
        }
        fs::create_dir_all(definition)?;
        let staged = FilePayload::stage(plan, context.source, plan.content.as_bytes())?;
        if let Some(executable) = staged.executable() {
            fs::set_permissions(executable, fs::Permissions::from_mode(mode))?;
        }
        fs::set_permissions(staged.definition()?, fs::Permissions::from_mode(0o644))?;
        Ok(staged)
    }

    async fn snapshot(&self, context: &InstallContext<'_, R>) -> Result<Registration> {
        Registration::capture(
            context.paths.scope == Scope::System,
            context.previous,
            context.runner,
        )
        .await
    }

    async fn restore_registration(
        &self,
        context: &InstallContext<'_, R>,
        registration: &Registration,
    ) -> Result<()> {
        registration
            .restore(
                context.paths.scope == Scope::System,
                context.previous,
                context.runner,
            )
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::infra::command::{
        Invocation,
        tests::{FakeRunner, STOPPED},
    };

    use crate::system::define::services::{LABEL, Manager};

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

    #[tokio::test]
    async fn failed_registration_restores_the_previous_service_and_binary() -> Result<()> {
        let scopes = [Scope::User, Scope::System];
        for scope in scopes {
            let dir = tempfile::tempdir()?;
            let mut paths = Paths::test(dir.path().join("config"));
            paths.scope = scope;
            let executable = dir.path().join("daemon");
            let definition = dir.path().join("service");
            let original = dir.path().join("expected");
            let source = dir.path().join("new-binary");
            fs::write(&executable, "old executable")?;
            fs::write(&definition, format!("# {LABEL} old service"))?;
            fs::copy(&definition, &original)?;
            fs::write(&source, "new executable")?;
            let plan = Plan {
                scope,
                manager: Manager::Systemd {
                    system: scope == crate::infra::scope::Scope::System,
                },
                executable: executable.clone(),
                definition: definition.clone(),
                content: format!("# {LABEL} new service"),
                stop: vec![],
                remove: vec![],
                reload: vec![],
                // Fails for the new definition, succeeds after the old one is restored.
                start: vec![Invocation::new(
                    "cmp",
                    [original.as_os_str(), definition.as_os_str()],
                )],
            };
            let stopped = "LoadState=loaded\nActiveState=inactive\nSubState=dead\nUnitFileState=disabled\nMainPID=0";
            let runner = FakeRunner::new([(true, stopped), (false, "failed registration")]);
            let result = install_plan(&paths, plan, &source, &runner).await;
            assert!(result.is_err());
            assert_eq!(
                runner.calls.borrow().len(),
                2,
                "a disabled stopped service stays stopped"
            );
            assert_eq!(fs::read_to_string(executable)?, "old executable");
            assert_eq!(fs::read(definition)?, fs::read(original)?);
        }
        Ok(())
    }

    #[tokio::test]
    async fn failed_first_install_removes_new_artifacts() -> Result<()> {
        let scopes = [Scope::User, Scope::System];
        for scope in scopes {
            let dir = tempfile::tempdir()?;
            let mut paths = Paths::test(dir.path().join("config"));
            let source = dir.path().join("source");
            fs::write(&source, "test binary, never executed")?;
            paths.scope = scope;
            let executable = dir.path().join("bin/daemon");
            let definition = dir.path().join("service");
            let plan = Plan {
                scope,
                manager: Manager::Systemd {
                    system: scope == crate::infra::scope::Scope::System,
                },
                executable: executable.clone(),
                definition: definition.clone(),
                content: format!("# {LABEL}"),
                stop: vec![],
                remove: vec![],
                reload: vec![],
                start: vec![Invocation::new("false", std::iter::empty::<&str>())],
            };
            let runner = FakeRunner::new([
                (true, "LoadState=not-found"),
                (false, "failed registration"),
            ]);
            assert!(install_plan(&paths, plan, &source, &runner).await.is_err());
            assert!(!executable.exists());
            assert!(!definition.exists());
        }
        Ok(())
    }

    #[tokio::test]
    async fn failed_rollback_stop_retains_new_files_and_previous_backups() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let paths = Paths::test(dir.path().join("state"));
        let executable = dir.path().join("daemon");
        let definition = dir.path().join("service");
        let expected = dir.path().join("old-definition");
        let source = dir.path().join("new-source");
        fs::write(&executable, "old binary")?;
        fs::write(&definition, format!("# {LABEL} old"))?;
        fs::copy(&definition, &expected)?;
        fs::write(&source, "new binary")?;
        let plan = Plan {
            scope: paths.scope,
            manager: Manager::Systemd { system: false },
            executable: executable.clone(),
            definition: definition.clone(),
            content: format!("# {LABEL} new"),
            // The first stop succeeds; after publication, stopping fails.
            stop: vec![Invocation::new(
                "cmp",
                [expected.as_os_str(), definition.as_os_str()],
            )],
            start: vec![Invocation::new("false", std::iter::empty::<&str>())],
            remove: Vec::new(),
            reload: Vec::new(),
        };
        let runner = FakeRunner::new([
            (true, STOPPED),
            (true, ""),
            (false, "failed registration"),
            (false, "failed stop"),
        ]);
        let error = install_plan(&paths, plan, &source, &runner)
            .await
            .err()
            .context("expected failed rollback")?;
        assert!(error.to_string().contains("recovery files"));
        assert_eq!(fs::read_to_string(executable)?, "new binary");
        assert_eq!(fs::read_to_string(definition)?, format!("# {LABEL} new"));
        let backups = fs::read_dir(dir.path())?
            .filter_map(std::result::Result::ok)
            .filter(|file| file.file_name().to_string_lossy().starts_with(".tmp"))
            .map(|file| fs::read_to_string(file.path()))
            .collect::<std::io::Result<Vec<_>>>()?;
        assert!(backups.iter().any(|data| data == "old binary"));
        assert!(backups.iter().any(|data| data == &format!("# {LABEL} old")));
        Ok(())
    }
}
