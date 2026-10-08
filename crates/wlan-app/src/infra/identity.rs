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

// Shared with xtask, including when the app itself is built for another OS.
#[allow(dead_code)]
pub const MACOS_BUNDLE_DIRECTORIES: &[&str] = &[
    "Contents",
    "Contents/MacOS",
    "Contents/Resources",
    "Contents/Library",
    "Contents/Library/LaunchAgents",
    "Contents/Library/LaunchDaemons",
    "Contents/_CodeSignature",
];
#[allow(dead_code)]
pub const MACOS_BUNDLE_FILES: &[&str] = &[
    "Contents/Info.plist",
    "Contents/PkgInfo",
    "Contents/MacOS/wlanlogin",
    "Contents/Library/LaunchAgents/dev.cxcs.tools.wlanlogin.plist",
    "Contents/Library/LaunchDaemons/dev.cxcs.tools.wlanlogin.plist",
    "Contents/_CodeSignature/CodeResources",
];

// These definitions are written before signing and never changed at installation.
#[allow(dead_code)]
pub fn macos_service_path(system: bool) -> &'static str {
    if system {
        "Contents/Library/LaunchDaemons/dev.cxcs.tools.wlanlogin.plist"
    } else {
        "Contents/Library/LaunchAgents/dev.cxcs.tools.wlanlogin.plist"
    }
}

#[allow(dead_code)]
pub fn macos_service_plist(system: bool) -> String {
    let template = if system {
        include_str!("../../../../packaging/macos/LaunchDaemon.plist")
    } else {
        include_str!("../../../../packaging/macos/LaunchAgent.plist")
    };
    template
        .replace("@APP_ID@", APP_ID)
        .replace("@APP_NAME@", APP_NAME)
}
