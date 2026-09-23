use anyhow::{Context, Result};
use core_foundation::{
    array::{CFArray, CFArrayRef},
    base::{CFType, TCFType},
    string::{CFString, CFStringRef},
};
use cxxy_wlanlogin_lib::Credentials;
use security_framework::os::macos::keychain::SecKeychain;
use security_framework_sys::base::{SecAccessRef, SecKeychainItemRef};
use std::{
    ffi::{CString, c_char, c_void},
    os::unix::ffi::OsStrExt,
    path::Path,
};

const SERVICE: &str = "org.seucxcs.cxxy-wlanlogin.system";
const ACCOUNT: &str = "saved-login";

#[link(name = "Security", kind = "framework")]
unsafe extern "C" {
    fn SecTrustedApplicationCreateFromPath(path: *const c_char, app: *mut *mut c_void) -> i32;
    fn SecAccessCreate(
        description: CFStringRef,
        applications: CFArrayRef,
        access: *mut SecAccessRef,
    ) -> i32;
    fn SecKeychainItemSetAccess(item: SecKeychainItemRef, access: SecAccessRef) -> i32;
}

fn keychain() -> Result<SecKeychain> {
    SecKeychain::open("/Library/Keychains/System.keychain").context("无法打开 System Keychain")
}

pub fn delete() -> Result<()> {
    let (_, item) = match keychain()?.find_generic_password(SERVICE, ACCOUNT) {
        Ok(found) => found,
        Err(error) if error.code() == -25300 => return Ok(()),
        Err(error) => return Err(error).context("无法查找待删除的 System Keychain 凭据"),
    };
    // SAFETY: item is a live, retained reference from the selected system keychain.
    // The high-level delete wrapper discards OSStatus, so check it explicitly.
    let code = unsafe {
        security_framework_sys::keychain_item::SecKeychainItemDelete(item.as_concrete_TypeRef())
    };
    anyhow::ensure!(
        code == 0 || code == -25300,
        "无法删除 System Keychain 凭据：{code}"
    );
    Ok(())
}

pub fn load() -> Result<Option<Credentials>> {
    match keychain()?.find_generic_password(SERVICE, ACCOUNT) {
        Ok((password, _)) => {
            super::super::decode(std::str::from_utf8(password.as_ref())?).map(Some)
        }
        Err(error) if error.code() == -25300 => Ok(None),
        Err(error) => Err(error).context("无法读取 System Keychain 凭据，请检查服务程序访问权限"),
    }
}

pub fn save(text: &str) -> Result<()> {
    keychain()?
        .set_generic_password(SERVICE, ACCOUNT, text.as_bytes())
        .context("无法保存 System Keychain 凭据")?;
    let current = std::env::current_exe()?;
    let installed =
        Path::new("/Library/Application Support/cxxy-wlanlogin/bin/cxxy-wlanlogin-daemon");
    authorize_binary(if installed.try_exists()? {
        installed
    } else {
        &current
    })
}

pub fn authorize_binary(binary: &Path) -> Result<()> {
    let (_, item) = keychain()?.find_generic_password(SERVICE, ACCOUNT)?;
    let mut applications = Vec::new();
    for path in [std::env::current_exe()?, binary.to_owned()] {
        let path = CString::new(path.as_os_str().as_bytes())?;
        // SAFETY: Security creates a CF object retained by CFType until after setting the ACL.
        unsafe {
            let mut application = std::ptr::null_mut();
            let result = SecTrustedApplicationCreateFromPath(path.as_ptr(), &mut application);
            anyhow::ensure!(result == 0, "无法授权服务程序读取系统凭据: {result}");
            applications.push(CFType::wrap_under_create_rule(application));
        }
    }
    let applications = CFArray::from_CFTypes(&applications);
    let description = CFString::new("Student_CX system service credentials");
    // SAFETY: all objects remain alive; access is released on success and on failure.
    unsafe {
        let mut access = std::ptr::null_mut();
        let result = SecAccessCreate(
            description.as_concrete_TypeRef(),
            applications.as_concrete_TypeRef(),
            &mut access,
        );
        anyhow::ensure!(result == 0, "无法创建系统凭据访问控制: {result}");
        let owned = CFType::wrap_under_create_rule(access.cast());
        let result = SecKeychainItemSetAccess(item.as_concrete_TypeRef(), access);
        drop(owned);
        anyhow::ensure!(result == 0, "无法设置系统凭据访问控制: {result}");
    }
    Ok(())
}
