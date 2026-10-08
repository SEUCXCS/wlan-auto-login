//! Query the signed GUI-session host, including when called from the terminal.
pub(super) mod native;
use super::host;
use crate::infra::error::Result;
use std::{sync::Mutex, time::Duration};

pub struct Monitor {
    observed: Mutex<Option<std::result::Result<Option<String>, String>>>,
}
impl Monitor {
    pub async fn new() -> Result<Self> {
        Ok(Self {
            observed: Mutex::new(None),
        })
    }
    pub async fn snapshot(&self) -> Result<Option<String>> {
        let result = host::snapshot().await;
        if let Ok(mut previous) = self.observed.lock() {
            *previous = Some(
                result
                    .as_ref()
                    .map(Clone::clone)
                    .map_err(ToString::to_string),
            );
        }
        result
    }
    pub async fn changed(&mut self) -> Result<()> {
        loop {
            tokio::time::sleep(Duration::from_secs(1)).await;
            let current = host::snapshot().await.map_err(|error| error.to_string());
            let changed = if let Ok(mut previous) = self.observed.lock() {
                let changed = previous.as_ref() != Some(&current);
                *previous = Some(current.clone());
                changed
            } else {
                true
            };
            if changed {
                return current
                    .map(|_| ())
                    .map_err(crate::infra::error::Error::message);
            }
        }
    }
}
impl crate::system::define::wifi::WifiConnection for Monitor {
    async fn target_session(&self) -> Result<Option<String>> {
        self.snapshot().await
    }
}
impl crate::system::define::wifi::WifiMonitor for Monitor {
    async fn changed(&mut self) -> Result<()> {
        self.changed().await
    }
}
