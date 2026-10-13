use crate::infra::error::Message;
use crate::infra::error::credentials::{CredentialError, Kind, Stage};
use crate::system::shared::credentials::decode;
use crate::system::shared::credentials::diagnostics;
use zeroize::Zeroizing;

use crate::infra::error::{Context, Result};
pub(crate) use crate::system::shared::credentials::{load_user as load, save_user as save};

use crate::infra::identity::{APP_ID as SERVICE, CREDENTIAL_ENTRY as ENTRY};

pub async fn delete() -> Result<()> {
    tokio::task::spawn_blocking(|| {
        {
            use dbus_secret_service::{EncryptionType, SecretService};
            let service = SecretService::connect(EncryptionType::Dh)?;
            let items = service.search_items(std::collections::HashMap::from([
                ("service", SERVICE),
                ("username", ENTRY),
                ("target", "default"),
            ]))?;
            // Remove every matching application item, including duplicates, without
            // reading secrets or touching the collection or other applications.
            for item in items.unlocked.iter().chain(&items.locked) {
                item.delete()
                    .context(Message::FailedToDeleteUserCredentialsUnlockTheSystem)?;
            }
            Ok(())
        }
    })
    .await
    .context(Message::CredentialDeletionTaskFailed)?
}

/// Standard Secret Service calls are permitted only after session preparation.
pub async fn load_for_daemon(
    session: tokio::sync::watch::Receiver<crate::system::define::credentials::SessionState>,
    permit: crate::system::define::credentials::CredentialPermit,
) -> Result<crate::system::define::credentials::BackgroundRead> {
    let validation_session = session.clone();
    load_with(
        async {
            crate::system::linux::runtime::session::prepare_read(&session)
                .await
                .map_err(|error| diagnostics::at(error, Stage::Session))
        },
        || async {
            tokio::task::spawn_blocking(move || read_standard(permit))
                .await
                .map_err(|error| diagnostics::at(error.into(), Stage::Task))?
        },
        |permit| async move {
            crate::system::linux::runtime::session::valid(&permit, &validation_session)
                .await
                .map_err(|error| diagnostics::at(error, Stage::Session))
        },
    )
    .await
}

async fn load_with<P, R, V>(
    prepare: P,
    read: impl FnOnce() -> R,
    valid: impl FnOnce(crate::system::define::credentials::SessionPermit) -> V,
) -> Result<crate::system::define::credentials::BackgroundRead>
where
    P: std::future::Future<
            Output = Result<Option<crate::system::define::credentials::SessionPermit>>,
        >,
    R: std::future::Future<Output = Result<crate::system::define::credentials::BackgroundRead>>,
    V: std::future::Future<Output = Result<bool>>,
{
    use crate::system::define::credentials::{BackgroundRead, WaitReason};
    // Even an already-running provider passes the same readiness gate.
    let Some(permit) = prepare.await? else {
        return Ok(BackgroundRead::Waiting(WaitReason::Session));
    };

    // The official library uses org.freedesktop.secrets and standard D-Bus activation.
    let mut saved = read().await?;

    // Revalidate the session after the blocking read, before returning credentials.
    if let BackgroundRead::Available(credentials) = &mut saved {
        credentials.permit.session = Some(permit.clone());
        if !valid(permit).await? {
            return Ok(BackgroundRead::Waiting(WaitReason::Session));
        }
    }
    Ok(saved)
}

