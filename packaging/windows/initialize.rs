//! Immediate MSI action: allocate a journal ID and seal the masked WixUI form.
use std::{ffi::c_void, fs::OpenOptions, io::Write, ptr};

#[repr(C)]
struct Blob {
    length: u32,
    data: *mut u8,
}
#[repr(C)]
struct SecurityAttributes {
    length: u32,
    descriptor: *mut c_void,
    inherit: i32,
}
#[link(name = "msi")]
unsafe extern "system" {
    fn MsiSetPropertyW(session: u32, name: *const u16, value: *const u16) -> u32;
    fn MsiGetPropertyW(session: u32, name: *const u16, value: *mut u16, length: *mut u32) -> u32;
}
#[link(name = "crypt32")]
unsafe extern "system" {
    fn CryptProtectData(
        input: *const Blob,
        description: *const u16,
        entropy: *const Blob,
        reserved: *mut c_void,
        prompt: *const c_void,
        flags: u32,
        output: *mut Blob,
    ) -> i32;
}
#[link(name = "advapi32")]
unsafe extern "system" {
    fn ConvertStringSecurityDescriptorToSecurityDescriptorW(
        text: *const u16,
        revision: u32,
        descriptor: *mut *mut c_void,
        length: *mut u32,
    ) -> i32;
}
#[link(name = "kernel32")]
unsafe extern "system" {
    fn LocalFree(pointer: *mut c_void) -> *mut c_void;
    fn CreateDirectoryW(path: *const u16, security: *const SecurityAttributes) -> i32;
}
fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}
fn property(session: u32, name: &str) -> Result<String, u32> {
    let name = wide(name);
    let mut length = 0;
    let result = unsafe { MsiGetPropertyW(session, name.as_ptr(), ptr::null_mut(), &mut length) };
    if result != 0 && result != 234 {
        return Err(result);
    }
    let mut value = vec![0; length as usize + 1];
    length += 1;
    let result =
        unsafe { MsiGetPropertyW(session, name.as_ptr(), value.as_mut_ptr(), &mut length) };
    let text = String::from_utf16(&value[..length as usize]).map_err(|_| 1603u32);
    value.fill(0);
    if result != 0 {
        return Err(result);
    }
    text
}
fn set_property(session: u32, name: &str, value: &str) -> Result<(), u32> {
    let result = unsafe { MsiSetPropertyW(session, wide(name).as_ptr(), wide(value).as_ptr()) };
    if result == 0 { Ok(()) } else { Err(result) }
}
struct Secrets(Vec<String>);
impl Drop for Secrets {
    fn drop(&mut self) {
        for field in &mut self.0 {
            unsafe {
                field.as_bytes_mut().fill(0);
            }
        }
    }
}
fn create_diagnostics_directory(path: &std::path::Path, inherit: bool) -> Result<(), u32> {
    let path = wide(&path.to_string_lossy());
    // The bootstrap root already grants its initiating user and SYSTEM access.
    // Preserve that access when the immediate action runs with an elevated token.
    if inherit {
        return if unsafe { CreateDirectoryW(path.as_ptr(), ptr::null()) } != 0 {
            Ok(())
        } else {
            Err(1603)
        };
    }
    let mut descriptor = ptr::null_mut();
    unsafe {
        if ConvertStringSecurityDescriptorToSecurityDescriptorW(
            wide("D:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;FA;;;OW)").as_ptr(),
            1,
            &mut descriptor,
            ptr::null_mut(),
        ) == 0
        {
            return Err(1603);
        }
        let security = SecurityAttributes {
            length: std::mem::size_of::<SecurityAttributes>() as u32,
            descriptor,
            inherit: 0,
        };
        let created = CreateDirectoryW(path.as_ptr(), &security);
        LocalFree(descriptor);
        if created == 0 {
            return Err(1603);
        }
    }
    Ok(())
}
fn initialize(session: u32) -> Result<(), u32> {
    let time = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| 1603u32)?;
    let transaction = format!("{:032x}", time.as_nanos() ^ u128::from(std::process::id()));
    set_property(session, "WLANLOGIN_TRANSACTION", &transaction)?;
    let fields = Secrets(vec![
        property(session, "WLANLOGIN_USERNAME")?,
        property(session, "WLANLOGIN_PASSWORD")?,
        property(session, "WLANLOGIN_LINE")?,
    ]);
    let log_root = property(session, "WLANLOGIN_LOG_ROOT")?;
    let inherit_security = !log_root.is_empty();
    let root = if log_root.is_empty() {
        std::env::temp_dir()
    } else {
        std::path::PathBuf::from(log_root)
    };
    let directory = root.join(format!("wlanlogin-form-{transaction}"));
    create_diagnostics_directory(&directory, inherit_security)?;
    set_property(
        session,
        "WLANLOGIN_DIAGNOSTICS",
        &directory.to_string_lossy(),
    )?;
    std::fs::write(
        directory.join("action.log"),
        b"Prepared Windows Installer action context\n",
    )
    .map_err(|_| 1603u32)?;
    if fields.0[0].is_empty() && fields.0[1].is_empty() {
        return Ok(());
    }
    let file = directory.join("credentials.bin");
    let outcome = (|| {
        let mut bytes = Vec::new();
        for field in &fields.0 {
            bytes.extend_from_slice(&(field.len() as u32).to_le_bytes());
            bytes.extend_from_slice(field.as_bytes());
        }
        let input = Blob {
            length: bytes.len() as u32,
            data: bytes.as_mut_ptr(),
        };
        let mut output = Blob {
            length: 0,
            data: ptr::null_mut(),
        };
        let flags = 1 | if property(session, "WLANLOGIN_SCOPE")? == "machine" {
            4
        } else {
            0
        };
        let encrypted = unsafe {
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
        bytes.fill(0);
        if encrypted == 0 {
            return Err(1603);
        }
        let written = (|| {
            let mut target = OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&file)
                .map_err(|_| 1603u32)?;
            target
                .write_all(unsafe {
                    std::slice::from_raw_parts(output.data, output.length as usize)
                })
                .map_err(|_| 1603u32)?;
            target.sync_all().map_err(|_| 1603u32)
        })();
        unsafe {
            LocalFree(output.data.cast());
        }
        written?;
        set_property(session, "WLANLOGIN_SESSION", &file.to_string_lossy())?;
        set_property(session, "WLANLOGIN_PASSWORD", "")
    })();
    if outcome.is_err() {
        let _ = std::fs::remove_file(&file);
        let _ = std::fs::remove_dir(&directory);
    }
    outcome
}

/// # Safety
/// Windows Installer supplies a valid, live installation-session handle.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn InitializeTransaction(session: u32) -> u32 {
    match std::panic::catch_unwind(|| initialize(session)) {
        Ok(Ok(())) => 0,
        Ok(Err(code)) => code,
        Err(_) => 1603,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn diagnostics_inherit_the_bootstrap_users_private_access() -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        crate::win::private_directory(root.path(), false)?;
        let directory = root.path().join("wlanlogin-form-test");
        super::create_diagnostics_directory(&directory, true)
            .map_err(|code| anyhow::anyhow!("Directory creation failed ({code})"))?;
        let security = crate::win::security(&directory)?;
        assert!(security.contains(&crate::win::sid()?));
        assert!(security.contains(";;;SY)"));
        assert!(!security.contains(";;;BA)"));
        assert!(!security.contains(";;;WD)"));
        Ok(())
    }
}
