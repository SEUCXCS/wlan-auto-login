use crate::infra::error::Message;
use crate::infra::error::{Context, Result};
use std::{future::Future, time::Duration};

// Native keychain calls on Tokio's blocking pool cannot be forcibly cancelled.
// Leave time for cleanup, but never wait forever when dropping the runtime.
pub const RUNTIME_GRACE: Duration = Duration::from_secs(2);

pub struct Shutdown {
    terminate: tokio::signal::unix::Signal,

    interrupt: tokio::signal::unix::Signal,
}

impl Shutdown {
    // Register before doing any daemon work, and retain these listeners until exit.
    pub fn new() -> Result<Self> {
        {
            use tokio::signal::unix::{SignalKind, signal};
            Ok(Self {
                terminate: signal(SignalKind::terminate())
                    .context(Message::FailedToRegisterSIGTERMListener)?,
                interrupt: signal(SignalKind::interrupt())
                    .context(Message::FailedToRegisterSIGINTListener)?,
            })
        }
    }

    pub async fn wait(&mut self) -> Result<()> {
        let received = tokio::select! {
            value = self.terminate.recv() => value,
            value = self.interrupt.recv() => value,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::tests::unix_shutdown;

    #[test]
    fn termination_interrupts_busy_work_and_stalled_blocking_tasks() -> Result<()> {
        for signal in [libc::SIGTERM, libc::SIGINT] {
            unix_shutdown::assert_signal_shutdown(|pid| {
                // SAFETY: the harness supplies the live PID of its test child.
                crate::infra::error::ensure!(
                    unsafe { libc::kill(pid as i32, signal) } == 0,
                    "cannot signal test child"
                );
                Ok(())
            })?;
        }
        Ok(())
    }

    #[test]
    #[ignore = "subprocess helper; invoked by termination_interrupts_busy_work_and_stalled_blocking_tasks"]
    fn signal_child() -> Result<()> {
        unix_shutdown::signal_child()
    }
}
