//! Platform extensions around the steps owned by shared workflows.
use crate::infra::error::{Error, Result};

pub(crate) trait Hooks<Step, Context> {
    async fn before(&self, _step: Step, _context: &Context) -> Result<()> {
        Ok(())
    }

    async fn after(
        &self,
        _step: Step,
        _context: &Context,
        _result: std::result::Result<(), &Error>,
    ) -> Result<()> {
        Ok(())
    }
}

pub(crate) struct NoHooks;
impl<S, C> Hooks<S, C> for NoHooks {}

pub(crate) async fn step<S: Copy, C, T>(
    hooks: &impl Hooks<S, C>,
    stage: S,
    context: &C,
    operation: impl Future<Output = Result<T>>,
) -> Result<T> {
    hooks.before(stage, context).await?;
    let result = operation.await;
    let after = hooks
        .after(stage, context, result.as_ref().map(|_| ()))
        .await;
    let value = result?;
    after?;
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    struct RecordingHooks {
        fail_before: bool,
    }
    impl Hooks<(), RefCell<Vec<&'static str>>> for RecordingHooks {
        async fn before(&self, (): (), log: &RefCell<Vec<&'static str>>) -> Result<()> {
            log.borrow_mut().push("before");
            crate::infra::error::ensure!(!self.fail_before, "before failed");
            Ok(())
        }
        async fn after(
            &self,
            (): (),
            log: &RefCell<Vec<&'static str>>,
            result: std::result::Result<(), &Error>,
        ) -> Result<()> {
            log.borrow_mut().push(if result.is_ok() {
                "after"
            } else {
                "after failure"
            });
            Ok(())
        }
    }

    #[tokio::test]
    async fn hooks_bracket_attempted_steps() {
        for (fail_before, fail_work, expected) in [
            (true, false, &["before"][..]),
            (false, true, &["before", "work", "after failure"][..]),
            (false, false, &["before", "work", "after"][..]),
        ] {
            let log = RefCell::new(Vec::new());
            let result = step(&RecordingHooks { fail_before }, (), &log, async {
                log.borrow_mut().push("work");
                crate::infra::error::ensure!(!fail_work, "work failed");
                Ok(())
            })
            .await;
            assert_eq!(result.is_err(), fail_before || fail_work);
            assert_eq!(*log.borrow(), expected);
        }
    }
}
