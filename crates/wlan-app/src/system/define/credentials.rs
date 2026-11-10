//! Credential storage contract shared by controllers and native backends.
use crate::infra::error::Result;
use wlanlogin_lib::Credentials;
#[derive(Clone, Copy, Debug)]
pub enum CredentialMode {
    Native,
    ProtectedFile,
}
pub trait Store {
    fn load(&self) -> impl std::future::Future<Output = Result<Option<Credentials>>>;
    fn save(&self, credentials: Credentials) -> impl std::future::Future<Output = Result<()>>;
}

/// Normal background states are independent of portal/network failure backoff.
/// Credentials deliberately have no Debug or Serialize representation here.
pub enum BackgroundRead {
    Available(BackgroundCredentials),
    Missing,
    Waiting(WaitReason),
    Failed(crate::infra::error::credentials::Diagnostic),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WaitReason {
    Session,
    CredentialsChanged,
    Provider,
    Locked,
    PermissionDenied,
    NeedsInteraction,
}

/// Local state only: never serialize or print credential values or their permits.
pub struct BackgroundCredentials {
    pub credentials: Credentials,
    pub permit: CredentialPermit,
}

#[derive(Clone)]
pub struct CredentialPermit {
    pub scope: crate::infra::scope::Scope,
    pub config: std::path::PathBuf,
    pub generation: u64,
    // The shared permit shape is identical; only Linux user storage consumes this field.
    #[allow(dead_code)]
    pub session: Option<SessionPermit>,
}

#[derive(Clone, Default, PartialEq, Eq)]
pub struct SessionState {
    pub hint: Option<String>,
    pub generation: u64,
}
impl SessionState {
    pub fn update(&mut self, hint: Option<String>) -> Result<()> {
        self.generation = self.generation.checked_add(1).ok_or_else(|| {
            crate::infra::error::error!(crate::infra::error::Message::SessionGenerationExhausted)
        })?;
        self.hint = hint;
        Ok(())
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct SessionIdentity {
    pub id: String,
    pub started_at: u64,
}
#[derive(Clone)]
#[allow(dead_code)] // Native session proof is Linux-only.
pub struct SessionPermit {
    pub identity: SessionIdentity,
    pub state: SessionState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Validity {
    Valid,
    SessionExpired,
    CredentialsChanged,
}

impl BackgroundRead {
    pub fn from_saved(saved: Option<Credentials>, permit: CredentialPermit) -> Self {
        match saved {
            Some(credentials) => Self::Available(BackgroundCredentials {
                credentials,
                permit,
            }),
            None => Self::Missing,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::error::credentials::{Diagnostic, Kind, Stage};
    use crate::infra::{
        control::State,
        error::{Context, Error, Result},
        paths::Paths,
        scope::Scope,
    };
    use crate::system::flow::credentials::save::run as save;
    use crate::system::shared::credentials::{
        diagnostics::{at, background, diagnostic, specific},
        validation::{snapshot, validate},
    };
    use std::cell::Cell;
    use wlanlogin_lib::Credentials;

    impl crate::system::define::credentials::BackgroundCredentials {
        pub(crate) fn fixture(
            paths: &crate::infra::paths::Paths,
            credentials: Credentials,
        ) -> Result<Self> {
            Ok(Self {
                credentials,
                permit: snapshot(paths)?,
            })
        }
    }

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

    #[test]
    fn decryption_context_preserves_timeouts_and_denials_and_labels_other_decrypt_failures() {
        for (cause, expected) in [
            (std::io::ErrorKind::TimedOut, Kind::Timeout),
            (std::io::ErrorKind::PermissionDenied, Kind::PermissionDenied),
            (std::io::ErrorKind::InvalidData, Kind::DecryptionFailed),
        ] {
            let error = specific(
                std::io::Error::from(cause).into(),
                Kind::DecryptionFailed,
                Stage::Decrypt,
            );
            let detail = diagnostic(&error, Stage::Read);
            assert_eq!(detail.kind, expected);
            assert_eq!(detail.stage, Stage::Decrypt);
            assert!(error.downcast_ref::<std::io::Error>().is_some());
        }
    }

    #[test]
    fn codec_and_native_causes_keep_distinct_kinds_and_stages() -> Result<()> {
        let cases = [
            (
                crate::system::shared::credentials::decode("not json").err(),
                Kind::InvalidFormat,
            ),
            (
                crate::system::shared::credentials::decode(
                    r#"{"version":2,"username":"a","password":"b"}"#,
                )
                .err(),
                Kind::UnsupportedVersion,
            ),
            (
                crate::system::shared::credentials::decode(r#"{"version":1,"username":"a"}"#).err(),
                Kind::Incomplete,
            ),
        ];
        for (error, kind) in cases {
            let nested = Err::<(), _>(error.context("expected codec error")?)
                .context("task wrapper")
                .err()
                .context("wrapped error")?;
            let detail = diagnostic(&nested, Stage::Read);
            assert_eq!(detail.kind, kind);
            assert_eq!(detail.stage, Stage::Decode);
            assert!(matches!(background(&nested), BackgroundRead::Failed(_)));
        }
        let denied = Error::from(std::io::Error::from(std::io::ErrorKind::PermissionDenied));
        assert!(matches!(
            background(&denied),
            BackgroundRead::Waiting(WaitReason::PermissionDenied)
        ));
        let timeout = Error::from(std::io::Error::from(std::io::ErrorKind::TimedOut));
        assert!(matches!(
            background(&timeout),
            BackgroundRead::Failed(Diagnostic {
                kind: Kind::Timeout,
                ..
            })
        ));
        let unknown = Error::message("unclassified backend failure");
        assert!(matches!(
            background(&unknown),
            BackgroundRead::Failed(Diagnostic {
                kind: Kind::Unknown,
                ..
            })
        ));
        Ok(())
    }

    #[test]
    fn sensitive_native_payload_is_retained_as_source_but_never_formatted_or_serialized()
    -> Result<()> {
        let secret = "DO_NOT_PRINT_SAVED_PASSWORD";
        let raw = Error::from(keyring::Error::BadEncoding(secret.as_bytes().to_vec()));
        let error = at(raw, Stage::Read);
        assert!(error.downcast_ref::<keyring::Error>().is_some());
        assert!(!format!("{error}").contains(secret));
        assert!(!format!("{error:?}").contains(secret));
        let detail = diagnostic(&error, Stage::Task);
        assert_eq!(detail.kind, Kind::InvalidEncoding);
        assert_eq!(detail.stage, Stage::Decode);
        assert!(!serde_json::to_string(&detail)?.contains(secret));
        Ok(())
    }

    #[tokio::test]
    async fn generations_are_scope_local_and_system_reads_ignore_desktop_changes() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let user = Paths::test(dir.path().join("user"));
        let mut system = Paths::test(dir.path().join("system"));
        system.scope = Scope::System;
        let permit = snapshot(&system)?;
        let (sender, receiver) = tokio::sync::watch::channel(SessionState::default());
        State::invalidate_credentials(&user)?;
        let mut state = receiver.borrow().clone();
        state.update(Some("!old-desktop".into()))?;
        sender.send_replace(state);
        assert_eq!(
            validate(&system, &permit, &receiver).await?,
            Validity::Valid
        );
        State::invalidate_credentials(&system)?;
        assert_eq!(
            validate(&system, &permit, &receiver).await?,
            Validity::CredentialsChanged
        );
        assert_eq!(State::load(&user)?.credential_generation, 1);
        Ok(())
    }

    #[tokio::test]
    async fn permits_are_bound_to_scope_and_config_root() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let mut paths = Paths::test(dir.path().join("one"));
        paths.scope = Scope::System;
        let permit = snapshot(&paths)?;
        let receiver = tokio::sync::watch::channel(SessionState::default()).1;
        let mut wrong = paths.clone();
        wrong.scope = Scope::User;
        assert!(validate(&wrong, &permit, &receiver).await.is_err());
        wrong = Paths::test(dir.path().join("two"));
        wrong.scope = Scope::System;
        assert!(validate(&wrong, &permit, &receiver).await.is_err());
        Ok(())
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
            let result = save(&paths, &store, credentials()).await;
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
        assert!(save(&paths, &store, credentials()).await.is_err());
        assert!(!store.called.get());
        std::fs::write(paths.config.join("control.json"), "invalid control state")?;
        assert!(save(&paths, &store, credentials()).await.is_err());
        assert!(!store.called.get());
        Ok(())
    }

    #[tokio::test]
    async fn invalidation_persistence_failure_prevents_native_save() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let actual = dir.path().join("actual");
        let paths = Paths::test(&actual);
        State::default().save(&paths)?;
        let link = dir.path().join("redirected");
        crate::infra::filesystem::tests::redirect_directory(&actual, &link)?;
        let redirected = Paths::test(&link);
        let store = RecordingStore {
            paths: &redirected,
            called: Cell::new(false),
            fail: false,
        };
        assert!(save(&redirected, &store, credentials()).await.is_err());
        assert!(!store.called.get());
        assert_eq!(State::load(&paths)?.credential_generation, 0);
        Ok(())
    }
}
