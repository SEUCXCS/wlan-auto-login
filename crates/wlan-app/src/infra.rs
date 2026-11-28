//! Shared process, persistence and filesystem infrastructure.
#[cfg(target_os = "linux")]
#[path = "system/linux/runtime/infra/cleanup.rs"]
pub(crate) mod cleanup;
#[cfg(target_os = "macos")]
#[path = "system/macos/runtime/infra/cleanup.rs"]
pub(crate) mod cleanup;
#[cfg(target_os = "windows")]
#[path = "system/windows/runtime/infra/cleanup.rs"]
pub(crate) mod cleanup;
#[cfg(target_os = "linux")]
#[path = "system/linux/runtime/infra/command.rs"]
pub(crate) mod command;
#[cfg(target_os = "macos")]
#[path = "system/macos/runtime/infra/command.rs"]
pub(crate) mod command;
#[cfg(target_os = "windows")]
#[path = "system/windows/runtime/infra/command.rs"]
pub(crate) mod command;
pub(crate) mod config;
#[cfg(target_os = "linux")]
#[path = "system/linux/runtime/infra/control.rs"]
pub(crate) mod control;
#[cfg(target_os = "macos")]
#[path = "system/macos/runtime/infra/control.rs"]
pub(crate) mod control;
#[cfg(target_os = "windows")]
#[path = "system/windows/runtime/infra/control.rs"]
pub(crate) mod control;
pub(crate) mod error;
pub(crate) mod file_log;
#[cfg(target_os = "linux")]
#[path = "system/linux/runtime/infra/filesystem.rs"]
pub(crate) mod filesystem;
#[cfg(target_os = "macos")]
#[path = "system/macos/runtime/infra/filesystem.rs"]
pub(crate) mod filesystem;
#[cfg(target_os = "windows")]
#[path = "system/windows/runtime/infra/filesystem.rs"]
pub(crate) mod filesystem;
pub(crate) mod identity;
pub(crate) mod logging;
#[cfg(target_os = "linux")]
#[path = "system/linux/runtime/infra/paths.rs"]
pub(crate) mod paths;
#[cfg(target_os = "macos")]
#[path = "system/macos/runtime/infra/paths.rs"]
pub(crate) mod paths;
#[cfg(target_os = "windows")]
#[path = "system/windows/runtime/infra/paths.rs"]
pub(crate) mod paths;
#[cfg(target_os = "linux")]
#[path = "system/linux/runtime/infra/scope.rs"]
pub(crate) mod scope;
#[cfg(target_os = "macos")]
#[path = "system/macos/runtime/infra/scope.rs"]
pub(crate) mod scope;
#[cfg(target_os = "windows")]
#[path = "system/windows/runtime/infra/scope.rs"]
pub(crate) mod scope;
#[cfg(target_os = "linux")]
#[path = "system/linux/runtime/infra/shutdown.rs"]
pub(crate) mod shutdown;
#[cfg(target_os = "macos")]
#[path = "system/macos/runtime/infra/shutdown.rs"]
pub(crate) mod shutdown;
#[cfg(target_os = "windows")]
#[path = "system/windows/runtime/infra/shutdown.rs"]
pub(crate) mod shutdown;

#[cfg(test)]
mod tests {
    mod cleanup {

        use crate::infra::{cleanup::*, error::Result};
        use std::fs;

        #[test]
        fn pruning_stops_before_the_shared_organization_even_when_empty() -> Result<()> {
            let dir = tempfile::tempdir()?;
            let app = dir.path().join("SEUCXCS/wlanlogin");
            let bin = app.join("data/bin");
            fs::create_dir_all(&bin)?;
            prune_empty_directories(&bin)?;
            prune_empty_directories(&bin)?;
            assert!(!app.exists());
            assert!(dir.path().join("SEUCXCS").is_dir());
            Ok(())
        }

