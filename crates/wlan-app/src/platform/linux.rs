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
    events: MessageStream,
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
            events,
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
        while let Some(message) = self.events.next().await {
            let message = message?;
            let header = message.header();
            let member = header.member().map(|v| v.as_str()).unwrap_or_default();
            if member == "StateChanged" || member == "DeviceAdded" || member == "DeviceRemoved" {
                return Ok(());
            }
            if member == "PropertiesChanged"
                && let Ok((_, changed, invalidated)) =
                    message
                        .body()
                        .deserialize::<(String, HashMap<String, OwnedValue>, Vec<String>)>()
            {
                let relevant = [
                    "State",
                    "ActiveConnection",
                    "ActiveAccessPoint",
                    "Ip4Config",
                    "PrimaryConnection",
                ];
                if relevant
                    .iter()
                    .any(|key| changed.contains_key(*key) || invalidated.iter().any(|v| v == key))
                {
                    return Ok(());
                }
            }
        }
        anyhow::bail!("NetworkManager 事件连接已关闭")
    }
}
