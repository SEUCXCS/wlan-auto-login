//! Scoped Keychain operations. Serialize the process-wide interaction switch.
use crate::infra::{
    error::{Context, Message, Result, ensure},
    identity::{APP_ID, CREDENTIAL_ENTRY, SYSTEM_CREDENTIAL_SERVICE},
    scope::Scope,
};
use crate::system::{
    define::permission::PermissionStatus,
    shared::credentials::{decode, encode},
};
use core_foundation::{
    array::{CFArray, CFArrayRef},
    base::{CFType, TCFType},
    string::{CFString, CFStringRef},
};
use security_framework::os::macos::keychain::{SecKeychain, SecPreferencesDomain};
use security_framework_sys::base::{SecAccessRef, SecKeychainItemRef};
use std::{
    ffi::{CString, c_char, c_void},
    os::unix::ffi::OsStrExt,
    path::Path,
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use wlanlogin_lib::Credentials;

static INTERACTIVE: AtomicBool = AtomicBool::new(false);
static OPERATIONS: Mutex<()> = Mutex::new(());
pub fn configure_interaction(allowed: bool) {
    INTERACTIVE.store(allowed, Ordering::Release);
}
pub fn interaction_allowed() -> bool {
    INTERACTIVE.load(Ordering::Acquire)
}
fn interactive(background: bool) -> bool {
    !background && INTERACTIVE.load(Ordering::Acquire)
}

fn with_interaction<T>(allowed: bool, operation: impl FnOnce() -> Result<T>) -> Result<T> {
    let _lock = OPERATIONS
        .lock()
        .map_err(|_| crate::infra::error::error!(Message::KeychainOperationLockPoisoned))?;
    unsafe {
        let code =
            security_framework_sys::keychain::SecKeychainSetUserInteractionAllowed(allowed.into());
        ensure!(
            code == 0,
            Message::KeychainInteractionFailed { status: code }
        );
    }
    struct Disable;
    impl Drop for Disable {
        fn drop(&mut self) {
            unsafe {
                security_framework_sys::keychain::SecKeychainSetUserInteractionAllowed(0);
            }
        }
    }
    let _disable = Disable;
    operation()
}

fn keychain(scope: Scope) -> Result<SecKeychain> {
    match scope {
        Scope::User => SecKeychain::default_for_domain(SecPreferencesDomain::User)
            .context(Message::FailedToOpenUserKeychain),
        Scope::System => SecKeychain::open("/Library/Keychains/System.keychain")
            .context(Message::FailedToOpenSystemKeychain),
    }
}
fn service(scope: Scope) -> &'static str {
    match scope {
        Scope::User => APP_ID,
        Scope::System => SYSTEM_CREDENTIAL_SERVICE,
    }
}
fn load_native(scope: Scope) -> Result<Option<Credentials>> {
    match keychain(scope)?.find_generic_password(service(scope), CREDENTIAL_ENTRY) {
        Ok((password, _)) => decode(std::str::from_utf8(password.as_ref())?).map(Some),
        Err(error) if error.code() == -25300 => Ok(None),
        Err(error) => Err(error).context(Message::KeychainCredentialInaccessible),
    }
}
pub async fn load(scope: Scope, background: bool) -> Result<Option<Credentials>> {
    tokio::task::spawn_blocking(move || {
        with_interaction(interactive(background), || load_native(scope))
    })
    .await?
}
pub async fn save(scope: Scope, credentials: Credentials) -> Result<()> {
    let text = encode(&credentials)?;
    tokio::task::spawn_blocking(move || {
        with_interaction(interactive(false), || {
            let chain = keychain(scope)?;
            let missing = match chain.find_generic_password(service(scope), CREDENTIAL_ENTRY) {
                Ok(_) => false,
                Err(error) if error.code() == -25300 => true,
                Err(error) => {
                    return Err(error).context(Message::KeychainUpdateRetainedCredential);
                }
            };
            chain
                .set_generic_password(service(scope), CREDENTIAL_ENTRY, text.as_bytes())
                .context(Message::KeychainSaveFailed)?;
            // Updating a password preserves its ACL. Installation handles upgrades.
            let installed = match scope {
                Scope::User => crate::infra::paths::user_daemon_executable()?,
                Scope::System => crate::infra::paths::system_daemon_executable()?,
            };
            if missing && installed.try_exists()? {
                authorize_native(scope, &installed)?;
            }
            Ok(())
        })
    })
    .await?
}
pub async fn delete(scope: Scope) -> Result<()> {
    tokio::task::spawn_blocking(move || {
        with_interaction(interactive(false), || {
            let (_, item) =
                match keychain(scope)?.find_generic_password(service(scope), CREDENTIAL_ENTRY) {
                    Ok(found) => found,
                    Err(error) if error.code() == -25300 => return Ok(()),
                    Err(error) => {
                        return Err(error).context(Message::KeychainDeleteAccessFailed);
                    }
                };
            let code = unsafe {
                security_framework_sys::keychain_item::SecKeychainItemDelete(
                    item.as_concrete_TypeRef(),
                )
            };
            ensure!(
                code == 0 || code == -25300,
                Message::KeychainDeleteFailed { status: code }
            );
            Ok(())
        })
    })
    .await?
}
pub fn authorize_binary(scope: Scope, binary: &Path) -> Result<()> {
    with_interaction(interactive(false), || authorize_native(scope, binary))
}

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
fn authorize_native(scope: Scope, binary: &Path) -> Result<()> {
    let (_, item) = keychain(scope)?.find_generic_password(service(scope), CREDENTIAL_ENTRY)?;
    let mut applications = Vec::new();
    let current = std::env::current_exe()?.canonicalize()?;
    let installed = binary.canonicalize()?;
    let mut paths = vec![current];
    if !paths.contains(&installed) {
        paths.push(installed);
    }
    for path in paths {
        let path = CString::new(path.as_os_str().as_bytes())?;
        unsafe {
            let mut application = std::ptr::null_mut();
            let code = SecTrustedApplicationCreateFromPath(path.as_ptr(), &mut application);
            ensure!(
                code == 0 && !application.is_null(),
                Message::KeychainTrustedApplicationFailed { status: code }
            );
            applications.push(CFType::wrap_under_create_rule(application));
        }
    }
    let applications = CFArray::from_CFTypes(&applications);
    let description = CFString::new("WLAN Login saved campus credentials");
    unsafe {
        let mut access = std::ptr::null_mut();
        let code = SecAccessCreate(
            description.as_concrete_TypeRef(),
            applications.as_concrete_TypeRef(),
            &mut access,
        );
        ensure!(
            code == 0 && !access.is_null(),
            Message::KeychainAccessCreateFailed { status: code }
        );
        let owned = CFType::wrap_under_create_rule(access.cast());
        let code = SecKeychainItemSetAccess(item.as_concrete_TypeRef(), access);
        drop(owned);
        ensure!(code == 0, Message::KeychainAuthorizeFailed { status: code });
    }
    Ok(())
}
pub async fn status(scope: Scope, request: bool) -> Result<PermissionStatus> {
    tokio::task::spawn_blocking(move || {
        match with_interaction(request && interactive(false), || load_native(scope)) {
            Ok(Some(_)) => Ok(PermissionStatus::Granted),
            Ok(None) => Ok(PermissionStatus::Missing),
            Err(error) => match error.downcast_ref::<security_framework::base::Error>() {
                Some(native) => Ok(classify(native.code())),
                None => Err(error),
            },
        }
    })
    .await?
}
fn classify(code: i32) -> PermissionStatus {
    match code {
        -25300 => PermissionStatus::Missing,
        -25308 | -25315 => PermissionStatus::NeedsInteraction,
        -25293 | -128 => PermissionStatus::Denied,
        -25291 | -25294 => PermissionStatus::Unavailable,
        _ => PermissionStatus::Unknown,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_locked_and_denied_credentials_have_distinct_results() {
        assert_eq!(classify(-25300), PermissionStatus::Missing);
        assert_eq!(classify(-25308), PermissionStatus::NeedsInteraction);
        assert_eq!(classify(-25293), PermissionStatus::Denied);
        assert_eq!(classify(-128), PermissionStatus::Denied);
        assert_eq!(classify(-25291), PermissionStatus::Unavailable);
        assert_eq!(classify(-99999), PermissionStatus::Unknown);
    }
}
