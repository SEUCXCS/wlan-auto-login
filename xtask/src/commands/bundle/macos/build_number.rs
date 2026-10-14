//! Reserve macOS bundle build numbers independently of Cargo versions and output paths.
use anyhow::{Context, Result, ensure};
use std::{
    fs::{self, File},
    io::Write,
    path::Path,
};

pub fn reserve(root: &Path) -> Result<u64> {
    // Keep the counter outside target so cargo clean cannot reset it. All
    // profiles and targets in this working copy share the same sequence.
    let directory = root.join(".bundle-build");
    fs::create_dir_all(&directory).context("Failed to create bundle build-number directory")?;
    let lock = File::options()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(directory.join("build-number.lock"))
        .context("Failed to open bundle build-number lock")?;
    lock.lock().context("Failed to lock bundle build number")?;

    let path = directory.join("build-number");
    let previous = match fs::read_to_string(&path) {
        Ok(value) => {
            let value = value.trim();
            ensure!(
                !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()),
                "Invalid bundle build number in {}",
                path.display()
            );
            value
                .parse::<u64>()
                .with_context(|| format!("Invalid bundle build number in {}", path.display()))?
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
        Err(error) => return Err(error).context("Failed to read bundle build number"),
    };
    let next = previous
        .checked_add(1)
        .context("Bundle build number is exhausted")?;

    // Publish the reservation atomically while holding the stable lock file.
    // Replacing the counter must not replace the inode on which callers lock.
    let mut staged = tempfile::NamedTempFile::new_in(&directory)?;
    writeln!(staged, "{next}")?;
    staged.as_file().sync_all()?;
    staged
        .persist(&path)
        .context("Failed to save bundle build number")?;
    Ok(next)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reservations_survive_output_cleanup_and_unused_numbers_are_not_reused() -> Result<()> {
        let root = tempfile::tempdir()?;
        let target = root.path().join("target");
        fs::create_dir(&target)?;
        assert_eq!(reserve(root.path())?, 1);
        // A build can fail after reservation, or its artifacts can be cleaned.
        fs::remove_dir(&target)?;
        assert_eq!(reserve(root.path())?, 2);
        assert_eq!(
            fs::read_to_string(root.path().join(".bundle-build/build-number"))?,
            "2\n"
        );
        Ok(())
    }

    #[test]
    fn concurrent_reservations_are_unique() -> Result<()> {
        let root = tempfile::tempdir()?;
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
        let workers: Vec<_> = (0..8)
            .map(|_| {
                let root = root.path().to_owned();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    reserve(&root)
                })
            })
            .collect();
        let mut numbers = Vec::new();
        for worker in workers {
            numbers.push(
                worker
                    .join()
                    .map_err(|_| anyhow::anyhow!("Build-number worker panicked"))??,
            );
        }
        numbers.sort_unstable();
        assert_eq!(numbers, (1..=8).collect::<Vec<_>>());
        Ok(())
    }

    #[test]
    fn invalid_or_exhausted_counters_fail_without_resetting() -> Result<()> {
        let root = tempfile::tempdir()?;
        let directory = root.path().join(".bundle-build");
        fs::create_dir(&directory)?;
        let path = directory.join("build-number");
        for value in ["", "broken", "0.1.0", "+1", "18446744073709551615"] {
            fs::write(&path, value)?;
            assert!(reserve(root.path()).is_err());
            assert_eq!(fs::read_to_string(&path)?, value);
        }
        Ok(())
    }
}
