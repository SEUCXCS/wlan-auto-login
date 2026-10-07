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
    base::{CFType, CFTypeRef, TCFType},
    string::{CFString, CFStringRef},
};
use security_framework::os::macos::{
    keychain::{SecKeychain, SecPreferencesDomain},
    keychain_item::SecKeychainItem,
};
use security_framework_sys::base::{
    SecAccessRef, SecKeychainAttribute, SecKeychainAttributeList, SecKeychainItemRef,
    SecKeychainRef,
};
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

/// The installed process reports only access state, never credential data.
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CredentialCheck {
    version: u32,
    status: PermissionStatus,
}
impl CredentialCheck {
    pub fn new(status: PermissionStatus) -> Self {
        Self { version: 1, status }
    }
    pub fn into_status(self) -> Result<PermissionStatus> {
        ensure!(self.version == 1, Message::InstalledCredentialCheckProtocol);
        Ok(self.status)
    }
}
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

/// Find the item reference without asking Security to decrypt its password.
fn find_item(chain: &SecKeychain, scope: Scope) -> Result<Option<SecKeychainItem>> {
    let service = service(scope).as_bytes();
    let account = CREDENTIAL_ENTRY.as_bytes();
    let mut item = std::ptr::null_mut();
    // SAFETY: the chain and attribute buffers remain live. Null password outputs
    // request only the retained item reference, as documented by Security.
    let code = unsafe {
        security_framework_sys::keychain::SecKeychainFindGenericPassword(
            chain.as_CFTypeRef(),
            service.len() as u32,
            service.as_ptr().cast(),
            account.len() as u32,
            account.as_ptr().cast(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut item,
        )
    };
    match code {
        0 => {
            ensure!(!item.is_null(), Message::KeychainCredentialInaccessible);
            // SAFETY: a successful lookup transfers ownership of the item reference.
            Ok(Some(unsafe {
                SecKeychainItem::wrap_under_create_rule(item)
            }))
        }
        -25300 => Ok(None),
        code => Err(security_framework::base::Error::from_code(code))
            .context(Message::KeychainCredentialInaccessible),
    }
}

/// Create only after an explicit missing result; access/update failures retain the item.
fn save_item<T>(
    found: Result<Option<T>>,
    update: impl FnOnce(T) -> Result<()>,
    add: impl FnOnce() -> Result<()>,
) -> Result<bool> {
    match found.context(Message::KeychainUpdateRetainedCredential)? {
        Some(item) => {
            update(item)?;
            Ok(false)
        }
        None => {
            add()?;
            Ok(true)
        }
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
            save_item(
                find_item(&chain, scope),
                |mut item| {
                    item.set_password(text.as_bytes())
                        .context(Message::KeychainSaveFailed)
                },
                || add_native(&chain, scope, text.as_bytes()),
            )?;
            Ok(())
        })
    })
    .await?
}
pub async fn delete(scope: Scope) -> Result<()> {
    tokio::task::spawn_blocking(move || {
        with_interaction(interactive(false), || {
            let Some(item) =
                find_item(&keychain(scope)?, scope).context(Message::KeychainDeleteAccessFailed)?
            else {
                return Ok(());
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

/// A locked keychain needs unlocking, not an ACL rewrite. The caller rechecks
/// access in the installed process after unlocking and before changing any ACL.
pub fn unlock_if_needed(scope: Scope) -> Result<bool> {
    with_interaction(interactive(false), || {
        let mut chain = keychain(scope)?;
        let mut status = 0;
        // SAFETY: the live chain and status output have the types required by Security.
        let code = unsafe { SecKeychainGetStatus(chain.as_concrete_TypeRef(), &mut status) };
        if code != 0 {
            return Err(security_framework::base::Error::from_code(code))
                .context(Message::KeychainUnlockFailed);
        }
        if status & 1 != 0 {
            // kSecUnlockStateStatus
            return Ok(false);
        }
        chain.unlock(None).context(Message::KeychainUnlockFailed)?;
        Ok(true)
    })
}

#[link(name = "Security", kind = "framework")]
unsafe extern "C" {
    fn SecTrustedApplicationCreateFromPath(path: *const c_char, app: *mut *mut c_void) -> i32;
    fn SecAccessCreate(
        description: CFStringRef,
        applications: CFArrayRef,
        access: *mut SecAccessRef,
    ) -> i32;
    fn SecKeychainGetStatus(chain: SecKeychainRef, status: *mut u32) -> i32;
    fn SecKeychainItemCreateFromContent(
        class: u32,
        attributes: *mut SecKeychainAttributeList,
        length: u32,
        data: *const c_void,
        chain: SecKeychainRef,
        access: SecAccessRef,
        item: *mut SecKeychainItemRef,
    ) -> i32;
    fn SecKeychainItemCopyAccess(item: SecKeychainItemRef, access: *mut SecAccessRef) -> i32;
    fn SecKeychainItemSetAccess(item: SecKeychainItemRef, access: SecAccessRef) -> i32;
    static kSecACLAuthorizationDecrypt: CFStringRef;
    fn SecAccessCopyMatchingACLList(access: SecAccessRef, authorization: CFTypeRef) -> CFArrayRef;
    fn SecACLCopyAuthorizations(acl: CFTypeRef) -> CFArrayRef;
    fn SecACLCopyContents(
        acl: CFTypeRef,
        applications: *mut CFArrayRef,
        description: *mut CFStringRef,
        prompt: *mut u16,
    ) -> i32;
    fn SecACLSetContents(
        acl: CFTypeRef,
        applications: CFArrayRef,
        description: CFStringRef,
        prompt: u16,
    ) -> i32;
    fn SecACLCreateWithSimpleContents(
        access: SecAccessRef,
        applications: CFArrayRef,
        description: CFStringRef,
        prompt: u16,
        acl: *mut CFTypeRef,
    ) -> i32;
    fn SecACLUpdateAuthorizations(acl: CFTypeRef, authorizations: CFArrayRef) -> i32;
}
fn trusted_applications(binary: Option<&Path>) -> Result<Vec<CFType>> {
    let mut applications = Vec::new();
    let current = std::env::current_exe()?.canonicalize()?;
    let mut paths = vec![current];
    if let Some(binary) = binary {
        let installed = binary.canonicalize()?;
        if !paths.contains(&installed) {
            paths.push(installed);
        }
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
    Ok(applications)
}

fn initial_access(binary: Option<&Path>) -> Result<CFType> {
    let applications = CFArray::from_CFTypes(&trusted_applications(binary)?);
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
        Ok(CFType::wrap_under_create_rule(access.cast()))
    }
}

fn add_native(chain: &SecKeychain, scope: Scope, password: &[u8]) -> Result<()> {
    let installed = match scope {
        Scope::User => crate::infra::paths::user_daemon_executable()?,
        Scope::System => crate::infra::paths::system_daemon_executable()?,
    };
    let access = initial_access(installed.try_exists()?.then_some(installed.as_path()))?;
    let attribute = |tag, value: &[u8]| SecKeychainAttribute {
        tag: u32::from_be_bytes(tag),
        length: value.len() as u32,
        data: value.as_ptr().cast_mut().cast(),
    };
    let mut attributes = [
        attribute(*b"svce", service(scope).as_bytes()),
        attribute(*b"acct", CREDENTIAL_ENTRY.as_bytes()),
    ];
    let mut list = SecKeychainAttributeList {
        count: attributes.len() as u32,
        attr: attributes.as_mut_ptr(),
    };
    // SAFETY: all attribute and password buffers and the access remain live for
    // the call. A null item output avoids allocating an unused retained item.
    let code = unsafe {
        SecKeychainItemCreateFromContent(
            u32::from_be_bytes(*b"genp"),
            &mut list,
            password.len() as u32,
            password.as_ptr().cast(),
            chain.as_concrete_TypeRef(),
            access.as_CFTypeRef().cast_mut().cast(),
            std::ptr::null_mut(),
        )
    };
    if code != 0 {
        return Err(security_framework::base::Error::from_code(code))
            .context(Message::KeychainSaveFailed);
    }
    Ok(())
}

fn authorize_native(scope: Scope, binary: &Path) -> Result<()> {
    let item = find_item(&keychain(scope)?, scope)?.context(Message::SavedCredentialsMissing)?;
    let applications = trusted_applications(Some(binary))?;
    // SAFETY: CopyAccess returns a retained reference; only a copied read ACL is
    // edited. Owner, partition constraints and unrelated ACLs stay in this access.
    unsafe {
        let mut access = std::ptr::null_mut();
        let code = SecKeychainItemCopyAccess(item.as_concrete_TypeRef(), &mut access);
        ensure!(
            code == 0 && !access.is_null(),
            Message::KeychainAccessCopyFailed { status: code }
        );
        let owned = CFType::wrap_under_create_rule(access.cast());
        if extend_read_access(&owned, &applications)? {
            let code = SecKeychainItemSetAccess(item.as_concrete_TypeRef(), access);
            ensure!(code == 0, Message::KeychainAuthorizeFailed { status: code });
        }
    }
    Ok(())
}

/// Extend one decrypt-only ACL. A mixed authorization ACL must remain unchanged:
/// adding an application there could grant delete/change-owner permissions too.
fn extend_read_access(access: &CFType, applications: &[CFType]) -> Result<bool> {
    unsafe {
        let access = access.as_CFTypeRef().cast_mut().cast();
        let tag = CFString::wrap_under_get_rule(kSecACLAuthorizationDecrypt);
        let matching = SecAccessCopyMatchingACLList(access, tag.as_CFTypeRef());
        // Security returns NULL when there is no matching ACL (for example,
        // a SecAccessCreate default containing only an Any authorization).
        let matching = if matching.is_null() {
            CFArray::<CFType>::from_CFTypes(&[])
        } else {
            CFArray::<CFType>::wrap_under_create_rule(matching)
        };
        for acl in &matching {
            let tags = SecACLCopyAuthorizations(acl.as_CFTypeRef());
            ensure!(
                !tags.is_null(),
                Message::KeychainReadAccessFailed { status: -50 }
            );
            let tags = CFArray::<CFString>::wrap_under_create_rule(tags);
            if tags.len() != 1 || !tags.get(0).is_some_and(|value| *value == tag) {
                continue;
            }
            let mut apps = std::ptr::null();
            let mut description = std::ptr::null();
            let mut prompt = 0;
            let code =
                SecACLCopyContents(acl.as_CFTypeRef(), &mut apps, &mut description, &mut prompt);
            ensure!(
                code == 0 && !description.is_null(),
                Message::KeychainReadAccessFailed { status: code }
            );
            let description = CFString::wrap_under_create_rule(description);
            // NULL means unrestricted applications. Preserve it rather than
            // narrowing a user-selected policy or rewriting for a locked keychain.
            if apps.is_null() {
                return Ok(false);
            }
            let apps = CFArray::<CFType>::wrap_under_create_rule(apps);
            let mut extended: Vec<CFType> = apps.iter().map(|app| (*app).clone()).collect();
            for app in applications {
                if !extended.contains(app) {
                    extended.push(app.clone());
                }
            }
            if extended.len() == apps.len() as usize {
                return Ok(false);
            }
            let extended = CFArray::from_CFTypes(&extended);
            let code = SecACLSetContents(
                acl.as_CFTypeRef(),
                extended.as_concrete_TypeRef(),
                description.as_concrete_TypeRef(),
                prompt,
            );
            ensure!(
                code == 0,
                Message::KeychainReadAccessFailed { status: code }
            );
            return Ok(true);
        }
        // No decrypt-only entry: add one without changing mixed ACLs or owner.
        let apps = CFArray::from_CFTypes(applications);
        let description = CFString::new("WLAN Login saved campus credentials");
        let mut acl = std::ptr::null();
        let code = SecACLCreateWithSimpleContents(
            access,
            apps.as_concrete_TypeRef(),
            description.as_concrete_TypeRef(),
            1,
            &mut acl,
        );
        ensure!(
            code == 0 && !acl.is_null(),
            Message::KeychainReadAccessFailed { status: code }
        );
        let acl = CFType::wrap_under_create_rule(acl);
        let tags = CFArray::from_CFTypes(&[tag]);
        let code = SecACLUpdateAuthorizations(acl.as_CFTypeRef(), tags.as_concrete_TypeRef());
        ensure!(
            code == 0,
            Message::KeychainReadAccessFailed { status: code }
        );
        Ok(true)
    }
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

    unsafe extern "C" {
        fn SecAccessCopyACLList(access: SecAccessRef, acls: *mut CFArrayRef) -> i32;
        fn SecAccessCreateWithOwnerAndACL(
            uid: u32,
            gid: u32,
            owner_type: u32,
            acls: CFArrayRef,
            error: *mut CFTypeRef,
        ) -> SecAccessRef;
        fn SecAccessCopyOwnerAndACL(
            access: SecAccessRef,
            uid: *mut u32,
            gid: *mut u32,
            owner_type: *mut u32,
            acls: *mut CFArrayRef,
        ) -> i32;
        static kSecACLAuthorizationChangeOwner: CFStringRef;
    }

    #[derive(Debug, PartialEq)]
    struct AclSnapshot {
        tags: Vec<String>,
        contents_status: i32,
        apps: Option<Vec<CFType>>,
        description: String,
        prompt: u16,
    }

    fn acls(access: &CFType) -> CFArray<CFType> {
        unsafe {
            let mut acls = std::ptr::null();
            assert_eq!(
                SecAccessCopyACLList(access.as_CFTypeRef().cast_mut().cast(), &mut acls),
                0
            );
            assert!(!acls.is_null());
            CFArray::wrap_under_create_rule(acls)
        }
    }

    fn snapshot(acl: &CFType) -> AclSnapshot {
        unsafe {
            let tags = CFArray::<CFString>::wrap_under_create_rule(SecACLCopyAuthorizations(
                acl.as_CFTypeRef(),
            ));
            let mut apps = std::ptr::null();
            let mut description = std::ptr::null();
            let mut prompt = 0;
            let contents_status =
                SecACLCopyContents(acl.as_CFTypeRef(), &mut apps, &mut description, &mut prompt);
            // UID owner ACLs have no simple application-list representation.
            if contents_status != 0 {
                assert_eq!(contents_status, -25240); // errSecInvalidACL
                return AclSnapshot {
                    tags: tags.iter().map(|tag| tag.to_string()).collect(),
                    contents_status,
                    apps: None,
                    description: String::new(),
                    prompt: 0,
                };
            }
            let apps = (!apps.is_null()).then(|| {
                CFArray::<CFType>::wrap_under_create_rule(apps)
                    .iter()
                    .map(|app| (*app).clone())
                    .collect()
            });
            AclSnapshot {
                tags: tags.iter().map(|tag| tag.to_string()).collect(),
                contents_status,
                apps,
                description: CFString::wrap_under_create_rule(description).to_string(),
                prompt,
            }
        }
    }

    fn owner(access: &CFType) -> (u32, u32, u32) {
        let (mut uid, mut gid, mut owner_type) = (0, 0, 0);
        unsafe {
            assert_eq!(
                SecAccessCopyOwnerAndACL(
                    access.as_CFTypeRef().cast_mut().cast(),
                    &mut uid,
                    &mut gid,
                    &mut owner_type,
                    std::ptr::null_mut()
                ),
                0
            );
        }
        (uid, gid, owner_type)
    }

    // In-memory SecAccess fixtures exercise the native ACL API without opening,
    // reading or modifying any real Keychain, credential or user service.
    fn access_fixture() -> Result<CFType> {
        let original = initial_access(None)?;
        let acls = acls(&original);
        unsafe {
            let access = SecAccessCreateWithOwnerAndACL(
                501,
                20,
                1,
                acls.as_concrete_TypeRef(),
                std::ptr::null_mut(),
            );
            assert!(!access.is_null());
            let access = CFType::wrap_under_create_rule(access.cast());
            assert!(extend_read_access(&access, &trusted_applications(None)?)?);
            Ok(access)
        }
    }

    fn decrypt_acl(access: &CFType) -> Result<CFType> {
        unsafe {
            let matches = SecAccessCopyMatchingACLList(
                access.as_CFTypeRef().cast_mut().cast(),
                kSecACLAuthorizationDecrypt.cast(),
            );
            assert!(!matches.is_null());
            let matches = CFArray::<CFType>::wrap_under_create_rule(matches);
            matches
                .get(0)
                .map(|acl| (*acl).clone())
                .context("Fixture has no decrypt ACL")
        }
    }

    #[test]
    fn extending_native_read_acl_preserves_owner_other_acls_and_prompt_policy() -> Result<()> {
        let access = access_fixture()?;
        let decrypt = decrypt_acl(&access)?;
        let before = snapshot(&decrypt);
        let apps = CFArray::from_CFTypes(
            before
                .apps
                .as_ref()
                .context("Fixture has no read applications")?,
        );
        let description = CFString::new("Isolated credential ACL");
        unsafe {
            assert_eq!(
                SecACLSetContents(
                    decrypt.as_CFTypeRef(),
                    apps.as_concrete_TypeRef(),
                    description.as_concrete_TypeRef(),
                    0x51
                ),
                0
            );
        }
        let owner_before = owner(&access);
        let read_before = snapshot(&decrypt);
        let unrelated_before: Vec<_> = acls(&access)
            .iter()
            .filter(|acl| **acl != decrypt)
            .map(|acl| snapshot(&acl))
            .collect();
        let added = trusted_applications(Some(Path::new("/usr/bin/true")))?;
        assert!(extend_read_access(&access, &added)?);
        assert_eq!(owner(&access), owner_before);
        assert_eq!(
            acls(&access)
                .iter()
                .filter(|acl| **acl != decrypt)
                .map(|acl| snapshot(&acl))
                .collect::<Vec<_>>(),
            unrelated_before
        );
        let after = snapshot(&decrypt);
        assert_eq!(after.tags, read_before.tags);
        assert_eq!(after.description, read_before.description);
        assert_eq!(after.prompt, read_before.prompt);
        let after_apps = after.apps.context("Updated ACL has no applications")?;
        assert!(
            read_before
                .apps
                .context("Fixture has no read applications")?
                .iter()
                .all(|app| after_apps.contains(app))
        );
        assert!(added.iter().all(|app| after_apps.contains(app)));
        // Applying the same identities again must not cause a SetAccess write.
        assert!(!extend_read_access(&access, &added)?);
        Ok(())
    }

    #[test]
    fn mixed_read_and_owner_acl_is_retained_when_adding_a_read_only_entry() -> Result<()> {
        let access = access_fixture()?;
        let decrypt = decrypt_acl(&access)?;
        unsafe {
            let tags = CFArray::from_CFTypes(&[
                CFString::wrap_under_get_rule(kSecACLAuthorizationDecrypt),
                CFString::wrap_under_get_rule(kSecACLAuthorizationChangeOwner),
            ]);
            assert_eq!(
                SecACLUpdateAuthorizations(decrypt.as_CFTypeRef(), tags.as_concrete_TypeRef()),
                0
            );
        }
        let owner_before = owner(&access);
        let before: Vec<_> = acls(&access).iter().map(|acl| snapshot(&acl)).collect();
        assert!(extend_read_access(
            &access,
            &trusted_applications(Some(Path::new("/usr/bin/true")))?
        )?);
        let after: Vec<_> = acls(&access).iter().map(|acl| snapshot(&acl)).collect();
        assert_eq!(owner(&access), owner_before);
        assert_eq!(after.len(), before.len() + 1);
        assert!(before.iter().all(|acl| after.contains(acl)));
        let tag = unsafe { CFString::wrap_under_get_rule(kSecACLAuthorizationDecrypt) }.to_string();
        assert_eq!(
            after
                .iter()
                .find(|acl| !before.contains(acl))
                .context("No read ACL was added")?
                .tags,
            [tag]
        );
        Ok(())
    }

    #[test]
    fn unrestricted_read_acl_is_not_narrowed_or_rewritten() -> Result<()> {
        let access = access_fixture()?;
        let decrypt = decrypt_acl(&access)?;
        let before = snapshot(&decrypt);
        let description = CFString::new(&before.description);
        unsafe {
            assert_eq!(
                SecACLSetContents(
                    decrypt.as_CFTypeRef(),
                    std::ptr::null(),
                    description.as_concrete_TypeRef(),
                    before.prompt
                ),
                0
            );
        }
        let before = snapshot(&decrypt);
        assert!(!extend_read_access(
            &access,
            &trusted_applications(Some(Path::new("/usr/bin/true")))?
        )?);
        assert_eq!(snapshot(&decrypt), before);
        Ok(())
    }

    #[test]
    fn failed_lookup_or_update_never_creates_a_replacement_credential() {
        for code in [-25308, -25293] {
            let result = save_item::<()>(
                Err(security_framework::base::Error::from_code(code))
                    .context(Message::KeychainCredentialInaccessible),
                |_| panic!("inaccessible items must not be updated"),
                || panic!("lookup failure must not create a new item"),
            );
            assert!(result.is_err());
        }
        let result = save_item(
            Ok(Some(())),
            |_| crate::infra::error::bail!("update denied"),
            || panic!("update failure must not create a new item"),
        );
        assert!(result.is_err());
    }

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
