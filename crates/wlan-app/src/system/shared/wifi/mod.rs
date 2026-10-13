//! Bounded Wi-Fi queries and a single worker for blocking native APIs.
use crate::infra::error::Message;
pub(crate) use crate::system::define::wifi::WifiConnection;
use crate::system::native::wifi::Monitor as NativeMonitor;
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
        crate::system::define::wifi::WifiMonitor::changed(&mut self.0).await
    }
}

impl WifiConnection for Monitor {
    async fn target_session(&self) -> crate::infra::error::Result<Option<String>> {
        self.snapshot().await
    }
}

pub struct WifiProbe(Result<Monitor, String>);

impl WifiProbe {
    pub async fn new() -> Self {
        Self(
            crate::system::flow::wifi::monitor::run()
                .await
                .map_err(|error| format!("{error:#}")),
        )
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

pub(crate) use crate::system::native::flow::wifi::state::run as wifi_state;

#[cfg(any(target_os = "windows", target_os = "macos", test))]
mod worker;
#[cfg(any(target_os = "windows", target_os = "macos"))]
pub(crate) use worker::Worker;
