//! Bounded frames with version and request identity checks. No credentials.
use super::*;
pub(super) const VERSION: u32 = 1;
pub(super) const MAX_FRAME: usize = 16 * 1024;
#[derive(Clone, Copy, Serialize, Deserialize)]
pub(super) enum Operation {
    Status,
    Snapshot,
    Authorize,
    Stop,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Request {
    pub(super) version: u32,
    pub(super) id: u64,
    pub(super) operation: Operation,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Response {
    pub(super) version: u32,
    pub(super) id: u64,
    pub(super) location: Status,
    pub(super) session: Option<String>,
    pub(super) error: Option<String>,
}

pub(super) async fn read_frame<T: serde::de::DeserializeOwned>(
    stream: &mut (impl AsyncReadExt + Unpin),
) -> Result<T> {
    let length = stream.read_u32().await? as usize;
    ensure!(
        length > 0 && length <= MAX_FRAME,
        Message::MacosHostProtocol
    );
    let mut bytes = vec![0; length];
    stream.read_exact(&mut bytes).await?;
    Ok(serde_json::from_slice(&bytes)?)
}
pub(super) async fn write_frame<T: Serialize>(
    stream: &mut (impl AsyncWriteExt + Unpin),
    value: &T,
) -> Result<()> {
    let bytes = serde_json::to_vec(value)?;
    ensure!(bytes.len() <= MAX_FRAME, Message::MacosHostProtocol);
    stream.write_u32(bytes.len() as u32).await?;
    stream.write_all(&bytes).await?;
    Ok(())
}
