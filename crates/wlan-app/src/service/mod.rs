//! Install and manage operating-system services; command workflows live in app.
use crate::infra::command;
mod logs;
use crate::system::define::services as manager;
use crate::system::shared::services::artifacts as definition;
mod status;

#[cfg(windows)]
pub(crate) use crate::system::windows::services::runtime as windows;

pub(crate) use crate::system::shared::services::ensure_no_conflicting_install;
pub(crate) use crate::system::shared::services::install;
pub(crate) use crate::system::shared::services::uninstall;
pub(crate) use crate::system::shared::services::{Action, manage};
pub(crate) use logs::{LogOptions, logs};
pub(crate) use status::status;
