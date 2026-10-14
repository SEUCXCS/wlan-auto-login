use crate::infra::error::Result;
use crate::infra::error::credentials::{CredentialError, Diagnostic, Kind, Stage};
use crate::system::credentials::ScopedStore;
use crate::system::define::credentials::{BackgroundRead, CredentialPermit, Validity};
use std::{future::Future, sync::Arc, time::Duration};

/// Keep a single read in flight if a native keychain call stops responding.
/// Timing out a JoinHandle must not detach it and start another read every tick.
pub struct BackgroundReader {
    store: Arc<ScopedStore>,
    session: tokio::sync::watch::Receiver<crate::system::define::credentials::SessionState>,
    pending: Option<tokio::task::JoinHandle<Result<BackgroundRead>>>,
}

impl BackgroundReader {
    pub fn new(paths: &crate::infra::paths::Paths) -> Result<Self> {
        Ok(Self {
            store: Arc::new(ScopedStore::new(paths, None)?),
            session: tokio::sync::watch::channel(
                crate::system::define::credentials::SessionState::default(),
            )
            .1,
            pending: None,
        })
    }

    pub async fn load(&mut self) -> Result<BackgroundRead> {
        let store = self.store.clone();
        let session = self.session.clone();
        let current = self.session.clone();
        let paths = store.paths().clone();
        let result = self
            .load_with(
                Duration::from_secs(10),
                async move {
                    crate::system::flow::credentials::background::run(&store, session).await
                },
                move |permit| async move {
                    crate::system::flow::credentials::background::validate(&paths, &permit, &current)
                        .await
                },
            )
            .await;
        match result {
            Ok(result) => Ok(result),
            Err(error) => Ok(crate::system::flow::credentials::background::failure(
                &error,
            )),
        }
    }

    pub fn session_changed(
        &mut self,
        session: tokio::sync::watch::Receiver<crate::system::define::credentials::SessionState>,
    ) {
        self.session = session;
    }

    async fn load_with<V>(
        &mut self,
        timeout: Duration,
        read: impl Future<Output = Result<BackgroundRead>> + Send + 'static,
        validate: impl FnOnce(CredentialPermit) -> V,
    ) -> Result<BackgroundRead>
    where
        V: Future<Output = Result<Validity>>,
    {
        let task = self.pending.get_or_insert_with(|| tokio::spawn(read));
        let result = tokio::time::timeout(timeout, task).await.map_err(|error| {
            CredentialError::with_source(Diagnostic::new(Kind::Timeout, Stage::Read), error.into())
        })?;
        self.pending = None;
        let saved = result.map_err(|error| {
            CredentialError::with_source(
                Diagnostic::new(Kind::TaskFailed, Stage::Task),
                error.into(),
            )
        })??;

        // Completion is not authorization: cached successes need a fresh checkpoint.
        if let BackgroundRead::Available(credentials) = &saved {
            let validity = tokio::time::timeout(timeout, validate(credentials.permit.clone()))
                .await
                .map_err(|error| {
                    CredentialError::with_source(
                        Diagnostic::new(Kind::Timeout, Stage::Session),
                        error.into(),
                    )
                })??;
            match validity {
                Validity::Valid => {}
                Validity::SessionExpired => {
                    return Ok(BackgroundRead::Waiting(
                        crate::system::define::credentials::WaitReason::Session,
                    ));
                }
                Validity::CredentialsChanged => {
                    return Ok(BackgroundRead::Waiting(
                        crate::system::define::credentials::WaitReason::CredentialsChanged,
                    ));
                }
            }
        }
        Ok(saved)
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
    use crate::infra::{control::State, error::Context, paths::Paths, scope::Scope};
    use crate::system::define::credentials::{
        BackgroundCredentials, SessionIdentity, SessionPermit, SessionState, WaitReason,
    };
    use wlanlogin_lib::Credentials;

    async fn valid(_: CredentialPermit) -> Result<Validity> {
        Ok(Validity::Valid)
    }

    fn saved(paths: &Paths) -> Result<BackgroundRead> {
        Ok(BackgroundRead::Available(BackgroundCredentials::fixture(
            paths,
            Credentials {
                username: "original".into(),
                password: "mock-secret".into(),
            },
        )?))
    }

    async fn completed(reader: &BackgroundReader) -> Result<()> {
        tokio::time::timeout(Duration::from_secs(1), async {
            while !reader
                .pending
                .as_ref()
                .is_some_and(|task| task.is_finished())
            {
                tokio::task::yield_now().await;
            }
        })
        .await?;
        Ok(())
    }

    #[tokio::test]
    async fn panicked_task_is_a_task_failure_and_a_subsequent_read_can_recover() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let mut reader = BackgroundReader::new(&Paths::test(dir.path()))?;
        let error = reader
            .load_with(
                Duration::from_secs(1),
                async { panic!("mock native task panic") },
                valid,
            )
            .await
            .err()
            .context("expected task error")?;
        assert!(matches!(
            crate::system::flow::credentials::background::failure(&error),
            BackgroundRead::Failed(Diagnostic {
                kind: Kind::TaskFailed,
                stage: Stage::Task,
                ..
            })
        ));
        assert!(matches!(
            reader
                .load_with(
                    Duration::from_secs(1),
                    async { Ok(BackgroundRead::Missing) },
                    valid
                )
                .await?,
            BackgroundRead::Missing
        ));
        Ok(())
    }

