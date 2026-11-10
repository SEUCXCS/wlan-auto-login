//! Native definition decoding and executable inspection.
use crate::infra::error::Result;
use crate::system::define::services::Plan;
use std::path::Path;
pub(crate) fn read_definition(path: &Path) -> std::io::Result<String> {
    std::fs::read_to_string(path)
}
pub(crate) fn inspect_executable(_plan: &Plan, executable: bool) -> Result<bool> {
    Ok(executable)
}
