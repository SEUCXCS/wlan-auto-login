//! Bounded Wi-Fi queries and a single worker for blocking native APIs.
use crate::infra::error::Message;
pub(crate) use crate::system::define::wifi::{WifiConnection, wifi_state};
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

#[cfg(any(target_os = "windows", target_os = "macos", test))]
use crate::infra::error::{Context, Result};
#[cfg(any(target_os = "windows", target_os = "macos", test))]
use tokio::sync::{mpsc, oneshot};

/// Own native handles on one thread. Blocking OS calls and handle cleanup must
/// never occupy the daemon's event loop or prevent the process from exiting.
#[cfg(any(target_os = "windows", target_os = "macos", test))]
pub(crate) struct Worker<T> {
    requests: mpsc::Sender<oneshot::Sender<Result<T>>>,
}

#[cfg(any(target_os = "windows", target_os = "macos", test))]
impl<T: Send + 'static> Worker<T> {
    pub fn new<F, W>(initialize: F) -> Result<Self>
    where
        F: FnOnce() -> W + Send + 'static,
        W: FnMut() -> Result<T> + 'static,
    {
        let (requests, mut receiver) = mpsc::channel::<oneshot::Sender<Result<T>>>(1);
        // The worker itself need not be Send: native objects are created, used
        // and destroyed only on this thread. A stuck call cannot spawn more workers.
        std::thread::Builder::new()
            .name("wifi-monitor".into())
            .spawn(move || {
                let mut query = initialize();
                while let Some(reply) = receiver.blocking_recv() {
                    if !reply.is_closed() {
                        let _ = reply.send(query());
                    }
                }
            })
            .context(Message::FailedToStartWiFiWorkerThread)?;
        Ok(Self { requests })
    }

    pub async fn query(&self) -> Result<T> {
        let (sender, receiver) = oneshot::channel();
        self.requests
            .send(sender)
            .await
            .context(Message::WiFiWorkerChannelClosed)?;
        receiver.await.context(Message::WiFiWorkerThreadStopped)?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        rc::Rc,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
        time::{Duration, Instant},
    };

    #[tokio::test]
    async fn stalled_native_call_is_bounded_and_does_not_block_async_shutdown() -> Result<()> {
        let (release, wait) = std::sync::mpsc::channel();
        let calls = Arc::new(AtomicUsize::new(0));
        let count = calls.clone();
        let worker = Worker::new(move || {
            // Real native handles may also be !Send. Construct them on the worker.
            let state = Rc::new(());
            move || {
                let _ = &state;
                let call = count.fetch_add(1, Ordering::SeqCst);
                if call == 0 {
                    wait.recv_timeout(Duration::from_secs(3))?;
                }
                Ok(call)
            }
        })?;
        let started = Instant::now();
        for _ in 0..4 {
            assert!(
                tokio::time::timeout(Duration::from_millis(20), worker.query())
                    .await
                    .is_err()
            );
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(started.elapsed() < Duration::from_secs(1));
        // Cancelling queued requests does not leave the worker permanently wedged.
        release.send(())?;
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(1), worker.query()).await??,
            1
        );
        Ok(())
    }
}
