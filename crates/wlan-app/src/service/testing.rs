//! In-memory service-manager responses shared by unit tests.
use super::command::{CommandOutput, Invocation, Runner};
use crate::infra::error::Message;
use crate::infra::error::{Context, Result};
use std::{cell::RefCell, collections::VecDeque};

#[derive(Default)]
pub(in crate::service) struct FakeRunner {
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
