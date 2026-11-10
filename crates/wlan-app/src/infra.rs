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
#[cfg(target_os = "linux")]
#[path = "system/linux/runtime/infra/logging.rs"]
pub(crate) mod logging;
#[cfg(target_os = "macos")]
#[path = "system/macos/runtime/infra/logging.rs"]
pub(crate) mod logging;
#[cfg(target_os = "windows")]
#[path = "system/windows/runtime/infra/logging.rs"]
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
                .event_format(Format)
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
            let paths = Paths::test(dir.path());
            let lock = singleton(&paths)?;
            assert!(singleton(&paths).is_err());
            drop(lock);
            assert!(singleton(&paths).is_ok());
            Ok(())
        }
    }
}
