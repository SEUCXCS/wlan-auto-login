//! Native process runtime support; application orchestration stays in app/bootstrap.
pub(crate) mod gui;
pub(crate) mod host;
#[cfg(target_os = "macos")]
pub(crate) mod service;
