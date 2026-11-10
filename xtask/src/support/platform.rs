//! Select host-only developer capabilities.
#[cfg(target_os = "macos")]
mod macos;
#[cfg(not(target_os = "macos"))]
mod other;
#[cfg(target_os = "macos")]
pub use macos::require_macos;
#[cfg(not(target_os = "macos"))]
pub use other::require_macos;
