#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::Monitor;
#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
pub use windows::{Monitor, current_sid};
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::Monitor;
#[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
compile_error!("Supported platforms: Linux, macOS and Windows");

pub const TARGET_SSID: &[u8] = b"Student_CX";

/// Only an explicitly identified Student_CX connection returns a session.
pub trait WifiConnection {
    fn target_session(&self) -> impl std::future::Future<Output = anyhow::Result<Option<String>>>;
}

impl WifiConnection for Monitor {
    async fn target_session(&self) -> anyhow::Result<Option<String>> {
        self.snapshot().await
    }
}

pub struct WifiProbe(Result<Monitor, String>);

impl WifiProbe {
    pub async fn new() -> Self {
        Self(Monitor::new().await.map_err(|error| format!("{error:#}")))
    }
}

impl WifiConnection for WifiProbe {
    async fn target_session(&self) -> anyhow::Result<Option<String>> {
        match &self.0 {
            Ok(monitor) => monitor.snapshot().await,
            Err(message) => anyhow::bail!("{message}"),
        }
    }
}

pub enum WifiState {
    Target,
    Other,
    Unavailable(String),
}

impl WifiState {
    pub fn is_target(&self) -> bool {
        matches!(self, Self::Target)
    }

    pub fn notice(&self) -> Option<String> {
        match self {
            Self::Target => None,
            Self::Other => Some("当前未连接 Student_CX，仅保存凭据，不执行校园网认证。".into()),
            Self::Unavailable(reason) => Some(format!(
                "无法确认当前 Wi-Fi（{reason}）；仅保存凭据，不执行校园网认证。"
            )),
        }
    }
}

pub async fn wifi_state(wifi: &impl WifiConnection) -> WifiState {
    match wifi.target_session().await {
        Ok(Some(_)) => WifiState::Target,
        Ok(None) => WifiState::Other,
        Err(error) => WifiState::Unavailable(format!("{error:#}")),
    }
}
