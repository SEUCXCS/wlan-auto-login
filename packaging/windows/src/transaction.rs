//! MSI owns the payload and PATH; these actions compose the existing CLI operations.
use crate::{
    Scope, cli,
    diagnostics::Diagnostics,
    ipc, services,
    state::{self, Paths, Snapshot},
    validate_id, win,
};
use anyhow::{Context, Result, bail, ensure};
use fs2::FileExt;
use std::{
    fs::{self, File, OpenOptions},
    path::Path,
    time::{Duration, Instant},
};
use zeroize::Zeroizing;

pub fn dispatch(arguments: Vec<String>) -> Result<()> {
    let [action, scope, product, session, binary, log_directory] = arguments.as_slice() else {
        bail!("Invalid installer action arguments");
    };
    let scope = Scope::parse(scope)?;
    let diagnostics = Diagnostics::open(log_directory, product)?;
    diagnostics.stage(&format!("action={action} scope={scope:?} starting"))?;
    diagnostics.stage(&format!("process elevated={}", win::elevated()?))?;
    ensure!(
        win::elevated()? == (scope == Scope::Machine),
        "The MSI action is running under the wrong Windows identity"
    );
    validate_id(product)?;
    let paths = Paths::new(scope)?;
    win::plain_path(Path::new(binary))?;
    ensure!(
        Path::new(binary).parent() == Some(paths.install.as_path())
            && Path::new(binary)
                .file_name()
                .is_some_and(|name| name == "wlanlogin.exe"),
        "Unexpected installation destination"
    );
    let journal = paths.install.join(format!(".installer-{product}"));
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(paths.install.join(".installer.lock"))?;
    lock.try_lock_exclusive()
        .context("Another installer action is in progress")?;
    let result = match action.as_str() {
        "install" => install(
            &paths,
            scope,
            session,
            product,
            Path::new(binary),
            &journal,
            &diagnostics,
        ),
        "rollback" => rollback(&paths, scope, Path::new(binary), &journal, &diagnostics),
        "commit" => cleanup(&journal),
        "uninstall" => uninstall(&paths, scope, Path::new(binary), &journal, &diagnostics),
        _ => bail!("Unknown installer action"),
    };
    ipc::cleanup(session, product)?;
    drop(lock);
    state::remove_file(&paths.install.join(".installer.lock"))?;
    if let Err(error) = &result {
        // A surviving encrypted snapshot and helper support a concrete recovery path.
        if journal.try_exists()? {
            let _ = fs::copy(std::env::current_exe()?, journal.join("recovery.exe"));
            let _ = fs::write(journal.join("error.txt"), format!("{error:#}\n"));
        }
    }
    diagnostics.stage(&format!(
        "action={action} finished success={}",
        result.is_ok()
    ))?;
    if action == "commit" && result.is_ok() {
        diagnostics.cleanup()?;
    }
    result
}

fn install(
    paths: &Paths,
    scope: Scope,
    session: &str,
    transaction: &str,
    binary: &Path,
    journal: &Path,
    diagnostics: &Diagnostics,
) -> Result<()> {
    ensure!(
        !journal.try_exists()?,
        "An earlier recovery snapshot exists at {}",
        journal.display()
    );
    let form = if session.is_empty() {
        None
    } else {
        Some(ipc::receive(session, transaction, scope)?)
    };
    diagnostics.stage("Capturing the original scope state")?;
    let snapshot = Snapshot::capture(paths, scope, form.is_some())?;
    persist(&snapshot, scope, journal)?;
    diagnostics.stage("Invoking service install")?;
    // The public CLI takes its own operation/service locks and performs its existing rollback.
    cli::run(
        binary,
        scope,
        &["service", "install", "--non-interactive"],
        form.as_ref(),
        &diagnostics.cli_log(),
    )?;
    diagnostics.stage("Checking service registration and daemon readiness")?;
    services::wait(scope, true)?;
    wait_runtime(paths)?;
    Ok(())
}

fn uninstall(
    paths: &Paths,
    scope: Scope,
    binary: &Path,
    journal: &Path,
    diagnostics: &Diagnostics,
) -> Result<()> {
    ensure!(
        !journal.try_exists()?,
        "A recovery snapshot already exists at {}",
        journal.display()
    );
    let snapshot = Snapshot::capture_for_removal(paths, scope)?;
    persist(&snapshot, scope, journal)?;
    diagnostics.stage("Invoking service uninstall --purge")?;
    cli::run(
        binary,
        scope,
        &["service", "uninstall", "--purge"],
        None,
        &diagnostics.cli_log(),
    )
}