fn read_standard(
    permit: crate::system::define::credentials::CredentialPermit,
) -> Result<crate::system::define::credentials::BackgroundRead> {
    use crate::system::define::credentials::{BackgroundRead, WaitReason};
    use dbus_secret_service::{EncryptionType, SecretService};
    let service = SecretService::connect_with_max_prompt_timeout(EncryptionType::Dh, 0)
        .map_err(|e| diagnostics::at(e.into(), Stage::Connect))?;
    let items = service
        .search_items(std::collections::HashMap::from([
            ("service", SERVICE),
            ("username", ENTRY),
            ("target", "default"),
        ]))
        .map_err(|e| diagnostics::at(e.into(), Stage::Search))?;
    if !items.locked.is_empty() {
        return Ok(BackgroundRead::Waiting(WaitReason::Locked));
    }
    if items.unlocked.len() > 1 {
        return Err(CredentialError::new(Kind::DuplicateEntries, Stage::Search).into());
    }
    match items.unlocked.first() {
        Some(item) => {
            let bytes = Zeroizing::new(
                item.get_secret()
                    .map_err(|e| diagnostics::at(e.into(), Stage::Read))?,
            );
            let text = std::str::from_utf8(&bytes)
                .map_err(|e| diagnostics::at(e.into(), Stage::Decode))?;
            decode(text).map(|credentials| BackgroundRead::from_saved(Some(credentials), permit))
        }
        None => Ok(BackgroundRead::Missing),
    }
}

