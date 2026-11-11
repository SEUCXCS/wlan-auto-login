//! Small Windows API wrappers shared by the setup UI and MSI action executable.
use anyhow::{Context, Result, ensure};
use std::{
    ffi::OsStr,
    fs,
    io::Write,
    os::windows::{ffi::OsStrExt, fs::MetadataExt},
    path::{Path, PathBuf},
    ptr,
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE, LocalFree},
    Security::{
        Authorization::{
            ConvertSecurityDescriptorToStringSecurityDescriptorW, ConvertSidToStringSidW,
            ConvertStringSecurityDescriptorToSecurityDescriptorW,
        },
        DACL_SECURITY_INFORMATION, GetFileSecurityW, GetSecurityDescriptorControl,
        GetTokenInformation, PROTECTED_DACL_SECURITY_INFORMATION, SE_DACL_PROTECTED,
        SetFileSecurityW, TOKEN_ELEVATION, TOKEN_QUERY, TOKEN_USER, TokenElevation, TokenUser,
        UNPROTECTED_DACL_SECURITY_INFORMATION,
    },
    System::{
        Com::CoTaskMemFree,
        Threading::{GetCurrentProcess, OpenProcessToken},
    },
    UI::{
        Shell::{FOLDERID_ProgramData, FOLDERID_ProgramFiles, SHGetKnownFolderPath},
        WindowsAndMessaging::{MB_ICONERROR, MB_ICONINFORMATION, MB_OK, MessageBoxW},
    },
};

pub fn wide(value: impl AsRef<OsStr>) -> Vec<u16> {
    value.as_ref().encode_wide().chain(Some(0)).collect()
}
pub fn check(result: i32) -> Result<()> {
    if result == 0 {
        Err(std::io::Error::last_os_error().into())
    } else {
        Ok(())
    }
}
pub struct Handle(pub HANDLE);
impl Drop for Handle {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}

pub fn message(text: &str, error: bool) {
    unsafe {
        MessageBoxW(
            ptr::null_mut(),
            wide(text).as_ptr(),
            wide("CXCS WLAN Login").as_ptr(),
            MB_OK
                | if error {
                    MB_ICONERROR
                } else {
                    MB_ICONINFORMATION
                },
        );
    }
}

pub fn token() -> Result<Handle> {
    unsafe {
        let mut token = ptr::null_mut();
        check(OpenProcessToken(
            GetCurrentProcess(),
            TOKEN_QUERY,
            &mut token,
        ))?;
        Ok(Handle(token))
    }
}
pub fn elevated() -> Result<bool> {
    unsafe {
        let token = token()?;
        let mut value: TOKEN_ELEVATION = std::mem::zeroed();
        let mut size = 0;
        check(GetTokenInformation(
            token.0,
            TokenElevation,
            (&mut value as *mut TOKEN_ELEVATION).cast(),
            std::mem::size_of_val(&value) as u32,
            &mut size,
        ))?;
        Ok(value.TokenIsElevated != 0)
    }
}
pub fn sid() -> Result<String> {
    unsafe {
        let token = token()?;
        let mut size = 0;
        GetTokenInformation(token.0, TokenUser, ptr::null_mut(), 0, &mut size);
        let mut data = vec![0usize; (size as usize).div_ceil(std::mem::size_of::<usize>())];
        check(GetTokenInformation(
            token.0,
            TokenUser,
            data.as_mut_ptr().cast(),
            size,
            &mut size,
        ))?;
        let mut string = ptr::null_mut();
        check(ConvertSidToStringSidW(
            (*data.as_ptr().cast::<TOKEN_USER>()).User.Sid,
            &mut string,
        ))?;
        let value = string_from_ptr(string);
        LocalFree(string.cast());
        Ok(value)
    }
}
/// Read a Windows string returned by a native API.
///
/// # Safety
/// A non-null pointer must address a readable, null-terminated UTF-16 string.
pub unsafe fn string_from_ptr(value: *const u16) -> String {
    if value.is_null() {
        return String::new();
    }
    unsafe {
        let mut size = 0;
        while *value.add(size) != 0 {
            size += 1;
        }
        String::from_utf16_lossy(std::slice::from_raw_parts(value, size))
    }
}

pub fn program_files() -> Result<PathBuf> {
    known_folder(&FOLDERID_ProgramFiles)
}
pub fn program_data() -> Result<PathBuf> {
    known_folder(&FOLDERID_ProgramData)
}
fn known_folder(folder: &windows_sys::core::GUID) -> Result<PathBuf> {
    unsafe {
        let mut path = ptr::null_mut();
        let result = SHGetKnownFolderPath(folder, 0, ptr::null_mut(), &mut path);
        ensure!(
            result >= 0,
            "Could not resolve a global Windows folder ({result:#x})"
        );
        let value = PathBuf::from(string_from_ptr(path));
        CoTaskMemFree(path.cast());
        Ok(value)
    }
}

