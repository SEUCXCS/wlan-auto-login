//! Drain and coalesce provider/lock/session signals independently of credential reads.
use crate::infra::error::{Context, Message, Result};
use futures_util::StreamExt;
use zbus::{Connection, MatchRule, MessageStream, message::Type};

pub struct Events {
    updates: tokio::sync::watch::Receiver<std::result::Result<(), String>>,
    tasks: Vec<tokio::task::JoinHandle<()>>,
}
impl Events {
    pub async fn new() -> Result<Self> {
        let session = Connection::session().await?;
        let system = Connection::system().await?;
        let (sender, updates) = tokio::sync::watch::channel(Ok(()));
        let mut events = Self {
            updates,
            tasks: Vec::new(),
        };
        for (bus, rule) in [
            (
                session.clone(),
                MatchRule::builder()
                    .msg_type(Type::Signal)
                    .sender("org.freedesktop.DBus")?
                    .interface("org.freedesktop.DBus")?
                    .member("NameOwnerChanged")?
                    .arg(0, "org.freedesktop.secrets")?
                    .build(),
            ),
            (
                session.clone(),
                MatchRule::builder()
                    .msg_type(Type::Signal)
                    .sender("org.freedesktop.secrets")?
                    .interface("org.freedesktop.DBus.Properties")?
                    .member("PropertiesChanged")?
                    .build(),
            ),
            (
                session,
                MatchRule::builder()
                    .msg_type(Type::Signal)
                    .sender("org.freedesktop.systemd1")?
                    .path("/org/freedesktop/systemd1/unit/graphical_2dsession_2etarget")?
                    .interface("org.freedesktop.DBus.Properties")?
                    .member("PropertiesChanged")?
                    .build(),
            ),
            (
                system,
                MatchRule::builder()
                    .msg_type(Type::Signal)
                    .sender("org.freedesktop.login1")?
                    .build(),
            ),
        ] {
            let mut stream = MessageStream::for_match_rule(rule, &bus, Some(64)).await?;
            let sender = sender.clone();
            events.tasks.push(tokio::spawn(async move {
                // Retain the bus and continuously drain the bounded zbus queue.
                let _bus = bus;
                while let Some(message) = stream.next().await {
                    match message {
                        Ok(_) => {
                            let _ = sender.send_replace(Ok(()));
                        }
                        Err(error) => {
                            let _ = sender.send_replace(Err(error.to_string()));
                            return;
                        }
                    }
                }
                let _ = sender.send_replace(Err("Credential event bus disconnected".into()));
            }));
        }
        Ok(events)
    }
    pub async fn changed(&mut self) -> Result<()> {
        self.updates
            .changed()
            .await
            .context(Message::CredentialEventListenerStopped)?;
        self.updates
            .borrow_and_update()
            .clone()
            .map_err(crate::infra::error::Error::message)
    }
}
impl Drop for Events {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}
