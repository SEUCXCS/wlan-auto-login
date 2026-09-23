//! Install and manage operating-system services; command workflows live in app.
mod command;
mod definition;
mod install;
mod lifecycle;
mod logs;
mod manager;
mod status;
#[cfg(test)]
mod testing;
mod uninstall;
#[cfg(windows)]
pub(crate) mod windows;

pub(crate) use definition::ensure_no_conflicting_install;
pub(crate) use install::install;
pub(crate) use lifecycle::{Action, manage};
pub(crate) use logs::{LogOptions, logs};
pub(crate) use status::status;
pub(crate) use uninstall::uninstall;
