//! Default workflows composed from shared steps and native adapters.
pub(crate) mod control;
pub(crate) mod credentials;
#[cfg(any(not(target_os = "macos"), test))]
pub(crate) mod install;
pub(crate) mod logs;
pub(crate) mod permission;
pub(crate) mod restart;
#[cfg(not(target_os = "macos"))]
pub(crate) mod run;
pub(crate) mod start;
pub(crate) mod status;
pub(crate) mod stop;
pub(crate) mod uninstall;
pub(crate) mod wifi;
