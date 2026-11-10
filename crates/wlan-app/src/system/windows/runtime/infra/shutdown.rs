use crate::infra::error::Message;
use crate::infra::error::{Context, Result};
use std::{future::Future, time::Duration};

// Native keychain calls on Tokio's blocking pool cannot be forcibly cancelled.
// Leave time for cleanup, but never wait forever when dropping the runtime.
pub const RUNTIME_GRACE: Duration = Duration::from_secs(2);

pub struct Shutdown {
    console: Option<ConsoleSignals>,
}

struct ConsoleSignals {
    interrupt: tokio::signal::windows::CtrlC,
    close: tokio::signal::windows::CtrlClose,
    ctrl_break: tokio::signal::windows::CtrlBreak,
}

impl Shutdown {
    // Register before doing any daemon work, and retain these listeners until exit.
    pub fn new() -> Result<Self> {
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
