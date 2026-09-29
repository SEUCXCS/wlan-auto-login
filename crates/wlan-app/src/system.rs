//! Select native system adapters at compile time.
pub(crate) mod define;
#[cfg(any(target_os = "linux", test))]
pub(crate) mod linux;
#[cfg(any(target_os = "macos", test))]
pub(crate) mod macos;
#[cfg(any(target_os = "windows", test))]
pub(crate) mod windows;

#[cfg(target_os = "linux")]
#[allow(unused_imports)]
pub(crate) use linux as native;
#[cfg(target_os = "macos")]
#[allow(unused_imports)]
pub(crate) use macos as native;
#[cfg(target_os = "windows")]
#[allow(unused_imports)]
pub(crate) use windows as native;
#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
compile_error!("Supported platforms: Linux, macOS and Windows");
