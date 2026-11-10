//! Native definition decoding and executable inspection.
use crate::infra::error::Result;
use crate::system::define::services::Plan;
use std::path::Path;
pub(crate) fn read_definition(path: &Path) -> std::io::Result<String> {
    std::fs::read_to_string(path)
}
pub(crate) fn inspect_executable(_plan: &Plan, executable: bool) -> Result<bool> {
    Ok(executable)
}
use crate::infra::error::Message;
use std::path::PathBuf;

pub(crate) fn original_user_home() -> Result<Option<PathBuf>> {
    let Some(uid) = std::env::var("SUDO_UID")
        .ok()
        .and_then(|value| value.parse::<u32>().ok())
        .filter(|uid| *uid != 0)
    else {
        return Ok(None);
    };
    // SUDO_UID is only a lookup hint for conflict detection, never authority for an operation.
    let mut buffer = vec![0u8; 64 * 1024];
    let mut record: libc::passwd = unsafe { std::mem::zeroed() };
    let mut result = std::ptr::null_mut();
    // SAFETY: getpwuid_r writes pointers into our live buffer; copy the home path before returning.
    unsafe {
        let code = libc::getpwuid_r(
            uid,
            &mut record,
            buffer.as_mut_ptr().cast(),
            buffer.len(),
            &mut result,
        );
        crate::infra::error::ensure!(
            code == 0 && !result.is_null(),
            Message::FailedToResolveTheOriginalSudoUser
        );
        use std::os::unix::ffi::OsStrExt;
        Ok(Some(
            std::ffi::OsStr::from_bytes(std::ffi::CStr::from_ptr(record.pw_dir).to_bytes()).into(),
        ))
    }
}
