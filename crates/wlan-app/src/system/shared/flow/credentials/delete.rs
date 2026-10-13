//! Invalidate cached reads before deleting the selected scope's credential storage.
use crate::infra::{control::State, error::Result, paths::Paths};

pub(crate) async fn run(paths: &Paths) -> Result<()> {
    run_with(paths, crate::system::shared::credentials::delete(paths)).await
}

async fn run_with(
    paths: &Paths,
    delete: impl std::future::Future<Output = Result<()>>,
) -> Result<()> {
    // Caller holds the operation lock; failed invalidation must prevent deletion.
    State::invalidate_credentials(paths)?;

    delete.await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    #[tokio::test]
    async fn delete_invalidates_before_mutation_and_preserves_invalidation_on_failure() -> Result<()>
    {
        for fail in [false, true] {
            let dir = tempfile::tempdir()?;
            let paths = Paths::test(dir.path());
            let called = Cell::new(false);
            let result = run_with(&paths, async {
                assert_eq!(State::load(&paths)?.credential_generation, 1);
                called.set(true);
                if fail {
                    crate::infra::error::bail!("mock native deletion failed")
                }
                Ok(())
            })
            .await;
            assert_eq!(result.is_err(), fail);
            assert!(called.get());
            assert_eq!(State::load(&paths)?.credential_generation, 1);
        }
        Ok(())
    }
    #[tokio::test]
    async fn invalidation_failure_preserves_credentials_without_deleting() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let paths = Paths::test(dir.path());
        State {
            credential_generation: u64::MAX,
            ..Default::default()
        }
        .save(&paths)?;
        let called = Cell::new(false);
        assert!(
            run_with(&paths, async {
                called.set(true);
                Ok(())
            })
            .await
            .is_err()
        );
        assert!(!called.get());
        Ok(())
    }
}
