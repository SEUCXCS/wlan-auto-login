use super::TARGET_SSID;
use anyhow::{Context, Result};
use futures_util::StreamExt;
use std::collections::HashMap;
use zbus::{
    Connection, MatchRule, MessageStream, Proxy,
    message::Type,
    proxy::CacheProperties,
    zvariant::{OwnedObjectPath, OwnedValue},
};

const NM: &str = "org.freedesktop.NetworkManager";
const ROOT: &str = "/org/freedesktop/NetworkManager";

pub struct Monitor {
    connection: Connection,
    events: Events,
    boot: String,
}

impl Monitor {
    pub async fn new() -> Result<Self> {
        let connection = Connection::system().await.context("无法连接系统 D-Bus")?;
        let rule = MatchRule::builder()
            .msg_type(Type::Signal)
            .sender(NM)?
            .path_namespace(ROOT)?
            .build();
        let events = MessageStream::for_match_rule(rule, &connection, Some(64)).await?;
        let boot = std::fs::read_to_string("/proc/sys/kernel/random/boot_id").unwrap_or_default();
        Ok(Self {
            connection,
            events: Events::new(events),
            boot,
        })
    }

    async fn proxy<'a>(&'a self, path: &'a str, interface: &'a str) -> Result<Proxy<'a>> {
        Ok(zbus::proxy::Builder::new(&self.connection)
            .destination(NM)?
            .path(path)?
            .interface(interface)?
            .cache_properties(CacheProperties::No)
            .build()
            .await?)
    }

    pub async fn snapshot(&self) -> Result<Option<String>> {
        let manager = self.proxy(ROOT, NM).await?;
        let devices: Vec<OwnedObjectPath> = manager
            .call("GetDevices", &())
            .await
            .context("需要运行 NetworkManager")?;
        for path in devices {
            let device = self
                .proxy(path.as_str(), "org.freedesktop.NetworkManager.Device")
                .await?;
            let kind: u32 = device.get_property("DeviceType").await?;
            let state: u32 = device.get_property("State").await?;
            if kind != 2 || state != 100 {
                continue;
            }
            let wifi = self
                .proxy(
                    path.as_str(),
                    "org.freedesktop.NetworkManager.Device.Wireless",
                )
                .await?;
            let ap: OwnedObjectPath = wifi.get_property("ActiveAccessPoint").await?;
            if ap.as_str() == "/" {
                continue;
            }
            let ap = self
                .proxy(ap.as_str(), "org.freedesktop.NetworkManager.AccessPoint")
                .await?;
            let ssid: Vec<u8> = ap.get_property("Ssid").await?;
            if ssid == TARGET_SSID {
                let active: OwnedObjectPath = device.get_property("ActiveConnection").await?;
                return Ok(Some(format!("{}:{}", self.boot.trim(), active)));
            }
        }
        Ok(None)
    }

    pub async fn changed(&mut self) -> Result<()> {
        self.events.changed().await
    }
}

struct Events {
    updates: tokio::sync::watch::Receiver<Result<(), String>>,
    task: tokio::task::JoinHandle<()>,
}

impl Events {
    fn new(mut stream: MessageStream) -> Self {
        let (sender, updates) = tokio::sync::watch::channel(Ok(()));
        let task = tokio::spawn(async move {
            // zbus requires continuous polling: a full signal queue otherwise
            // blocks method replies on the same connection. watch coalesces
            // bursts without ever waiting for the daemon to finish its check.
            while let Some(message) = stream.next().await {
                match message {
                    Ok(message) => {
                        if relevant(&message) {
                            let _ = sender.send_replace(Ok(()));
                        }
                    }
                    Err(error) => {
                        let _ = sender.send_replace(Err(format!("{error:#}")));
                        return;
                    }
                }
            }
            let _ = sender.send_replace(Err("NetworkManager 事件连接已关闭".into()));
        });
        Self { updates, task }
    }

    async fn changed(&mut self) -> Result<()> {
        self.updates
            .changed()
            .await
            .context("NetworkManager 事件监听已退出")?;
        self.updates
            .borrow_and_update()
            .clone()
            .map_err(anyhow::Error::msg)
    }
}

impl Drop for Events {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn relevant(message: &zbus::Message) -> bool {
    let header = message.header();
    let member = header.member().map(|v| v.as_str()).unwrap_or_default();
    if member == "StateChanged" || member == "DeviceAdded" || member == "DeviceRemoved" {
        return true;
    }
    if member == "PropertiesChanged"
        && let Ok((_, changed, invalidated)) =
            message
                .body()
                .deserialize::<(String, HashMap<String, OwnedValue>, Vec<String>)>()
    {
        return [
            "State",
            "ActiveConnection",
            "ActiveAccessPoint",
            "Ip4Config",
            "PrimaryConnection",
        ]
        .iter()
        .any(|key| changed.contains_key(*key) || invalidated.iter().any(|v| v == key));
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    struct FakeManager;

    #[zbus::interface(name = "org.freedesktop.NetworkManager")]
    impl FakeManager {
        fn get_devices(&self) -> Vec<OwnedObjectPath> {
            Vec::new()
        }
    }

    #[tokio::test]
    async fn signal_burst_does_not_block_queries_while_daemon_is_busy() -> Result<()> {
        tokio::time::timeout(Duration::from_secs(5), async {
            // A private socket pair, not the machine's system or session bus.
            let (left, right) = tokio::net::UnixStream::pair()?;
            let server = zbus::connection::Builder::unix_stream(left)
                .server(zbus::Guid::generate())?
                .p2p()
                .serve_at(ROOT, FakeManager)?
                .build();
            let client = zbus::connection::Builder::unix_stream(right).p2p().build();
            let (server, connection) = tokio::try_join!(server, client)?;
            let rule = MatchRule::builder()
                .msg_type(Type::Signal)
                .path_namespace(ROOT)?
                .build();
            let stream = MessageStream::for_match_rule(rule, &connection, Some(64)).await?;
            let mut monitor = Monitor {
                connection,
                events: Events::new(stream),
                boot: "test".into(),
            };
            // Deliberately do not call changed() until AFTER the query reply.
            // The old inline consumer deadlocked once its 64-message queue filled.
            for _ in 0..256 {
                server
                    .emit_signal(None::<&str>, ROOT, NM, "StateChanged", &(100u32,))
                    .await?;
            }
            assert_eq!(monitor.snapshot().await?, None);
            monitor.changed().await?;
            server.close().await?;
            // The reader can have delivered the reply before the event task
            // consumes the last queued signals. Observe their changes until
            // disconnect, without assuming a particular Tokio task ordering.
            while monitor.changed().await.is_ok() {}
            Ok::<_, anyhow::Error>(())
        })
        .await?
    }
}
