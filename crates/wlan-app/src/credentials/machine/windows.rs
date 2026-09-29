use crate::infra::error::Message;
use crate::infra::error::Result;
use windows_sys::Win32::{Foundation::LocalFree, Security::Cryptography::*};
use zeroize::{Zeroize, Zeroizing};

pub fn encrypt(bytes: &[u8]) -> Result<Zeroizing<Vec<u8>>> {
    transform(bytes, true)
}
pub fn decrypt(bytes: &[u8]) -> Result<Zeroizing<Vec<u8>>> {
    transform(bytes, false)
}

fn transform(bytes: &[u8], encrypt: bool) -> Result<Zeroizing<Vec<u8>>> {
    crate::infra::error::ensure!(
        bytes.len() <= 64 * 1024,
        Message::CredentialPayloadExceedsTheSizeLimit
    );
    let input = CRYPT_INTEGER_BLOB {
        cbData: bytes.len() as u32,
        pbData: bytes.as_ptr().cast_mut(),
    };
    let mut output: CRYPT_INTEGER_BLOB = unsafe { std::mem::zeroed() };
    // SAFETY: DPAPI reads input only, allocates output with LocalAlloc, and cannot prompt.
    unsafe {
        let ok = if encrypt {
            CryptProtectData(
                &input,
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_LOCAL_MACHINE | CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        } else {
            CryptUnprotectData(
                &input,
                std::ptr::null_mut(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        };
        if ok == 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        if output.cbData == 0 || output.pbData.is_null() {
            LocalFree(output.pbData.cast());
            crate::infra::error::bail!(Message::DPAPIReturnedAnEmptyCredentialPayload);
        }
        let buffer = std::slice::from_raw_parts_mut(output.pbData, output.cbData as usize);
        let bytes = Zeroizing::new(buffer.to_vec());
        buffer.zeroize();
        LocalFree(output.pbData.cast());
        Ok(bytes)
    }
}
