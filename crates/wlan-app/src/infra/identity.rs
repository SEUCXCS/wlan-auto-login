//! Stable native identifiers and names shared by all application adapters.
pub const APP_ID: &str = "dev.cxcs.tools.wlanlogin";
pub const ORGANIZATION: &str = "SEUCXCS";
/// Command, executable and directory name; keep separate from the display name.
pub const APP_NAME: &str = "wlanlogin";
pub const DISPLAY_NAME: &str = "CXCS WLAN Login";
pub const CREDENTIAL_ENTRY: &str = "saved-login";
#[allow(dead_code)]
pub const APP_BUNDLE: &str = "wlanlogin.app";

#[allow(dead_code)]
pub const SYSTEMD_UNIT: &str = "dev.cxcs.tools.wlanlogin.service";
#[allow(dead_code)]
pub const LAUNCHD_PLIST: &str = "dev.cxcs.tools.wlanlogin.plist";
#[allow(dead_code)]
pub const SYSTEM_CREDENTIAL_SERVICE: &str = "dev.cxcs.tools.wlanlogin.system";
#[allow(dead_code)]
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
    "Contents/Resources/icon.icns",
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

#[cfg(target_os = "linux")]
#[path = "../system/linux/runtime/infra/identity.rs"]
mod platform;
#[cfg(target_os = "macos")]
#[path = "../system/macos/runtime/infra/identity.rs"]
mod platform;
#[cfg(target_os = "windows")]
#[path = "../system/windows/runtime/infra/identity.rs"]
mod platform;
#[cfg(not(target_os = "macos"))]
#[allow(unused_imports)]
pub(crate) use platform::DAEMON_BINARY;
