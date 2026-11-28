//! Select host-only developer capabilities.
#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
pub use windows::require_windows;
#[cfg(not(target_os = "windows"))]
pub fn require_windows() -> anyhow::Result<()> {
    anyhow::bail!("Windows packaging requires a Windows host with the Windows SDK")
}
#[cfg(target_os = "macos")]
mod macos;
#[cfg(not(target_os = "macos"))]
mod other;
#[cfg(target_os = "macos")]
pub use macos::require_macos;
#[cfg(not(target_os = "macos"))]
pub use other::require_macos;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::{require_linux, set_mode};
#[cfg(not(target_os = "linux"))]
pub fn require_linux() -> anyhow::Result<()> {
    anyhow::bail!("Linux packaging requires a Linux host")
}
#[cfg(not(target_os = "linux"))]
pub fn set_mode(_path: &std::path::Path, _mode: u32) -> anyhow::Result<()> {
    require_linux()
}