        #[test]
        fn cleanup_rejects_redirected_organization_and_app_directories() -> Result<()> {
            for redirect_organization in [false, true] {
                let dir = tempfile::tempdir()?;
                let outside = dir.path().join("outside");
                let source = if redirect_organization {
                    outside.join("wlanlogin/state")
                } else {
                    outside.join("state")
                };
                fs::create_dir_all(&source)?;
                fs::write(source.join("daemon.log"), "preserve")?;
                let organization = dir.path().join("SEUCXCS");
                let link = if redirect_organization {
                    organization.clone()
                } else {
                    fs::create_dir(&organization)?;
                    organization.join("wlanlogin")
                };
                crate::infra::filesystem::tests::redirect_directory(&outside, &link)?;
                let state = organization.join("wlanlogin/state");
                assert!(remove_file(&state.join("daemon.log")).is_err());
                assert!(prune_empty_directories(&state).is_err());
                assert_eq!(fs::read_to_string(source.join("daemon.log"))?, "preserve");
            }
            Ok(())
        }
    }

    mod logging {

        use crate::infra::{
            error::{Message, Result},
            logging::Format,
            scope::Scope,
        };
        use std::{
            io::Write,
            sync::{Arc, Mutex},
        };

        #[derive(Clone)]
        struct Buffer(Arc<Mutex<Vec<u8>>>);
        impl Write for Buffer {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                self.0
                    .lock()
                    .map_err(|_| std::io::Error::other("buffer poisoned"))?
                    .extend_from_slice(bytes);
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }

        #[test]
        fn logs_include_utc_level_target_and_safe_structured_diagnostics() -> Result<()> {
            let bytes = Arc::new(Mutex::new(Vec::new()));
            let writer = Buffer(bytes.clone());
            let subscriber = tracing_subscriber::fmt()
                .with_ansi(false)
                .with_writer(move || writer.clone())
                .event_format(Format {
                    scope: Some(Scope::User),
                })
                .finish();
            let error = crate::infra::error::Error::from(wlanlogin_lib::Error::Rejected {
                message: "password=never-log-this\nforged log entry".into(),
                credential_rejected: true,
            });
            tracing::subscriber::with_default(subscriber, || {
                tracing::warn!(target: "wlan::daemon", error = %error, retry_after_secs = 30, "{}", Message::CheckFailed);
            });
            let output = String::from_utf8(
                bytes
                    .lock()
                    .map_err(|_| crate::infra::error::error!("buffer poisoned"))?
                    .clone(),
            )?;
            assert_eq!(output.lines().count(), 1);
            assert_eq!(&output[10..11], "T");
            assert_eq!(&output[19..20], "Z");
            assert!(output.contains("WARN  wlan::daemon:"));
            assert!(output.contains("scope=\"user\""));
            assert!(output.contains("retry_after_secs=30"));
            assert!(output.contains("Credentials rejected"));
            assert!(!output.contains("never-log-this"));
            assert!(!output.contains("forged"));
            assert!(output.is_ascii());
            Ok(())
        }
    }

    mod scope {

        use crate::infra::scope::Scope;
        #[test]
        fn actual_elevation_selects_default_scope_and_flags_override_it() {
            assert_eq!(Scope::select(false, false, false), Scope::User);
            assert_eq!(Scope::select(false, false, true), Scope::System);
            assert_eq!(Scope::select(true, false, true), Scope::User);
            assert_eq!(Scope::select(false, true, false), Scope::System);
        }
    }

    mod locks {

        use crate::infra::{control::*, error::Result, paths::Paths};
        use std::time::Duration;

        #[tokio::test(start_paused = true)]
        async fn contended_operation_waits_until_the_lock_is_released() -> Result<()> {
            let dir = tempfile::tempdir()?;
            let paths = Paths::test(dir.path());
            let held = operation_lock(&paths).await?;
            let waiting = operation_lock(&paths);
            tokio::pin!(waiting);
            assert!(
                tokio::time::timeout(Duration::from_millis(20), &mut waiting)
                    .await
                    .is_err()
            );
            drop(held);
            let _acquired = tokio::time::timeout(Duration::from_secs(1), waiting).await??;
            Ok(())
        }

