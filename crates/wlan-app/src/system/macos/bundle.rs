//! Resolve and authenticate the signed application used by the Wi-Fi host.
use crate::infra::{
    error::{Context, Message, Result, ensure},
    identity::{APP_BUNDLE, APP_NAME},
    scope::Scope,
};
use core_foundation::{base::TCFType, data::CFData, url::CFURL};
use security_framework::os::macos::code_signing::{
    Flags, GuestAttributes, SecCode, SecRequirement, SecStaticCode,
};
use std::{
    os::fd::AsRawFd,
    path::{Path, PathBuf},
};

pub fn enclosing(executable: &Path) -> Option<PathBuf> {
    let macos = executable.parent()?;
    let contents = macos.parent()?;
    let bundle = contents.parent()?;
    (macos.file_name()? == "MacOS"
        && contents.file_name()? == "Contents"
        && bundle.extension()? == "app")
        .then(|| bundle.to_owned())
}

pub fn current() -> Result<PathBuf> {
    enclosing(&std::env::current_exe()?.canonicalize()?).context(Message::MacosBundleRequired)
}

pub fn resolve(scope: Scope) -> Result<PathBuf> {
    let installed = crate::infra::paths::macos_bundle(scope)?;
    if executable(&installed).is_file() {
        return Ok(installed);
    }
    if let Ok(current) = current() {
        return Ok(current);
    }
    let executable = std::env::current_exe()?;
    let sibling = executable
        .parent()
        .context(Message::MacosBundleRequired)?
        .join("bundle")
        .join(APP_BUNDLE);
    ensure!(sibling.is_dir(), Message::MacosBundleRequired);
    Ok(sibling)
}

pub fn executable(bundle: &Path) -> PathBuf {
    bundle.join("Contents/MacOS").join(APP_NAME)
}

#[link(name = "Security", kind = "framework")]
unsafe extern "C" {
    fn SecCodeCopyDesignatedRequirement(
        code: *const std::ffi::c_void,
        flags: u32,
        requirement: *mut security_framework_sys::code_signing::SecRequirementRef,
    ) -> i32;
}

/// Authenticate the live socket peer using its kernel audit token, never a peer-supplied PID.
pub fn verify_peer(stream: &tokio::net::UnixStream, expected_uid: u32, app: &Path) -> Result<()> {
    ensure!(
        stream.peer_cred()?.uid() == expected_uid,
        Message::MacosHostIdentity
    );
    let mut token = [0u8; 32];
    let mut length = token.len() as libc::socklen_t;
    let status = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            0,
            libc::LOCAL_PEERTOKEN,
            token.as_mut_ptr().cast(),
            &mut length,
        )
    };
    ensure!(
        status == 0 && length as usize == token.len(),
        Message::MacosHostIdentity
    );
    let data = CFData::from_buffer(&token);
    let mut attributes = GuestAttributes::new();
    attributes.set_audit_token(data.as_concrete_TypeRef());
    let peer = SecCode::copy_guest_with_attribues(None, &attributes, Flags::NONE)?;
    let url = CFURL::from_path(app, true).context(Message::MacosBundleRequired)?;
    let expected = SecStaticCode::from_path(&url, Flags::NONE)?;
    let mut requirement = std::ptr::null_mut();
    let status =
        unsafe { SecCodeCopyDesignatedRequirement(expected.as_CFTypeRef(), 0, &mut requirement) };
    ensure!(
        status == 0 && !requirement.is_null(),
        Message::MacosHostIdentity
    );
    let requirement = unsafe { SecRequirement::wrap_under_create_rule(requirement) };
    peer.check_validity(Flags::NONE, &requirement)
        .context(Message::MacosHostIdentity)
}
