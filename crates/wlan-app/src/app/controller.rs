//! Shared application operations for command-line and terminal interfaces.
pub(crate) mod daemon;
pub(crate) mod login;
pub(crate) mod logout;
#[cfg(any(target_os = "macos", test))]
pub(crate) mod permission;
pub(crate) mod service;
