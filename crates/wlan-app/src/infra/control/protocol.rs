//! Strict current IPC frames. Only diagnostic enums cross the status boundary.
use crate::infra::error::{Message, Result, ensure};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
pub const VERSION: u32 = 3;
const MAX_FRAME: usize = 4096;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CredentialState {
    #[default]
    NotChecked,
    Available,
    Missing,
    WaitingForSession,
    WaitingForProvider,
    WaitingForUnlock,
    WaitingForPermission,
    WaitingForInteraction,
    Failed,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeStatus {
    pub outcome: Option<String>,
    pub credentials: CredentialState,
    #[serde(deserialize_with = "required_diagnostic")]
    pub diagnostic: Option<crate::infra::error::credentials::Diagnostic>,
}
// The field is nullable but required. Serde otherwise defaults missing Option fields.
fn required_diagnostic<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Option<crate::infra::error::credentials::Diagnostic>, D::Error> {
    Option::deserialize(deserializer)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case", deny_unknown_fields)]
pub enum Command {
    Wake,
    Stop,
    ManagedRunning,
    ManagedStop,
    Status,
    SessionReady { session: String },
    SessionEnd { session: String },
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub version: u32,
    pub scope: String,
    pub token: String,
    pub action: Command,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Response {
    pub version: u32,
    pub accepted: bool,
    pub status: Option<RuntimeStatus>,
}
pub async fn read<T: serde::de::DeserializeOwned>(
    stream: &mut (impl AsyncRead + Unpin),
) -> Result<T> {
    let length = stream.read_u32().await? as usize;
    ensure!(length > 0 && length <= MAX_FRAME, Message::ControlProtocol);
    let mut bytes = vec![0; length];
    stream.read_exact(&mut bytes).await?;
    serde_json::from_slice(&bytes).map_err(Into::into)
}
pub async fn write<T: Serialize>(stream: &mut (impl AsyncWrite + Unpin), value: &T) -> Result<()> {
    let bytes = serde_json::to_vec(value)?;
    ensure!(bytes.len() <= MAX_FRAME, Message::ControlProtocol);
    stream.write_u32(bytes.len() as u32).await?;
    stream.write_all(&bytes).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn frames_reject_oversize_and_old_unframed_bytes() -> Result<()> {
        for size in [0, u32::MAX] {
            let (mut writer, mut reader) = tokio::io::duplex(128);
            writer.write_u32(size).await?;
            assert!(read::<Request>(&mut reader).await.is_err());
        }
        let (mut writer, mut reader) = tokio::io::duplex(128);
        writer.write_all(&[b'a'; 65]).await?;
        assert!(read::<Request>(&mut reader).await.is_err());
        Ok(())
    }
    #[test]
    fn current_status_carries_only_safe_credential_diagnostics_and_requires_the_new_schema()
    -> Result<()> {
        use crate::infra::error::credentials::{Diagnostic, Kind, Stage};
        let status = RuntimeStatus {
            outcome: Some("credential_read_failed".into()),
            credentials: CredentialState::Failed,
            diagnostic: Some(Diagnostic::new(Kind::InvalidFormat, Stage::Decode)),
        };
        let text = serde_json::to_string(&status)?;
        assert!(text.contains("invalid_format") && text.contains("decode"));
        for field in ["password", "username", "source", "token"] {
            assert!(!text.contains(field));
        }
        assert!(
            serde_json::from_str::<RuntimeStatus>(
                r#"{"outcome":null,"credentials":"not_checked"}"#
            )
            .is_err()
        );
        Ok(())
    }

    #[test]
    fn status_has_no_secret_fields_and_unknown_protocol_fields_are_rejected() -> Result<()> {
        let text = serde_json::to_string(&RuntimeStatus {
            outcome: Some("waiting_for_keyring_unlock".into()),
            credentials: CredentialState::WaitingForUnlock,
            diagnostic: None,
        })?;
        for secret in ["password", "username", "token"] {
            assert!(!text.contains(secret));
        }
        assert!(serde_json::from_str::<Request>(r#"{"version":3,"scope":"user","token":"test","action":{"command":"wake"},"password":"secret"}"#).is_err());
        Ok(())
    }
}
