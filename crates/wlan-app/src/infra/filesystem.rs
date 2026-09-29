use crate::infra::error::Message;
use crate::infra::error::{Context, Result};
use std::{fs, path::Path};

pub fn private_dir(path: &Path) -> Result<()> {
    fs::create_dir_all(path)?;
    let metadata = fs::symlink_metadata(path)?;
    crate::infra::error::ensure!(
        metadata.is_dir() && !metadata.file_type().is_symlink(),
        Message::ConfigurationDirectoryMustNotBeASymbolic {
            value0: (path.display()).to_string()
        }
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        crate::infra::error::ensure!(
            metadata.uid() == unsafe { libc::geteuid() },
            Message::ConfigurationDirectoryIsNotOwnedByThe {
                value0: (path.display()).to_string()
            }
        );
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    #[cfg(windows)]
    restrict_windows(path, false)?;
    Ok(())
}

#[cfg(windows)]
pub fn restrict_windows(path: &Path, public: bool) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::{
        Foundation::LocalFree,
        Security::{
            Authorization::ConvertStringSecurityDescriptorToSecurityDescriptorW,
            DACL_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION, SetFileSecurityW,
        },
    };
    let identity = if crate::infra::scope::elevated()? {
        "BA".into()
    } else {
        crate::infra::scope::current_sid()?
    };
    let read = if public { "(A;OICI;GRGX;;;BU)" } else { "" };
    let sddl: Vec<u16> = format!("D:P(A;OICI;FA;;;SY)(A;OICI;FA;;;{identity}){read}")
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let path: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    // SAFETY: the descriptor is allocated by Windows and freed after SetFileSecurity copies it.
    unsafe {
        let mut descriptor = std::ptr::null_mut();
        if ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            1,
            &mut descriptor,
            std::ptr::null_mut(),
        ) == 0
        {
            return Err(std::io::Error::last_os_error().into());
        }
        let result = SetFileSecurityW(
            path.as_ptr(),
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            descriptor,
        );
        let error = std::io::Error::last_os_error();
        LocalFree(descriptor);
        if result == 0 {
            return Err(error.into());
        }
    }
    Ok(())
}

pub fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    let parent = path.parent().context(Message::FileHasNoParentDirectory)?;
    private_dir(parent)?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    temp.write_all(bytes)?;
    temp.as_file().sync_all()?;
    temp.persist(path).map_err(|e| e.error)?;
    Ok(())
}
