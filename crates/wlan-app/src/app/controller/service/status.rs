//! App exit policy around the selected platform's status workflow.
use crate::infra::{
    error::{Message, Result},
    paths::Paths,
};
pub(crate) use crate::system::define::services::InstallationReport;
#[cfg(test)]
pub(crate) use crate::system::flow::status::inspect;

pub async fn run(paths: &Paths, discover: bool) -> Result<Vec<InstallationReport>> {
    crate::system::flow::status::run(paths, discover).await
}

pub fn check(reports: &[InstallationReport]) -> Result<()> {
    let errors = reports
        .iter()
        .filter_map(|report| report.state.as_ref().and_then(|state| state.as_ref().err()))
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    crate::infra::error::ensure!(
        errors.is_empty(),
        Message::FailedToDetermineServiceStatus {
            value0: errors.join("; ")
        }
    );
    Ok(())
}
