//! Active console user and scope-specific paths for peer verification.
use super::*;
pub(super) struct Session {
    pub(super) uid: u32,
    pub(super) runtime: PathBuf,
}
pub(super) fn session(scope: Scope) -> Result<Session> {
    if scope == Scope::User {
        return Ok(Session {
            uid: unsafe { libc::geteuid() },
            runtime: Paths::new(Scope::User)?.runtime,
        });
    }
    let uid = fs::metadata("/dev/console")?.uid();
    ensure!(uid != 0, Message::MacosUserSessionUnavailable);
    let mut buffer = vec![0u8; 64 * 1024];
    let mut record: libc::passwd = unsafe { std::mem::zeroed() };
    let mut result = std::ptr::null_mut();
    let status = unsafe {
        libc::getpwuid_r(
            uid,
            &mut record,
            buffer.as_mut_ptr().cast(),
            buffer.len(),
            &mut result,
        )
    };
    ensure!(
        status == 0 && !result.is_null(),
        Message::MacosUserSessionUnavailable
    );
    let home = unsafe { std::ffi::CStr::from_ptr(record.pw_dir) };
    let home = Path::new(std::ffi::OsStr::from_bytes(home.to_bytes()));
    Ok(Session {
        uid,
        runtime: paths::app_dir(home.join("Library/Application Support")),
    })
}
