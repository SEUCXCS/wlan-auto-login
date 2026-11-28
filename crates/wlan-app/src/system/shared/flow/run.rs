//! The ordinary process entry has no additional native hosting requirements.
use crate::infra::{error::Result, paths::Paths};

#[allow(dead_code)] // The macOS native runner owns its GUI loop.
pub(crate) async fn run(
    _paths: &Paths,
    _managed: bool,
    operation: impl std::future::Future<Output = Result<()>>,
) -> Result<()> {
    // The application supplies its daemon future, preserving app -> system -> infra.
    operation.await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn foreground_run_drives_the_supplied_operation() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let paths = Paths::test(directory.path());
        let mut ran = false;
        run(&paths, false, async {
            ran = true;
            Ok(())
        })
        .await?;
        assert!(ran);
        Ok(())
    }
}
