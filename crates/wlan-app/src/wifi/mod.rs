use crate::infra::error::Message;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
use linux::Monitor as NativeMonitor;
#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
use windows::Monitor as NativeMonitor;
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
    pub async fn new() -> crate::infra::error::Result<Self> {
        use crate::infra::error::Context;
        tokio::time::timeout(QUERY_TIMEOUT, NativeMonitor::new())
            .await
            .context(Message::WiFiMonitorInitializationTimedOut)?
            .map(Self)
    }

    pub async fn snapshot(&self) -> crate::infra::error::Result<Option<String>> {
        use crate::infra::error::Context;
        tokio::time::timeout(QUERY_TIMEOUT, self.0.snapshot())
            .await
            .context(Message::WiFiStatusQueryTimedOut)?
    }

    pub async fn changed(&mut self) -> crate::infra::error::Result<()> {
        self.0.changed().await
    }
}

/// Only an explicitly identified Student_CX connection returns a session.
pub trait WifiConnection {
    fn target_session(
        &self,
    ) -> impl std::future::Future<Output = crate::infra::error::Result<Option<String>>>;
}

impl WifiConnection for Monitor {
    async fn target_session(&self) -> crate::infra::error::Result<Option<String>> {
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
    async fn target_session(&self) -> crate::infra::error::Result<Option<String>> {
        match &self.0 {
            Ok(monitor) => monitor.snapshot().await,
            Err(message) => crate::infra::error::bail!("{message}"),
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
            Self::Other => Some(
                "Not connected to Student_CX; credentials will be saved without authentication"
                    .into(),
            ),
            Self::Unavailable(reason) => Some(
                (Message::WiFiIdentificationUnavailableReasonCredentialsWill {
                    reason: (reason).to_string(),
                })
                .to_string(),
            ),
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
