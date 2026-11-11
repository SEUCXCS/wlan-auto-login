pub(crate) mod app_service;
pub(crate) mod definition;
#[cfg(test)]
pub(crate) mod logs;
pub(crate) mod manager;

#[cfg(target_os = "macos")]
pub(crate) mod bundle;
#[cfg(target_os = "macos")]
pub(crate) mod payload;
#[cfg(target_os = "macos")]
pub(crate) mod readiness;

#[cfg(target_os = "macos")]
pub(crate) mod artifacts;
#[cfg(target_os = "macos")]
mod dispatch;

#[cfg(target_os = "macos")]
pub(crate) use dispatch::execute_optional;
