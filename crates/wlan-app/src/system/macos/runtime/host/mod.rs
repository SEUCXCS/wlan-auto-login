//! A credential-free Wi-Fi host launched in the user's GUI session.
//! Root clients authenticate the helper's live code identity and active console user.
use crate::infra::{
    error::{Context, Message, Result, ensure},
    filesystem::private_dir,
    paths::{self, Paths},
    scope::Scope,
};
use crate::system::macos::{
    permission::location::{Location, Status},
    services::bundle,
    wifi::native,
};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    os::unix::{
        ffi::OsStrExt,
        fs::{FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt},
    },
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{UnixListener, UnixStream},
    sync::{Mutex, Semaphore},
    task::JoinHandle,
    time::Instant,
};

static SYSTEM: AtomicBool = AtomicBool::new(false);
static REQUEST: AtomicU64 = AtomicU64::new(1);
static STARTUP: Mutex<()> = Mutex::const_new(());

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Foreground,
    Service,
    System,
}
impl Role {
    fn name(self) -> &'static str {
        match self {
            Self::Foreground => "foreground",
            Self::Service => "service",
            Self::System => "system",
        }
    }
}

pub fn configure(scope: Scope) {
    SYSTEM.store(scope == Scope::System, Ordering::Release);
}
pub(crate) fn scope() -> Scope {
    if SYSTEM.load(Ordering::Acquire) {
        Scope::System
    } else {
        Scope::User
    }
}
fn socket(runtime: &Path, role: Role) -> PathBuf {
    runtime.join(format!("wifi-{}.sock", role.name()))
}

mod client;
mod identity;
mod launch;
mod protocol;
mod server;
pub(crate) use client::{location_permission, prepare, snapshot, stop_helpers};
use identity::{Session, session};
use launch::launch_host;
#[cfg(test)]
use launch::{host_launch, launch};
use protocol::*;
pub(crate) use server::Host;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_interactive_authorization_launches_in_the_foreground() {
        for scope in [Scope::User, Scope::System] {
            let foreground = host_launch(Path::new("/Applications/Test.app"), scope, 501, true);
            let background = host_launch(Path::new("/Applications/Test.app"), scope, 501, false);
            assert!(!foreground.args.iter().any(|arg| arg == "-g"));
            assert!(background.args.iter().any(|arg| arg == "-g"));
            assert!(
                foreground
                    .args
                    .windows(3)
                    .any(|args| { args == ["--args", "daemon", "--internal-wifi-host"] })
            );
            if scope == Scope::System {
                assert_eq!(foreground.program, "/bin/launchctl");
                assert!(foreground.args.iter().any(|arg| arg == "#501"));
                assert!(foreground.args.iter().any(|arg| arg == "--system-helper"));
            }
        }
    }

    #[tokio::test]
    async fn host_launch_reports_failures_without_retrying() -> Result<()> {
        use crate::infra::command::tests::FakeRunner;
        let app = Path::new("/Applications/Test.app");
        for success in [false, true] {
            let runner = FakeRunner::new([(success, "")]);
            assert_eq!(
                launch(app, Scope::User, 501, false, &runner).await.is_ok(),
                success
            );
            let calls = runner.calls.borrow();
            assert_eq!(calls.len(), 1);
            assert_eq!(calls[0].program, "/usr/bin/open");
            assert!(
                calls[0]
                    .args
                    .windows(3)
                    .any(|args| args == ["--args", "daemon", "--internal-wifi-host"])
            );
        }
        Ok(())
    }

    #[tokio::test]
    async fn ipc_rejects_oversized_frames_without_allocating_the_payload() -> Result<()> {
        let (mut writer, mut reader) = tokio::io::duplex(64);
        writer.write_u32(u32::MAX).await?;
        assert!(read_frame::<Request>(&mut reader).await.is_err());
        Ok(())
    }
    #[tokio::test]
    async fn response_preserves_request_identity_and_permission_failure() -> Result<()> {
        let (mut writer, mut reader) = tokio::io::duplex(1024);
        let response = Response {
            version: VERSION,
            id: 19,
            location: Status::Denied,
            session: None,
            error: Some("denied".into()),
        };
        write_frame(&mut writer, &response).await?;
        let decoded: Response = read_frame(&mut reader).await?;
        assert_eq!(decoded.id, 19);
        assert_eq!(decoded.location, Status::Denied);
        assert!(decoded.session.is_none());
        Ok(())
    }
}
