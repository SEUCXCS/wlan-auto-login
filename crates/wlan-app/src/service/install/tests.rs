use super::*;
#[cfg(unix)]
use crate::service::{command::Invocation, definition::LABEL, manager::Manager};

#[cfg(unix)]
#[tokio::test]
async fn failed_registration_restores_the_previous_service_and_binary() -> Result<()> {
    let scopes = if cfg!(target_os = "linux") {
        vec![crate::scope::Scope::User, crate::scope::Scope::System]
    } else {
        vec![crate::scope::Scope::User]
    };
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
                system: scope == crate::scope::Scope::System,
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
        assert!(install_plan(&paths, plan, &source).await.is_err());
        assert_eq!(fs::read_to_string(executable)?, "old executable");
        assert_eq!(fs::read(definition)?, fs::read(original)?);
    }
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn failed_first_install_removes_new_artifacts() -> Result<()> {
    let scopes = if cfg!(target_os = "linux") {
        vec![crate::scope::Scope::User, crate::scope::Scope::System]
    } else {
        vec![crate::scope::Scope::User]
    };
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
                system: scope == crate::scope::Scope::System,
            },
            executable: executable.clone(),
            definition: definition.clone(),
            content: format!("# {LABEL}"),
            stop: vec![],
            remove: vec![],
            reload: vec![],
            start: vec![Invocation::new("false", std::iter::empty::<&str>())],
        };
        assert!(install_plan(&paths, plan, &source).await.is_err());
        assert!(!executable.exists());
        assert!(!definition.exists());
    }
    Ok(())
}

#[test]
fn backups_restore_existing_files_and_remove_new_files() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let file = dir.path().join("file");
    let absent = Backup::capture(&file)?;
    fs::write(&file, "original")?;
    let backup = Backup::capture(&file)?;
    fs::write(&file, "replacement")?;
    backup.restore(&file)?;
    assert_eq!(fs::read_to_string(&file)?, "original");
    absent.restore(&file)?;
    assert!(!file.exists());
    Ok(())
}
