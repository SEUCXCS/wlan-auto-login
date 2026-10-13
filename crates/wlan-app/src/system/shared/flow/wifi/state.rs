use crate::system::define::wifi::{WifiConnection, WifiState};

pub async fn run(wifi: &impl WifiConnection) -> WifiState {
    // Query identity only; an unreadable SSID must not authorize credential submission.
    match wifi.target_session().await {
        Ok(Some(_)) => WifiState::Target,
        Ok(None) => WifiState::Other,
        Err(error) => WifiState::Unavailable(format!("{error:#}")),
    }
}
