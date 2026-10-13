use crate::infra::error::Result;
use crate::system::define::credentials::Store;

pub(crate) async fn run(
    paths: &crate::infra::paths::Paths,
    store: &impl Store,
    credentials: wlanlogin_lib::Credentials,
) -> Result<()> {
    // Invalidate cached results before even a partially successful write. Caller holds the operation lock.
    crate::infra::control::State::invalidate_credentials(paths)?;

    // Delegate storage policy to the selected scope's adapter.
    store.save(credentials).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::{control::State, paths::Paths};
    use std::cell::Cell;

    struct RecordingStore<'a> {
        paths: &'a Paths,
        called: Cell<bool>,
        fail: bool,
    }
    impl Store for RecordingStore<'_> {
        async fn load(&self) -> Result<Option<wlanlogin_lib::Credentials>> {
            panic!("save must not read old secrets")
        }
        async fn save(&self, _: wlanlogin_lib::Credentials) -> Result<()> {
            assert_eq!(State::load(self.paths)?.credential_generation, 1);
            self.called.set(true);
            if self.fail {
                crate::infra::error::bail!("mock native write failed")
            }
            Ok(())
        }
    }
    fn credentials() -> wlanlogin_lib::Credentials {
        wlanlogin_lib::Credentials {
            username: "mock".into(),
            password: "mock-only".into(),
        }
    }
    #[tokio::test]
    async fn save_invalidates_before_native_mutation_even_if_the_mutation_fails() -> Result<()> {
        for fail in [false, true] {
            let dir = tempfile::tempdir()?;
            let paths = Paths::test(dir.path());
            let store = RecordingStore {
                paths: &paths,
                called: Cell::new(false),
                fail,
            };
            let result = run(&paths, &store, credentials()).await;
            assert_eq!(result.is_err(), fail);
            assert!(store.called.get());
            assert_eq!(State::load(&paths)?.credential_generation, 1);
        }
        Ok(())
    }
    #[tokio::test]
    async fn invalidation_failure_prevents_native_save() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let paths = Paths::test(dir.path());
        State {
            credential_generation: u64::MAX,
            ..Default::default()
        }
        .save(&paths)?;
        let store = RecordingStore {
            paths: &paths,
            called: Cell::new(false),
            fail: false,
        };
        assert!(run(&paths, &store, credentials()).await.is_err());
        assert!(!store.called.get());
        std::fs::write(paths.config.join("control.json"), "invalid control state")?;
        assert!(run(&paths, &store, credentials()).await.is_err());
        assert!(!store.called.get());
        Ok(())
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn invalidation_persistence_failure_prevents_native_save() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let actual = dir.path().join("actual");
        let paths = Paths::test(&actual);
        State::default().save(&paths)?;
        let link = dir.path().join("redirected");
        std::os::unix::fs::symlink(&actual, &link)?;
        let redirected = Paths::test(&link);
        let store = RecordingStore {
            paths: &redirected,
            called: Cell::new(false),
            fail: false,
        };
        assert!(run(&redirected, &store, credentials()).await.is_err());
        assert!(!store.called.get());
        assert_eq!(State::load(&paths)?.credential_generation, 0);
        Ok(())
    }
}
