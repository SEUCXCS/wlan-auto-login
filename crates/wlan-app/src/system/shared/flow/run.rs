//! Run the application's daemon after preparing the native process environment.
use super::hooks::{Hooks, NoHooks, step};
use crate::infra::{error::Result, paths::Paths};

#[derive(Clone, Copy)]
pub(crate) enum Step {
    Prepare,
    Execute,
}
pub(crate) struct RunContext<'a> {
    pub paths: &'a Paths,
    pub managed: bool,
}

pub(crate) trait RunHooks: for<'a> Hooks<Step, RunContext<'a>> {
    type Guard;
    fn prepare(&self, paths: &Paths, managed: bool) -> impl Future<Output = Result<Self::Guard>>;
}
impl RunHooks for NoHooks {
    type Guard = ();
    async fn prepare(&self, _paths: &Paths, _managed: bool) -> Result<()> {
        Ok(())
    }
}

pub(crate) async fn run(
    paths: &Paths,
    managed: bool,
    operation: impl Future<Output = Result<()>>,
) -> Result<()> {
    let context = RunContext { paths, managed };
    let hooks = super::NativeRun;
    // macOS holds its service and Wi-Fi guards here; other platforms return ().
    #[allow(clippy::let_unit_value)]
    let _guard = step(
        &hooks,
        Step::Prepare,
        &context,
        hooks.prepare(context.paths, context.managed),
    )
    .await?;
    step(&hooks, Step::Execute, &context, operation).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn foreground_run_drives_the_supplied_operation() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let paths = Paths::test(directory.path());
        let mut ran = false;
        run(&paths, false, async {
            ran = true;
            Ok(())
        })
        .await?;
        assert!(ran);
        Ok(())
    }
}
