//! Execute bounded service-manager commands and expose a testable runner.
//! In-memory service-manager responses shared by unit tests.
use crate::infra::error::Message;
use crate::infra::error::{Context, Result, bail};
use std::{
    ffi::{OsStr, OsString},
    time::Duration,
};

#[derive(Clone, Debug)]
pub(crate) struct Invocation {
    pub(crate) program: OsString,
    pub(crate) args: Vec<OsString>,
    pub(crate) optional: bool,
}
impl Invocation {
    pub(crate) fn new(
        program: impl AsRef<OsStr>,
        args: impl IntoIterator<Item = impl AsRef<OsStr>>,
    ) -> Self {
        Self {
            program: program.as_ref().to_owned(),
            args: args.into_iter().map(|v| v.as_ref().to_owned()).collect(),
            optional: false,
        }
    }
    pub(crate) fn optional(mut self) -> Self {
        self.optional = true;
        self
    }
    pub(crate) async fn run(&self) -> Result<()> {
        let result = self.output().await?;
        if !self.optional {
            result.check(self)?;
        }
        Ok(())
    }
    pub(crate) async fn output(&self) -> Result<CommandOutput> {
        let mut command = tokio::process::Command::new(&self.program);
        command.args(&self.args).kill_on_drop(true);
        #[cfg(windows)]
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW, available on Win7.
        let result = tokio::time::timeout(Duration::from_secs(30), command.output())
            .await
            .context(Message::ServiceManagerCommandTimedOut)??;
        Ok(CommandOutput {
            success: result.status.success(),
            stdout: String::from_utf8_lossy(&result.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&result.stderr).into_owned(),
        })
    }
}

pub(crate) struct CommandOutput {
    pub(crate) success: bool,
    pub(crate) stdout: String,
    pub(crate) stderr: String,
}
impl CommandOutput {
    pub(crate) fn check(&self, command: &Invocation) -> Result<()> {
        if !self.success {
            bail!(Message::CommandFailed {
                value0: (command.program.to_string_lossy()).to_string(),
                value1: format!("{:?}", command.args),
                value2: (if self.stderr.trim().is_empty() {
                    self.stdout.trim()
                } else {
                    self.stderr.trim()
                })
                .to_string()
            });
        }
        Ok(())
    }
}

pub(crate) trait Runner {
    fn output(
        &self,
        command: &Invocation,
    ) -> impl std::future::Future<Output = Result<CommandOutput>>;
}

pub(crate) struct SystemRunner;
impl Runner for SystemRunner {
    async fn output(&self, command: &Invocation) -> Result<CommandOutput> {
        command.output().await
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::{CommandOutput, Invocation, Runner};
    use crate::infra::error::Message;
    use crate::infra::error::{Context, Result};
    use std::{cell::RefCell, collections::VecDeque};

    #[derive(Default)]
    pub(crate) struct FakeRunner {
        pub outputs: RefCell<VecDeque<CommandOutput>>,
        pub calls: RefCell<Vec<Invocation>>,
    }
    impl FakeRunner {
        pub fn new(outputs: impl IntoIterator<Item = (bool, &'static str)>) -> Self {
            Self {
                outputs: RefCell::new(
                    outputs
                        .into_iter()
                        .map(|(success, stdout)| CommandOutput {
                            success,
                            stdout: stdout.into(),
                            stderr: String::new(),
                        })
                        .collect(),
                ),
                calls: RefCell::new(Vec::new()),
            }
        }
    }
    impl Runner for FakeRunner {
        async fn output(&self, command: &Invocation) -> Result<CommandOutput> {
            self.calls.borrow_mut().push(command.clone());
            self.outputs
                .borrow_mut()
                .pop_front()
                .context(Message::UnexpectedServiceCommand)
        }
    }

    pub const RUNNING: &str = "LoadState=loaded\nActiveState=active\nSubState=running\nUnitFileState=enabled\nMainPID=123\nResult=success\n";
    pub const STOPPED: &str = "LoadState=loaded\nActiveState=inactive\nSubState=dead\nUnitFileState=enabled\nMainPID=0\nResult=success\n";
}
