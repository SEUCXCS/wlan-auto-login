#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
use linux::Monitor as NativeMonitor;
#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
use windows::Monitor as NativeMonitor;
#[cfg(target_os = "windows")]
pub use windows::current_sid;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
use macos::Monitor as NativeMonitor;
#[cfg(any(target_os = "windows", target_os = "macos", test))]
mod worker;
#[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
compile_error!("Supported platforms: Linux, macOS and Windows");

pub const TARGET_SSID: &[u8] = b"Student_CX";

const QUERY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

pub struct Monitor(NativeMonitor);

impl Monitor {
    pub async fn new() -> anyhow::Result<Self> {
        use anyhow::Context;
        tokio::time::timeout(QUERY_TIMEOUT, NativeMonitor::new())
            .await
            .context("初始化 Wi-Fi 监听超时")?
            .map(Self)
    }

    pub async fn snapshot(&self) -> anyhow::Result<Option<String>> {
        use anyhow::Context;
        tokio::time::timeout(QUERY_TIMEOUT, self.0.snapshot())
            .await
            .context("读取 Wi-Fi 状态超时")?
    }

    pub async fn changed(&mut self) -> anyhow::Result<()> {
        self.0.changed().await
    }
}

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
