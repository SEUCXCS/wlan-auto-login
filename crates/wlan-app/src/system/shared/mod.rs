//! Reusable implementations behind the system contracts.
#[cfg(target_os = "linux")]
#[path = "../linux/credentials/scoped.rs"]
pub(crate) mod credentials;
#[cfg(target_os = "macos")]
#[path = "../macos/credentials/scoped.rs"]
pub(crate) mod credentials;
#[cfg(target_os = "windows")]
#[path = "../windows/credentials/scoped.rs"]
pub(crate) mod credentials;
pub(crate) mod flow;
pub(crate) mod services;
pub(crate) mod wifi;
