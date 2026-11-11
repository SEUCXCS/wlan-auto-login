//! Native definition decoding and executable inspection.
use crate::infra::error::Result;
use crate::system::define::services::{Manager, Plan};
use std::{io, path::Path};
pub(crate) fn read_definition(path: &Path) -> io::Result<String> {
    decode_definition(&std::fs::read(path)?)
}
fn decode_definition(bytes: &[u8]) -> io::Result<String> {
    let (body, little) = if let Some(body) = bytes.strip_prefix(&[0xff, 0xfe]) {
        (body, true)
    } else if let Some(body) = bytes.strip_prefix(&[0xfe, 0xff]) {
        (body, false)
    } else {
        return String::from_utf8(
            bytes
                .strip_prefix(&[0xef, 0xbb, 0xbf])
                .unwrap_or(bytes)
                .to_vec(),
        )
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e));
    };
    if body.len() % 2 != 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Incomplete UTF-16 service definition",
        ));
    }
    let words = body
        .as_chunks::<2>()
        .0
        .iter()
        .map(|word| {
            if little {
                u16::from_le_bytes([word[0], word[1]])
            } else {
                u16::from_be_bytes([word[0], word[1]])
            }
        })
        .collect::<Vec<_>>();
    String::from_utf16(&words).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}
pub(crate) fn encode_task_xml(content: &str) -> Vec<u8> {
    [0xff, 0xfe]
        .into_iter()
        .chain(content.encode_utf16().flat_map(u16::to_le_bytes))
        .collect()
}
pub(crate) fn definition_bytes(plan: &Plan) -> Vec<u8> {
    match plan.manager {
        Manager::TaskScheduler { .. } => encode_task_xml(&plan.content),
        _ => plan.content.as_bytes().to_vec(),
    }
}
pub(crate) fn inspect_executable(_plan: &Plan, executable: bool) -> Result<bool> {
    Ok(executable)
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::windows::services::definition::{windows_plan, windows_system_plan};
    #[test]
    fn task_definition_round_trips_unicode_and_legacy_files() -> Result<()> {
        let plan = windows_plan(
            "C:/用户 & Test/𐐀/wlanlogin-daemon.exe".into(),
            "task.xml".into(),
            "S-1-5-21-123",
        )?;
        let bytes = definition_bytes(&plan);
        assert!(bytes.starts_with(&[0xff, 0xfe]));
        let decoded = decode_definition(&bytes)?;
        assert_eq!(decoded, plan.content);
        assert!(decoded.contains("UTF-16"));
        assert!(decoded.contains("用户 &amp; Test/𐐀"));
        let legacy = plan.content.replace("UTF-16", "UTF-8");
        assert_eq!(decode_definition(legacy.as_bytes())?, legacy);
        let dir = tempfile::tempdir()?;
        let file = dir.path().join("task.xml");
        std::fs::write(&file, bytes)?;
        crate::system::shared::services::artifacts::assert_owned(&file)?;
        let system = windows_system_plan("daemon.exe".into(), "service.json".into())?;
        assert_eq!(definition_bytes(&system), system.content.as_bytes());
        Ok(())
    }
    #[test]
    fn malformed_encodings_and_unowned_definitions_are_rejected() -> Result<()> {
        for bytes in [
            &[0xff, 0xfe, 0x41][..],
            &[0xff, 0xfe, 0x00, 0xd8][..],
            &[0xff][..],
        ] {
            assert!(decode_definition(bytes).is_err());
        }
        let dir = tempfile::tempdir()?;
        let file = dir.path().join("other.xml");
        std::fs::write(&file, encode_task_xml("<Task>Unrelated application</Task>"))?;
        assert!(crate::system::shared::services::artifacts::assert_owned(&file).is_err());
        Ok(())
    }
}
