//! Stable native identifiers and names shared by all application adapters.
pub const APP_ID: &str = "dev.cxcs.tools.wlanlogin";
pub const ORGANIZATION: &str = "cxcs-tools";
pub const APP_NAME: &str = "wlanlogin";
pub const CREDENTIAL_ENTRY: &str = "saved-login";
#[cfg(target_os = "macos")]
pub const APP_BUNDLE: &str = "wlanlogin.app";

#[cfg(windows)]
pub const DAEMON_BINARY: &str = "wlanlogin-daemon.exe";
#[cfg(not(windows))]
pub const DAEMON_BINARY: &str = "wlanlogin-daemon";

#[cfg(any(target_os = "linux", test))]
pub const SYSTEMD_UNIT: &str = "dev.cxcs.tools.wlanlogin.service";
#[cfg(target_os = "macos")]
pub const LAUNCHD_PLIST: &str = "dev.cxcs.tools.wlanlogin.plist";
#[cfg(target_os = "macos")]
pub const SYSTEM_CREDENTIAL_SERVICE: &str = "dev.cxcs.tools.wlanlogin.system";
#[cfg(target_os = "macos")]
pub const NETWORK_QUEUE: &str = "dev.cxcs.tools.wlanlogin.network";
