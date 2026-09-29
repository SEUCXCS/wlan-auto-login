use super::ScopedStore;
use crate::infra::error::Message;
use crate::infra::error::{Context, Result};
use cxxy_wlanlogin_lib::Credentials;
use std::{future::Future, sync::Arc, time::Duration};

/// Keep a single read in flight if a native keychain call stops responding.
/// Timing out a JoinHandle must not detach it and start another read every tick.
pub struct BackgroundReader {
    store: Arc<ScopedStore>,
    pending: Option<tokio::task::JoinHandle<Result<Option<Credentials>>>>,
}

impl BackgroundReader {
    pub fn new(paths: &crate::infra::paths::Paths) -> Result<Self> {
        Ok(Self {
            store: Arc::new(ScopedStore::new(paths, None)?),
            pending: None,
        })
    }

    pub async fn load(&mut self) -> Result<Option<Credentials>> {
        let store = self.store.clone();
        self.load_with(Duration::from_secs(10), async move {
            store.load_background().await
        })
        .await
    }

    async fn load_with(
        &mut self,
        timeout: Duration,
        read: impl Future<Output = Result<Option<Credentials>>> + Send + 'static,
    ) -> Result<Option<Credentials>> {
        let task = self.pending.get_or_insert_with(|| tokio::spawn(read));
        let result = tokio::time::timeout(timeout, task)
            .await
            .context(Message::BackgroundCredentialReadTimedOutWaitingForThe)?;
        self.pending = None;
        result.context(Message::BackgroundCredentialReadTaskFailed)?
    }
}

impl Drop for BackgroundReader {
    fn drop(&mut self) {
        if let Some(task) = &self.pending {
            task.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn timed_out_background_read_is_reused_until_it_finishes() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let mut reader = BackgroundReader::new(&crate::infra::paths::Paths::test(dir.path()))?;
        let release = Arc::new(tokio::sync::Notify::new());
        let wait = release.clone();
        assert!(
            reader
                .load_with(Duration::from_millis(20), async move {
                    wait.notified().await;
                    Ok(Some(Credentials {
                        username: "original".into(),
                        password: "test".into(),
                    }))
                })
                .await
                .is_err()
        );
        for _ in 0..3 {
            assert!(
                reader
                    .load_with(Duration::from_millis(10), async {
                        panic!("a stalled read must not start a second keychain request");
                    })
                    .await
                    .is_err()
            );
        }
        release.notify_one();
        let result = reader
            .load_with(Duration::from_secs(1), async {
                panic!("the original result must be consumed first");
            })
            .await?
            .context("saved credentials expected")?;
        assert_eq!(result.username, "original");
        assert!(
            reader
                .load_with(Duration::from_secs(1), async { Ok(None) })
                .await?
                .is_none()
        );
        Ok(())
    }
}
