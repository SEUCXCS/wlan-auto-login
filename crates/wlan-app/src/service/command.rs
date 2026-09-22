//! Execute bounded service-manager commands and expose a testable runner.
use anyhow::{Context, Result, bail};
use std::{
    ffi::{OsStr, OsString},
    time::Duration,
};

#[derive(Clone, Debug)]
pub(super) struct Invocation {
    pub(super) program: OsString,
    pub(super) args: Vec<OsString>,
    pub(super) optional: bool,
}
impl Invocation {
    pub(super) fn new(
        program: impl AsRef<OsStr>,
        args: impl IntoIterator<Item = impl AsRef<OsStr>>,
    ) -> Self {
        Self {
            program: program.as_ref().to_owned(),
            args: args.into_iter().map(|v| v.as_ref().to_owned()).collect(),
            optional: false,
        }
    }
    pub(super) fn optional(mut self) -> Self {
        self.optional = true;
        self
    }
    pub(super) async fn run(&self) -> Result<()> {
        let result = self.output().await?;
        if !self.optional {
            result.check(self)?;
        }
        Ok(())
    }
    pub(super) async fn output(&self) -> Result<CommandOutput> {
        let mut command = tokio::process::Command::new(&self.program);
        command.args(&self.args).kill_on_drop(true);
        #[cfg(windows)]
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW, available on Win7.
        let result = tokio::time::timeout(Duration::from_secs(30), command.output())
            .await
            .context("服务管理命令超时")??;
        Ok(CommandOutput {
            success: result.status.success(),
            stdout: String::from_utf8_lossy(&result.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&result.stderr).into_owned(),
        })
    }
}

pub(super) struct CommandOutput {
    pub(super) success: bool,
    pub(super) stdout: String,
    pub(super) stderr: String,
}
impl CommandOutput {
    pub(super) fn check(&self, command: &Invocation) -> Result<()> {
        if !self.success {
            bail!(
                "{} {:?} 执行失败：{}",
                command.program.to_string_lossy(),
                command.args,
                if self.stderr.trim().is_empty() {
                    self.stdout.trim()
                } else {
                    self.stderr.trim()
                }
            );
        }
        Ok(())
    }
}

pub(super) trait Runner {
    fn output(
        &self,
        command: &Invocation,
    ) -> impl std::future::Future<Output = Result<CommandOutput>>;
}

pub(super) struct SystemRunner;
impl Runner for SystemRunner {
    async fn output(&self, command: &Invocation) -> Result<CommandOutput> {
        command.output().await
    }
}