pub fn plain_path(path: &Path) -> Result<()> {
    for ancestor in path.ancestors() {
        match fs::symlink_metadata(ancestor) {
            Ok(metadata) => ensure!(
                metadata.file_attributes() & 0x400 == 0,
                "Installation paths must not contain reparse points"
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

pub fn private_directory(path: &Path, machine: bool) -> Result<()> {
    plain_path(path)?;
    fs::create_dir_all(path)?;
    plain_path(path)?;
    let owner = if machine { "BA".into() } else { sid()? };
    set_security(path, &format!("D:P(A;OICI;FA;;;SY)(A;OICI;FA;;;{owner})"))
}

pub fn security(path: &Path) -> Result<String> {
    unsafe {
        let mut length = 0;
        GetFileSecurityW(
            wide(path).as_ptr(),
            DACL_SECURITY_INFORMATION,
            ptr::null_mut(),
            0,
            &mut length,
        );
        ensure!(length > 0, "Could not inspect file permissions");
        let mut buffer = vec![0usize; (length as usize).div_ceil(std::mem::size_of::<usize>())];
        check(GetFileSecurityW(
            wide(path).as_ptr(),
            DACL_SECURITY_INFORMATION,
            buffer.as_mut_ptr().cast(),
            length,
            &mut length,
        ))?;
        let mut string = ptr::null_mut();
        check(ConvertSecurityDescriptorToStringSecurityDescriptorW(
            buffer.as_mut_ptr().cast(),
            1,
            DACL_SECURITY_INFORMATION,
            &mut string,
            ptr::null_mut(),
        ))?;
        let result = string_from_ptr(string);
        LocalFree(string.cast());
        Ok(result)
    }
}
pub fn set_security(path: &Path, sddl: &str) -> Result<()> {
    unsafe {
        let mut descriptor = ptr::null_mut();
        check(ConvertStringSecurityDescriptorToSecurityDescriptorW(
            wide(sddl).as_ptr(),
            1,
            &mut descriptor,
            ptr::null_mut(),
        ))?;
        let mut control = 0;
        let mut revision = 0;
        let inspected = GetSecurityDescriptorControl(descriptor, &mut control, &mut revision);
        if inspected == 0 {
            LocalFree(descriptor);
            return Err(std::io::Error::last_os_error().into());
        }
        let protection = if control & SE_DACL_PROTECTED != 0 {
            PROTECTED_DACL_SECURITY_INFORMATION
        } else {
            UNPROTECTED_DACL_SECURITY_INFORMATION
        };
        let result = SetFileSecurityW(
            wide(path).as_ptr(),
            DACL_SECURITY_INFORMATION | protection,
            descriptor,
        );
        let error = std::io::Error::last_os_error();
        LocalFree(descriptor);
        if result == 0 {
            return Err(error.into());
        }
        Ok(())
    }
}

pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    plain_path(path)?;
    let parent = path.parent().context("File has no parent")?;
    fs::create_dir_all(parent)?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(bytes)?;
    temporary.as_file().sync_all()?;
    temporary.persist(path).map_err(|error| error.error)?;
    Ok(())
}

pub fn protect(bytes: &[u8], machine: bool, decrypt: bool) -> Result<zeroize::Zeroizing<Vec<u8>>> {
    use windows_sys::Win32::Security::Cryptography::{
        CRYPT_INTEGER_BLOB, CRYPTPROTECT_LOCAL_MACHINE, CRYPTPROTECT_UI_FORBIDDEN,
        CryptProtectData, CryptUnprotectData,
    };
    unsafe {
        let input = CRYPT_INTEGER_BLOB {
            cbData: bytes.len().try_into()?,
            pbData: bytes.as_ptr().cast_mut(),
        };
        let mut output: CRYPT_INTEGER_BLOB = std::mem::zeroed();
        let flags = CRYPTPROTECT_UI_FORBIDDEN
            | if machine && !decrypt {
                CRYPTPROTECT_LOCAL_MACHINE
            } else {
                0
            };
        let result = if decrypt {
            CryptUnprotectData(
                &input,
                ptr::null_mut(),
                ptr::null(),
                ptr::null_mut(),
                ptr::null(),
                flags,
                &mut output,
            )
        } else {
            CryptProtectData(
                &input,
                ptr::null(),
                ptr::null(),
                ptr::null_mut(),
                ptr::null(),
                flags,
                &mut output,
            )
        };
        check(result).context("Could not protect or recover the installation snapshot")?;
        let value = zeroize::Zeroizing::new(
            std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec(),
        );
        std::ptr::write_bytes(output.pbData, 0, output.cbData as usize);
        LocalFree(output.pbData.cast());
        Ok(value)
    }
}
