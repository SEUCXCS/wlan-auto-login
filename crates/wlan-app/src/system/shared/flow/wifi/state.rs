use super::Step;
use crate::system::define::wifi::{WifiConnection, WifiState};
use crate::system::flow::{NativeWifi, hooks::step};

pub async fn run(wifi: &impl WifiConnection) -> WifiState {
    // Query identity only; an unreadable SSID must not authorize credential submission.
    match step(&NativeWifi, Step::State, wifi, wifi.target_session()).await {
        Ok(Some(_)) => WifiState::Target,
        Ok(None) => WifiState::Other,
        Err(error) => WifiState::Unavailable(format!("{error:#}")),
    }
}