    #[tokio::test]
    async fn timed_out_background_read_is_reused_until_it_finishes() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let paths = Paths::test(dir.path());
        let mut reader = BackgroundReader::new(&paths)?;
        let original = saved(&paths)?;
        let release = Arc::new(tokio::sync::Notify::new());
        let wait = release.clone();
        assert!(
            reader
                .load_with(
                    Duration::from_millis(20),
                    async move {
                        wait.notified().await;
                        Ok(original)
                    },
                    valid
                )
                .await
                .is_err()
        );
        for _ in 0..3 {
            let timeout = reader
                .load_with(
                    Duration::from_millis(10),
                    async { panic!("a stalled read must not start a second keychain request") },
                    |_| async { panic!("unfinished reads cannot be consumed") },
                )
                .await
                .err()
                .context("expected timeout")?;
            assert!(matches!(
                crate::system::flow::credentials::background::failure(&timeout),
                BackgroundRead::Failed(Diagnostic {
                    kind: Kind::Timeout,
                    stage: Stage::Read,
                    ..
                })
            ));
        }
        release.notify_one();
        completed(&reader).await?;
        let result = reader
            .load_with(
                Duration::from_secs(1),
                async { panic!("the original result must be consumed first") },
                valid,
            )
            .await?;
        let BackgroundRead::Available(result) = result else {
            panic!("expected credentials")
        };
        assert_eq!(result.credentials.username, "original");
        assert!(matches!(
            reader
                .load_with(
                    Duration::from_secs(1),
                    async { Ok(BackgroundRead::Missing) },
                    valid
                )
                .await?,
            BackgroundRead::Missing
        ));
        Ok(())
    }

    #[tokio::test]
    async fn completed_timed_out_read_must_be_revalidated_after_session_end() -> Result<()> {
        // Test logout, replacement, ABA, and no-hint state, plus an unchanged control.
        for transition in 0..5 {
            let dir = tempfile::tempdir()?;
            let paths = Paths::test(dir.path());
            let initial = SessionState {
                hint: if transition == 3 {
                    None
                } else {
                    Some("old".into())
                },
                generation: 0,
            };
            let (sender, session) = tokio::sync::watch::channel(initial.clone());
            let mut original = BackgroundCredentials::fixture(
                &paths,
                Credentials {
                    username: "old-session".into(),
                    password: "mock-only".into(),
                },
            )?;
            original.permit.session = Some(SessionPermit {
                identity: SessionIdentity {
                    id: "old".into(),
                    started_at: 1,
                },
                state: initial.clone(),
            });
            let release = Arc::new(tokio::sync::Notify::new());
            let wait = release.clone();
            let mut reader = BackgroundReader::new(&paths)?;
            reader.session_changed(session.clone());
            assert!(
                reader
                    .load_with(
                        Duration::from_millis(10),
                        async move {
                            wait.notified().await;
                            Ok(BackgroundRead::Available(original))
                        },
                        valid
                    )
                    .await
                    .is_err()
            );
            release.notify_one();
            completed(&reader).await?;
            // The task has already finished while the old session was still valid.
            let mut next = initial;
            match transition {
                0 => next.update(Some("!old".into()))?,
                1 => next.update(Some("new".into()))?,
                2 => {
                    next.update(Some("!old".into()))?;
                    next.update(Some("old".into()))?;
                }
                3 => next.update(None)?,
                _ => {}
            }
            sender.send_replace(next);
            let result = reader
                .load_with(
                    Duration::from_secs(1),
                    async { panic!("must validate the completed original task") },
                    |permit| async move {
                        let permit = permit.session.context("missing session permit")?;
                        Ok(if permit.state == *session.borrow() {
                            Validity::Valid
                        } else {
                            Validity::SessionExpired
                        })
                    },
                )
                .await?;
            if transition == 4 {
                assert!(matches!(result, BackgroundRead::Available(_)));
            } else {
                assert!(matches!(
                    result,
                    BackgroundRead::Waiting(WaitReason::Session)
                ));
            }
            assert!(reader.pending.is_none());
        }
        Ok(())
    }

    #[tokio::test]
    async fn completed_system_read_is_discarded_after_credential_mutation() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let mut paths = Paths::test(dir.path());
        paths.scope = Scope::System;
        let original = saved(&paths)?;
        let mut reader = BackgroundReader::new(&paths)?;
        let release = Arc::new(tokio::sync::Notify::new());
        let wait = release.clone();
        assert!(
            reader
                .load_with(
                    Duration::from_millis(10),
                    async move {
                        wait.notified().await;
                        Ok(original)
                    },
                    valid
                )
                .await
                .is_err()
        );
        State::invalidate_credentials(&paths)?;
        assert!(
            reader
                .load_with(
                    Duration::from_millis(10),
                    async { panic!("invalidation must not start a second native read") },
                    valid
                )
                .await
                .is_err()
        );
        release.notify_one();
        completed(&reader).await?;
        let session = tokio::sync::watch::channel(SessionState::default()).1;
        let result = reader
            .load_with(
                Duration::from_secs(1),
                async { panic!("must consume the old task first") },
                |permit| async move {
                    crate::system::shared::credentials::validation::validate(
                        &paths, &permit, &session,
                    )
                    .await
                },
            )
            .await?;
        assert!(matches!(
            result,
            BackgroundRead::Waiting(WaitReason::CredentialsChanged)
        ));
        Ok(())
    }

    #[tokio::test]
    async fn failed_or_timed_out_consumption_validation_never_returns_credentials() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let paths = Paths::test(dir.path());
        let mut reader = BackgroundReader::new(&paths)?;
        let original = saved(&paths)?;
        let failure = reader
            .load_with(Duration::from_secs(1), async { Ok(original) }, |_| async {
                Err(std::io::Error::from(std::io::ErrorKind::BrokenPipe).into())
            })
            .await;
        assert!(failure.is_err());
        assert!(reader.pending.is_none());
        let original = saved(&paths)?;
        let error = reader
            .load_with(
                Duration::from_millis(10),
                async { Ok(original) },
                |_| async { std::future::pending().await },
            )
            .await
            .err()
            .context("expected validation timeout")?;
        assert!(matches!(
            crate::system::flow::credentials::background::failure(&error),
            BackgroundRead::Failed(Diagnostic {
                kind: Kind::Timeout,
                stage: Stage::Session,
                ..
            })
        ));
        Ok(())
    }
}