        #[tokio::test]
        async fn instance_lock_is_exclusive_and_released_on_drop() -> Result<()> {
            let dir = tempfile::tempdir()?;
            let mut paths = Paths::test(dir.path());
            if RETAIN_LOCKS_ON_PURGE {
                paths.runtime = dir.path().join("run");
                std::fs::create_dir(&paths.runtime)?;
                let legacy = std::fs::File::create(paths.runtime.join("daemon.lock"))?;
                fs2::FileExt::try_lock_exclusive(&legacy)?;
                assert!(singleton(&paths).is_err());
                drop(legacy);
            }
            let lock = singleton(&paths)?;
            assert!(singleton(&paths).is_err());
            drop(lock);
            assert!(singleton(&paths).is_ok());
            Ok(())
        }
    }

    mod ipc {
        use crate::infra::{
            control::{
                Command, CredentialState, Endpoint, Request, Response, Signal, VERSION, read,
                request, serve, status, test_connect, wake, write,
            },
            error::Result,
            paths::Paths,
        };
        use std::{fs, time::Duration};
        #[tokio::test]
        async fn status_and_session_notifications_are_authenticated_versioned_and_scope_bound()
        -> Result<()> {
            let directory = tempfile::tempdir()?;
            let paths = Paths::test(directory.path().join("user"));
            let (server, mut commands) = serve(&paths).await?;
            if let Ok(mut status) = server.status.write() {
                status.credentials = CredentialState::WaitingForUnlock;
            }
            assert_eq!(
                status(&paths).await?.credentials,
                CredentialState::WaitingForUnlock
            );
            assert!(commands.try_recv().is_err());
            let endpoint: Endpoint =
                serde_json::from_slice(&fs::read(paths.runtime.join("daemon-endpoint.json"))?)?;
            for (version, scope) in [(VERSION + 1, "user"), (VERSION, "system")] {
                let mut socket = test_connect(&endpoint.address).await?;
                write(
                    &mut socket,
                    &Request {
                        version,
                        scope: scope.into(),
                        token: endpoint.token.clone(),
                        action: Command::Stop,
                    },
                )
                .await?;
                assert!(read::<Response>(&mut socket).await.is_err());
                assert!(commands.try_recv().is_err());
            }
            request(
                &paths,
                Command::SessionReady {
                    session: "session-1".into(),
                },
            )
            .await?;
            assert_eq!(
                commands.recv().await,
                Some(Signal::SessionReady("session-1".into()))
            );
            request(
                &paths,
                Command::SessionEnd {
                    session: "session-1".into(),
                },
            )
            .await?;
            assert_eq!(
                commands.recv().await,
                Some(Signal::SessionEnd("session-1".into()))
            );
            let mut other = Paths::test(directory.path().join("system"));
            other.scope = crate::infra::scope::Scope::System;
            let (_system, mut system_commands) = serve(&other).await?;
            assert!(
                request(
                    &other,
                    Command::SessionReady {
                        session: "session-1".into()
                    }
                )
                .await
                .is_err()
            );
            wake(&paths).await;
            assert_eq!(commands.recv().await, Some(Signal::Wake));
            assert!(system_commands.try_recv().is_err());
            Ok(())
        }

        #[tokio::test]
        async fn local_ipc_requires_token_and_wakes_daemon() -> Result<()> {
            let dir = tempfile::tempdir()?;
            let paths = Paths::test(dir.path());
            let (_server, mut receiver) = serve(&paths).await?;
            let endpoint: Endpoint =
                serde_json::from_slice(&fs::read(paths.runtime.join("daemon-endpoint.json"))?)?;
            let mut stranger = test_connect(&endpoint.address).await?;
            write(
                &mut stranger,
                &Request {
                    version: VERSION,
                    scope: "user".into(),
                    token: "x".repeat(64),
                    action: Command::Wake,
                },
            )
            .await?;
            assert!(
                tokio::time::timeout(Duration::from_millis(100), receiver.recv())
                    .await
                    .is_err()
            );
            wake(&paths).await;
            assert_eq!(
                tokio::time::timeout(Duration::from_secs(1), receiver.recv()).await?,
                Some(Signal::Wake)
            );

            Ok(())
        }
    }

