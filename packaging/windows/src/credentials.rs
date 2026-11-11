//! Preserve the application's one native Credential Manager record byte for byte.
use crate::{identity, win};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::ptr;
use windows_sys::Win32::{
    Foundation::{ERROR_NOT_FOUND, GetLastError},
    Security::Credentials::*,
};
use zeroize::ZeroizeOnDrop;

#[derive(Serialize, Deserialize, ZeroizeOnDrop)]
pub struct Credential {
    flags: u32,
    persist: u32,
    username: String,
    comment: String,
    alias: String,
    blob: Vec<u8>,
    attributes: Vec<(String, u32, Vec<u8>)>,
}
fn target() -> Vec<u16> {
    win::wide(format!(
        "{}.{}",
        identity::CREDENTIAL_ENTRY,
        identity::APP_ID
    ))
}

pub fn capture() -> Result<Option<Credential>> {
    unsafe {
        let mut pointer = ptr::null_mut();
        if CredReadW(target().as_ptr(), CRED_TYPE_GENERIC, 0, &mut pointer) == 0 {
            if GetLastError() == ERROR_NOT_FOUND {
                return Ok(None);
            }
            return Err(std::io::Error::last_os_error().into());
        }
        let raw = &*pointer;
        let mut attributes = Vec::new();
        for index in 0..raw.AttributeCount as usize {
            let attribute = &*raw.Attributes.add(index);
            attributes.push((
                win::string_from_ptr(attribute.Keyword),
                attribute.Flags,
                bytes(attribute.Value, attribute.ValueSize),
            ));
        }
        let result = Credential {
            flags: raw.Flags,
            persist: raw.Persist,
            username: win::string_from_ptr(raw.UserName),
            comment: win::string_from_ptr(raw.Comment),
            alias: win::string_from_ptr(raw.TargetAlias),
            blob: bytes(raw.CredentialBlob, raw.CredentialBlobSize),
            attributes,
        };
        CredFree(pointer.cast());
        Ok(Some(result))
    }
}
unsafe fn bytes(pointer: *const u8, length: u32) -> Vec<u8> {
    if length == 0 {
        Vec::new()
    } else {
        unsafe { std::slice::from_raw_parts(pointer, length as usize).to_vec() }
    }
}
pub fn restore(snapshot: Option<&Credential>) -> Result<()> {
    unsafe {
        let Some(snapshot) = snapshot else {
            if CredDeleteW(target().as_ptr(), CRED_TYPE_GENERIC, 0) == 0
                && GetLastError() != ERROR_NOT_FOUND
            {
                return Err(std::io::Error::last_os_error().into());
            }
            return Ok(());
        };
        let target = target();
        let username = win::wide(&snapshot.username);
        let comment = win::wide(&snapshot.comment);
        let alias = win::wide(&snapshot.alias);
        let keywords: Vec<_> = snapshot
            .attributes
            .iter()
            .map(|(key, _, _)| win::wide(key))
            .collect();
        let mut attributes: Vec<_> = snapshot
            .attributes
            .iter()
            .enumerate()
            .map(|(index, (_, flags, value))| CREDENTIAL_ATTRIBUTEW {
                Keyword: keywords[index].as_ptr().cast_mut(),
                Flags: *flags,
                ValueSize: value.len() as u32,
                Value: value.as_ptr().cast_mut(),
            })
            .collect();
        let raw = CREDENTIALW {
            Flags: snapshot.flags,
            Type: CRED_TYPE_GENERIC,
            TargetName: target.as_ptr().cast_mut(),
            Comment: comment.as_ptr().cast_mut(),
            LastWritten: std::mem::zeroed(),
            CredentialBlobSize: snapshot.blob.len() as u32,
            CredentialBlob: snapshot.blob.as_ptr().cast_mut(),
            Persist: snapshot.persist,
            AttributeCount: attributes.len() as u32,
            Attributes: attributes.as_mut_ptr(),
            TargetAlias: alias.as_ptr().cast_mut(),
            UserName: username.as_ptr().cast_mut(),
        };
        win::check(CredWriteW(&raw, 0))
    }
}