fn persist(snapshot: &Snapshot, scope: Scope, journal: &Path) -> Result<()> {
    win::private_directory(journal, scope == Scope::Machine)?;
    let bytes = Zeroizing::new(serde_json::to_vec(snapshot)?);
    let encrypted = win::protect(&bytes, scope == Scope::Machine, false)?;
    win::write_atomic(&journal.join("snapshot.bin"), &encrypted)
}

fn rollback(
    paths: &Paths,
    scope: Scope,
    binary: &Path,
    journal: &Path,
    diagnostics: &Diagnostics,
) -> Result<()> {
    let snapshot_file = journal.join("snapshot.bin");
    if !snapshot_file.try_exists()? {
        return Ok(());
    }
    win::plain_path(&snapshot_file)?;
    let bytes = win::protect(&fs::read(snapshot_file)?, scope == Scope::Machine, true)?;
    let mut snapshot: Snapshot = serde_json::from_slice(&bytes)?;
    snapshot.validate(paths, scope)?;
    ensure!(
        snapshot.current_generation(paths)? <= snapshot.maximum_generation,
        "Concurrent credential change detected; recovery snapshot retained at {}",
        journal.display()
    );

    if snapshot.fresh() {
        // A fresh scope has no older credentials or files to preserve.
        cli::run(
            binary,
            scope,
            &["service", "uninstall", "--purge"],
            None,
            &diagnostics.cli_log(),
        )?;
    } else {
        // Uninstall stops/unregisters before any file restoration and keeps saved credentials.
        cli::run(
            binary,
            scope,
            &["service", "uninstall"],
            None,
            &diagnostics.cli_log(),
        )?;
    }
    services::wait(scope, false)?;
    let mut locks = lock_state(paths)?;
    if !snapshot.fresh() {
        snapshot.maximum_generation = snapshot
            .current_generation(paths)?
            .max(snapshot.generation)
            .checked_add(1)
            .context("Credential generation exhausted")?;
        persist(&snapshot, scope, journal)?;
    }
    snapshot.restore_files(paths)?;
    // Release the daemon singleton before restoring a previously running registration.
    drop(locks.remove(1));
    if let Some(registration) = &snapshot.registration {
        registration.restore(scope, journal)?;
    }
    drop(locks);
    snapshot.restore_directories()?;
    cleanup(journal)
}

pub(crate) fn lock_state(paths: &Paths) -> Result<Vec<File>> {
    fs::create_dir_all(&paths.config)?;
    let mut locks = Vec::new();
    for name in ["service.lock", "daemon.lock", "operation.lock"] {
        let path = paths.config.join(name);
        win::plain_path(&path)?;
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path)?;
        lock.try_lock_exclusive()
            .context("Application state is busy; retry after the current operation finishes")?;
        locks.push(lock);
    }
    Ok(locks)
}

fn wait_runtime(paths: &Paths) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Ok(lock) = OpenOptions::new()
            .read(true)
            .write(true)
            .open(paths.config.join("daemon.lock"))
            && lock.try_lock_exclusive().is_err()
        {
            return Ok(());
        }
        ensure!(
            Instant::now() < deadline,
            "The registered background process did not become ready"
        );
        std::thread::sleep(Duration::from_millis(200));
    }
}

fn cleanup(journal: &Path) -> Result<()> {
    if !journal.try_exists()? {
        return Ok(());
    }
    win::plain_path(journal)?;
    // The journal is an exact owned inventory, never a recursive delete of app data.
    let entries = fs::read_dir(journal)?.collect::<std::io::Result<Vec<_>>>()?;
    for entry in &entries {
        ensure!(
            ["snapshot.bin", "task.xml", "error.txt", "recovery.exe"]
                .iter()
                .any(|name| entry.file_name() == *name),
            "Unexpected recovery file; backup retained"
        );
        win::plain_path(&entry.path())?;
    }
    for entry in entries {
        state::remove_file(&entry.path())?;
    }
    fs::remove_dir(journal)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recovery_cleanup_validates_the_entire_inventory_before_deleting() -> Result<()> {
        let directory = tempfile::tempdir()?;
        fs::write(directory.path().join("snapshot.bin"), b"protected backup")?;
        fs::write(directory.path().join("unrelated"), b"keep")?;
        assert!(cleanup(directory.path()).is_err());
        assert_eq!(
            fs::read(directory.path().join("snapshot.bin"))?,
            b"protected backup"
        );
        fs::remove_file(directory.path().join("unrelated"))?;
        cleanup(directory.path())?;
        cleanup(directory.path())?;
        Ok(())
    }
}