pub fn prepare_daemon() -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::define::credentials::{BackgroundRead, WaitReason};
    use futures_util::StreamExt;
    use std::{
        cell::Cell,
        io::{BufRead, BufReader},
        process::{Child, Stdio},
    };
    use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

    fn session_permit() -> crate::system::define::credentials::SessionPermit {
        crate::system::define::credentials::SessionPermit {
            identity: crate::system::define::credentials::SessionIdentity {
                id: "mock".into(),
                started_at: 1,
            },
            state: crate::system::define::credentials::SessionState::default(),
        }
    }

    #[tokio::test]
    async fn session_preparation_precedes_even_an_already_running_provider_and_expiry_discards_credentials()
    -> Result<()> {
        let dir = tempfile::tempdir()?;
        let paths = crate::infra::paths::Paths::test(dir.path());
        let reads = Cell::new(0);
        let waiting = load_with(
            async { Ok::<_, crate::infra::error::Error>(None) },
            || async {
                reads.set(reads.get() + 1);
                panic!("read before session readiness")
            },
            |_| async { Ok(true) },
        )
        .await?;
        assert!(matches!(
            waiting,
            BackgroundRead::Waiting(WaitReason::Session)
        ));
        assert_eq!(reads.get(), 0);
        for valid in [false, true] {
            let result = load_with(
                async { Ok(Some(session_permit())) },
                || async {
                    reads.set(reads.get() + 1);
                    Ok(BackgroundRead::Available(
                        crate::system::define::credentials::BackgroundCredentials::fixture(
                            &paths,
                            wlanlogin_lib::Credentials {
                                username: "mock".into(),
                                password: "mock-only".into(),
                            },
                        )?,
                    ))
                },
                |_| async { Ok(valid) },
            )
            .await?;
            assert_eq!(matches!(result, BackgroundRead::Available(_)), valid);
        }
        let error = load_with(
            async {
                Err::<Option<crate::system::define::credentials::SessionPermit>, _>(
                    CredentialError::new(Kind::BackendUnavailable, Stage::Session).into(),
                )
            },
            || async { panic!("failed preparation must not read") },
            |_| async { Ok(true) },
        )
        .await
        .err()
        .context("expected preparation failure")?;
        assert!(matches!(
            diagnostics::background(&error),
            BackgroundRead::Failed(crate::infra::error::credentials::Diagnostic {
                kind: Kind::BackendUnavailable,
                stage: Stage::Session,
                ..
            })
        ));
        Ok(())
    }

    struct PrivateBus {
        child: Child,
        address: String,
        directory: tempfile::TempDir,
        _stdout: BufReader<std::process::ChildStdout>,
    }
    impl PrivateBus {
        fn start(mode: &str) -> Result<Self> {
            let directory = tempfile::tempdir()?;
            let address = format!(
                "unix:path={}",
                directory.path().join("bus.socket").display()
            );
            let service_dir = directory.path().join("services");
            std::fs::create_dir(&service_dir)?;
            let executable = std::env::current_exe()?;
            let quoted = executable
                .to_string_lossy()
                .replace('\\', "\\\\")
                .replace('"', "\\\"");
            std::fs::write(
                service_dir.join("secrets.service"),
                format!(
                    "[D-BUS Service]\nName=org.freedesktop.secrets\nExec=\"{quoted}\" --exact system::linux::credentials::user::tests::provider_child --ignored --nocapture\n"
                ),
            )?;
            let config = directory.path().join("bus.conf");
            std::fs::write(
                &config,
                format!(
                    "<busconfig><type>session</type><listen>{address}</listen><servicedir>{}</servicedir><policy context=\"default\"><allow send_destination=\"*\"/><allow receive_sender=\"*\"/><allow own=\"*\"/></policy></busconfig>",
                    service_dir.display()
                ),
            )?;
            let mut child = std::process::Command::new("dbus-daemon")
                .arg("--nofork")
                .arg("--print-address=1")
                .arg(format!("--config-file={}", config.display()))
                .env("DBUS_SESSION_BUS_ADDRESS", &address)
                .env("WLANLOGIN_TEST_PRIVATE_BUS", directory.path())
                .env("WLANLOGIN_TEST_PROVIDER_MODE", mode)
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()?;
            let stdout = child.stdout.take().context("private bus stdout")?;
            let mut stdout = BufReader::new(stdout);
            let mut announced = String::new();
            stdout.read_line(&mut announced)?;
            if announced.trim().is_empty() {
                let _ = child.kill();
                let _ = child.wait();
                crate::infra::error::bail!("Private D-Bus did not start");
            }
            Ok(Self {
                child,
                address: announced.trim().into(),
                directory,
                _stdout: stdout,
            })
        }
        async fn read(&self) -> Result<BackgroundRead> {
            // A fresh child owns its environment; parallel tests never change process env.
            let report = self.directory.path().join("read.json");
            let output = tokio::time::timeout(
                std::time::Duration::from_secs(8),
                tokio::process::Command::new(std::env::current_exe()?)
                    .args([
                        "--exact",
                        "system::linux::credentials::user::tests::read_child",
                        "--ignored",
                        "--nocapture",
                    ])
                    .env("DBUS_SESSION_BUS_ADDRESS", &self.address)
                    .env("WLANLOGIN_TEST_PRIVATE_BUS", self.directory.path())
                    .env("WLANLOGIN_TEST_READ_REPORT", &report)
                    .kill_on_drop(true)
                    .output(),
            )
            .await??;
            crate::infra::error::ensure!(
                output.status.success(),
                "Isolated Secret Service reader failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            let result: serde_json::Value = serde_json::from_slice(&std::fs::read(report)?)?;
            match result["state"].as_str() {
                Some("missing") => Ok(BackgroundRead::Missing),
                Some("locked") => Ok(BackgroundRead::Waiting(WaitReason::Locked)),
                Some("failed") => Ok(BackgroundRead::Failed(serde_json::from_value(
                    result["diagnostic"].clone(),
                )?)),
                _ => crate::infra::error::bail!("Invalid mock credential read report"),
            }
        }
    }
    impl Drop for PrivateBus {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }

    fn private_environment() -> Result<std::path::PathBuf> {
        let directory = std::env::var_os("WLANLOGIN_TEST_PRIVATE_BUS")
            .map(std::path::PathBuf::from)
            .context("private bus helper requires its parent")?;
        let address = std::env::var("DBUS_SESSION_BUS_ADDRESS").context("private bus address")?;
        crate::infra::error::ensure!(
            address.starts_with(&format!(
                "unix:path={}",
                directory.join("bus.socket").display()
            )),
            "Helper refuses non-private bus"
        );
        Ok(directory)
    }
    struct Provider {
        mode: String,
    }
    #[zbus::interface(name = "org.freedesktop.Secret.Service")]
    impl Provider {
        fn open_session(
            &self,
            algorithm: String,
            _input: OwnedValue,
        ) -> zbus::fdo::Result<(OwnedValue, OwnedObjectPath)> {
            assert_eq!(algorithm, "dh-ietf1024-sha256-aes128-cbc-pkcs7");
            Ok((
                OwnedValue::try_from(Value::from(vec![2u8]))
                    .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?,
                OwnedObjectPath::try_from("/session")
                    .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?,
            ))
        }
        fn search_items(
            &self,
            _attributes: std::collections::HashMap<String, String>,
        ) -> zbus::fdo::Result<(Vec<OwnedObjectPath>, Vec<OwnedObjectPath>)> {
            let item = OwnedObjectPath::try_from("/item")
                .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
            match self.mode.as_str() {
                "locked" => Ok((Vec::new(), vec![item])),
                "duplicate" => Ok((vec![item.clone(), item], Vec::new())),
                "exit" => std::process::exit(0),
                _ => Ok((Vec::new(), Vec::new())),
            }
        }
    }
    #[tokio::test]
    #[ignore = "private D-Bus activation helper; invoked only by its parent test"]
    async fn provider_child() -> Result<()> {
        let directory = private_environment()?;
        let mode = std::env::var("WLANLOGIN_TEST_PROVIDER_MODE").context("mock mode")?;
        let connection = zbus::connection::Builder::address(
            std::env::var("DBUS_SESSION_BUS_ADDRESS")
                .context("private address")?
                .as_str(),
        )?
        .serve_at("/org/freedesktop/secrets", Provider { mode })?
        .name("org.freedesktop.secrets")?
        .build()
        .await?;
        std::fs::write(directory.join("activated"), "mock provider started")?;
        let mut messages = zbus::MessageStream::from(connection);
        while let Some(message) = messages.next().await {
            if message.is_err() {
                break;
            }
        }
        Ok(())
    }
    #[test]
    #[ignore = "private D-Bus credential helper; invoked only by its parent test"]
    fn read_child() -> Result<()> {
        let directory = private_environment()?;
        let paths = crate::infra::paths::Paths::test(directory);
        let result = match read_standard(crate::system::shared::credentials::validation::snapshot(
            &paths,
        )?) {
            Ok(BackgroundRead::Missing) => serde_json::json!({"state":"missing"}),
            Ok(BackgroundRead::Waiting(WaitReason::Locked)) => {
                serde_json::json!({"state":"locked"})
            }
            Err(error) => {
                serde_json::json!({"state":"failed","diagnostic":diagnostics::diagnostic(&error,Stage::Read)})
            }
            _ => crate::infra::error::bail!("Unexpected mock result"),
        };
        let report = std::env::var_os("WLANLOGIN_TEST_READ_REPORT").context("mock report path")?;
        std::fs::write(report, serde_json::to_vec(&result)?)?;
        Ok(())
    }

    #[tokio::test]
    async fn official_library_activates_standard_service_only_after_readiness_and_reports_lock_or_duplicates()
    -> Result<()> {
        for mode in ["missing", "locked", "duplicate", "exit"] {
            let bus = PrivateBus::start(mode)?;
            let waiting = load_with(
                async { Ok::<_, crate::infra::error::Error>(None) },
                || bus.read(),
                |_| async { Ok(true) },
            )
            .await?;
            assert!(matches!(
                waiting,
                BackgroundRead::Waiting(WaitReason::Session)
            ));
            assert!(!bus.directory.path().join("activated").exists());
            let result = load_with(
                async { Ok(Some(session_permit())) },
                || bus.read(),
                |_| async { Ok(true) },
            )
            .await?;
            assert!(bus.directory.path().join("activated").exists());
            match mode {
                "missing" => assert!(matches!(result, BackgroundRead::Missing)),
                "locked" => assert!(matches!(
                    result,
                    BackgroundRead::Waiting(WaitReason::Locked)
                )),
                "exit" => assert!(matches!(
                    result,
                    BackgroundRead::Failed(crate::infra::error::credentials::Diagnostic {
                        kind: Kind::Timeout,
                        stage: Stage::Search,
                        ..
                    })
                )),
                _ => assert!(matches!(
                    result,
                    BackgroundRead::Failed(crate::infra::error::credentials::Diagnostic {
                        kind: Kind::DuplicateEntries,
                        stage: Stage::Search,
                        ..
                    })
                )),
            }
        }
        Ok(())
    }
}
