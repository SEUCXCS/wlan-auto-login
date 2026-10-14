//! Native permission adapter selected by the platform workflow.
mod provider;
pub(crate) use provider::Permissions;
#[cfg(target_os = "macos")]
pub(crate) mod location;
