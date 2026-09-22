#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::Monitor;
#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
pub use windows::Monitor;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::Monitor;
#[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
compile_error!("Supported platforms: Linux, macOS and Windows");

pub const TARGET_SSID: &[u8] = b"Student_CX";
