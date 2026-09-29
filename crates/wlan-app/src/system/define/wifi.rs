//! Wireless connection and event contracts.
use crate::infra::error::Message;
pub const TARGET_SSID: &[u8] = b"Student_CX";

/// Only an explicitly identified Student_CX connection returns a session.
pub trait WifiConnection {
    fn target_session(
        &self,
    ) -> impl std::future::Future<Output = crate::infra::error::Result<Option<String>>>;
}

pub trait WifiMonitor: WifiConnection {
    fn changed(&mut self) -> impl std::future::Future<Output = crate::infra::error::Result<()>>;
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