    #[cfg(unix)]
    pub(crate) mod unix_shutdown {
        use crate::infra::{
            error::Result,
            shutdown::{RUNTIME_GRACE, Shutdown, until_stopped},
        };
        use std::time::Duration;
        use std::{
            path::Path,
            process::{Child, Command, Stdio},
            time::Instant,
        };

        struct TestChild(Child);
        impl Drop for TestChild {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }

        pub(in crate::infra) fn assert_signal_shutdown(
            send: impl FnOnce(u32) -> Result<()>,
        ) -> Result<()> {
            let dir = tempfile::tempdir()?;
            let mut child = TestChild(
                Command::new(std::env::current_exe()?)
                    .args([
                        "--ignored",
                        "--exact",
                        "infra::shutdown::tests::signal_child",
                        "--nocapture",
                    ])
                    .env("WLAN_SHUTDOWN_TEST_DIR", dir.path())
                    .stdin(Stdio::null())
                    .spawn()?,
            );
            let deadline = Instant::now() + Duration::from_secs(8);
            while !dir.path().join("ready").exists() {
                crate::infra::error::ensure!(
                    child.0.try_wait()?.is_none(),
                    "signal test child exited before ready"
                );
                crate::infra::error::ensure!(
                    Instant::now() < deadline,
                    "signal test child did not become ready"
                );
                std::thread::sleep(Duration::from_millis(10));
            }
            send(child.0.id())?;
            loop {
                if let Some(status) = child.0.try_wait()? {
                    crate::infra::error::ensure!(
                        status.success(),
                        "signal test child failed: {status}"
                    );
                    break;
                }
                crate::infra::error::ensure!(
                    Instant::now() < deadline,
                    "daemon shutdown did not interrupt busy work"
                );
                std::thread::sleep(Duration::from_millis(10));
            }
            crate::infra::error::ensure!(
                dir.path().join("stopped").exists(),
                "child skipped graceful shutdown"
            );
            Ok(())
        }

        pub(in crate::infra) fn signal_child() -> Result<()> {
            let Some(directory) = std::env::var_os("WLAN_SHUTDOWN_TEST_DIR") else {
                return Ok(());
            };
            let directory = Path::new(&directory);
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;
            runtime.block_on(async {
                let mut shutdown = Shutdown::new()?;
                // Cancelling a wait must preserve the listener for subsequent shutdown signals.
                assert!(
                    tokio::time::timeout(Duration::from_millis(10), shutdown.wait())
                        .await
                        .is_err()
                );
                let paths = crate::infra::paths::Paths::test(directory);
                let work = async {
                    let _lock = crate::infra::control::operation_lock(&paths).await?;
                    let (started, ready) = tokio::sync::oneshot::channel();
                    tokio::task::spawn_blocking(move || {
                        let _ = started.send(());
                        // A native OS call can outlive cancellation of its async caller.
                        loop {
                            std::thread::park();
                        }
                    });
                    ready.await?;
                    std::fs::write(directory.join("ready"), [])?;
                    std::future::pending::<Result<()>>().await
                };
                until_stopped(work, shutdown.wait()).await?;
                let _lock = tokio::time::timeout(
                    Duration::from_millis(200),
                    crate::infra::control::operation_lock(&paths),
                )
                .await??;
                Ok::<_, crate::infra::error::Error>(())
            })?;
            runtime.shutdown_timeout(RUNTIME_GRACE);
            std::fs::write(directory.join("stopped"), [])?;
            Ok(())
        }
    }
}
