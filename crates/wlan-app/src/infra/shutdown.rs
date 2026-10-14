use crate::infra::error::Message;
use crate::infra::error::{Context, Result};
use std::{future::Future, time::Duration};

// Native keychain calls on Tokio's blocking pool cannot be forcibly cancelled.
// Leave time for cleanup, but never wait forever when dropping the runtime.
pub const RUNTIME_GRACE: Duration = Duration::from_secs(2);

pub struct Shutdown {
    #[cfg(unix)]
    terminate: tokio::signal::unix::Signal,
    #[cfg(unix)]
    interrupt: tokio::signal::unix::Signal,
    #[cfg(windows)]
    console: Option<ConsoleSignals>,
}

#[cfg(windows)]
struct ConsoleSignals {
    interrupt: tokio::signal::windows::CtrlC,
    close: tokio::signal::windows::CtrlClose,
    ctrl_break: tokio::signal::windows::CtrlBreak,
}

impl Shutdown {
    // Register before doing any daemon work, and retain these listeners until exit.
    pub fn new() -> Result<Self> {
        #[cfg(unix)]
        {
            use tokio::signal::unix::{SignalKind, signal};
            Ok(Self {
                terminate: signal(SignalKind::terminate())
                    .context(Message::FailedToRegisterSIGTERMListener)?,
                interrupt: signal(SignalKind::interrupt())
                    .context(Message::FailedToRegisterSIGINTListener)?,
            })
        }
        #[cfg(windows)]
        {
            use tokio::signal::windows;
            // SCM services and detached scheduled tasks have no console. They
            // use SCM / IPC stops; console registration must not gate startup.
            // SAFETY: GetConsoleWindow only reads the current process attachment.
            let attached =
                unsafe { !windows_sys::Win32::System::Console::GetConsoleWindow().is_null() };
            let console = if attached {
                Some(ConsoleSignals {
                    interrupt: windows::ctrl_c().context(Message::FailedToRegisterCtrlCListener)?,
                    close: windows::ctrl_close()
                        .context(Message::FailedToRegisterConsoleCloseListener)?,
                    ctrl_break: windows::ctrl_break()
                        .context(Message::FailedToRegisterCtrlBreakListener)?,
                })
            } else {
                None
            };
            Ok(Self { console })
        }
    }

    pub async fn wait(&mut self) -> Result<()> {
        #[cfg(unix)]
        let received = tokio::select! {
            value = self.terminate.recv() => value,
            value = self.interrupt.recv() => value,
        };
        #[cfg(windows)]
        let received = tokio::select! {

            value = async {
                match &mut self.console {
                    Some(signals) => tokio::select! {
                        value = signals.interrupt.recv() => value,
                        value = signals.close.recv() => value,
                        value = signals.ctrl_break.recv() => value,
                    },
                    None => std::future::pending().await,
                }
            } => value,
        };
        received.context(Message::ProcessShutdownSignalChannelClosed)
    }
}

pub async fn until_stopped(
    work: impl Future<Output = Result<()>>,
    stop: impl Future<Output = Result<()>>,
) -> Result<()> {
    tokio::select! {
        biased;
        result = stop => {
            result?;
            Ok(())
        },
        result = work => result,
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
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

    #[test]
    fn termination_interrupts_busy_work_and_stalled_blocking_tasks() -> Result<()> {
        // Signals only target these fresh test children, never the test runner
        // or a real daemon. No portal, keyring or installed service is involved.
        for signal in [libc::SIGTERM, libc::SIGINT] {
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
            // SAFETY: this is the PID of our live child; it has installed handlers.
            crate::infra::error::ensure!(
                unsafe { libc::kill(child.0.id() as i32, signal) } == 0,
                "cannot signal test child"
            );
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
        }
        Ok(())
    }

    #[test]
    #[ignore = "subprocess helper; invoked by termination_interrupts_busy_work_and_stalled_blocking_tasks"]
    fn signal_child() -> Result<()> {
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
