//! Stop barriers and error collection shared by file and signed-bundle rollback.
use crate::infra::error::Result;

#[derive(Default)]
pub(crate) struct Rollback {
    failures: Vec<String>,
}
impl Rollback {
    pub fn safe(&self) -> bool {
        self.failures.is_empty()
    }

    pub fn record(&mut self, result: Result<()>) {
        if let Err(error) = result {
            self.failures.push(error.to_string());
        }
    }

    /// Once stopping/unregistration fails, dependent file operations are forbidden.
    pub async fn step(&mut self, operation: impl std::future::Future<Output = Result<()>>) {
        if self.safe() {
            self.record(operation.await);
        }
    }

    pub fn detail(&self) -> String {
        self.failures.join("; ")
    }
}
