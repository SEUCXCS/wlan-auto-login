//! Validate ownership and permissions before trusting Windows application storage.
use crate::infra::{
    error::{Context, Result, ensure},
    identity::{APP_NAME, ORGANIZATION},
    scope,
};
use std::{
    ffi::c_void,
    fs::{File, OpenOptions},
    io::Write,
    os::windows::{ffi::OsStrExt, fs::MetadataExt, fs::OpenOptionsExt, io::AsRawHandle},
    path::{Path, PathBuf},
    ptr,
};
use windows_sys::Win32::{
    Foundation::{ERROR_ALREADY_EXISTS, LocalFree},
    Security::{
        ACCESS_ALLOWED_ACE, ACE_HEADER, ACL,
        Authorization::{
            ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
            GetSecurityInfo, SE_FILE_OBJECT, SetSecurityInfo,
        },
        DACL_SECURITY_INFORMATION, GetAce, GetSecurityDescriptorDacl, GetSecurityDescriptorOwner,
        INHERIT_ONLY_ACE, OWNER_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION,
        PSECURITY_DESCRIPTOR, PSID, SECURITY_ATTRIBUTES,
    },
    Storage::FileSystem::{
        CreateDirectoryW, FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS,
        FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ,
        FILE_SHARE_WRITE,
    },
    System::SystemServices::{ACCESS_ALLOWED_ACE_TYPE, ACCESS_DENIED_ACE_TYPE},
};

const READ_CONTROL: u32 = 0x0002_0000;
const WRITE_DAC: u32 = 0x0004_0000;
const WRITE_OWNER: u32 = 0x0008_0000;
// File-specific mutation rights, DELETE, WRITE_DAC/OWNER, GENERIC_WRITE/ALL.
const MUTATION: u32 = 0x500d_0156;
// An OS base such as ProgramData may allow creating children, but not replacing
// existing application directories or changing the base's security/reparse data.
const BASE_MUTATION: u32 = 0x100d_0040;
const SYSTEM: &str = "S-1-5-18";
const ADMINISTRATORS: &str = "S-1-5-32-544";
const TRUSTED_INSTALLER: &str = "S-1-5-80-956008885-3418522649-1831038044-1853292631-2271478464";

struct Policy {
    owner: String,
    trusted: Vec<String>,
}

impl Policy {
    fn machine() -> Self {
        Self {
            owner: "BA".into(),
            trusted: [SYSTEM, ADMINISTRATORS, TRUSTED_INSTALLER]
                .map(str::to_owned)
                .into(),
        }
    }

    fn process() -> Result<Self> {
        let user = scope::current_sid()?;
        let mut policy = Self::machine();
        if !scope::elevated()? {
            policy.owner = user.clone();
        }
        // User directories and test fixtures may be owned by the current user.
        // Machine credentials always use the stricter machine policy explicitly.
        policy.trusted.push(user);
        Ok(policy)
    }

    fn trusts(&self, sid: PSID) -> Result<bool> {
        Ok(self.trusted.contains(&sid_string(sid)?))
    }

    fn descriptor(&self, public: bool) -> Result<Descriptor> {
        let read = if public { "(A;OICI;GRGX;;;BU)" } else { "" };
        Descriptor::new(&format!(
            "O:{}D:P(A;OICI;FA;;;SY)(A;OICI;FA;;;{}){read}",
            self.owner, self.owner
        ))
    }
}

struct Descriptor(PSECURITY_DESCRIPTOR);

impl Descriptor {
    fn new(sddl: &str) -> Result<Self> {
        let sddl: Vec<u16> = sddl.encode_utf16().chain(Some(0)).collect();
        let mut descriptor = ptr::null_mut();
        // SAFETY: Windows allocates a self-contained descriptor owned by the guard.
        if unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                1,
                &mut descriptor,
                ptr::null_mut(),
            )
        } == 0
        {
            return Err(std::io::Error::last_os_error().into());
        }
        Ok(Self(descriptor))
    }

    fn apply(&self, file: &File, owner: bool) -> Result<()> {
        let mut dacl = ptr::null_mut();
        let mut sid = ptr::null_mut();
        let mut present = 0;
        let mut defaulted = 0;
        // SAFETY: all returned pointers remain inside this live descriptor.
        unsafe {
            if GetSecurityDescriptorDacl(self.0, &mut present, &mut dacl, &mut defaulted) == 0
                || GetSecurityDescriptorOwner(self.0, &mut sid, &mut defaulted) == 0
            {
                return Err(std::io::Error::last_os_error().into());
            }
            let flags = DACL_SECURITY_INFORMATION
                | PROTECTED_DACL_SECURITY_INFORMATION
                | if owner { OWNER_SECURITY_INFORMATION } else { 0 };
            let code = SetSecurityInfo(
                file.as_raw_handle(),
                SE_FILE_OBJECT,
                flags,
                if owner { sid } else { ptr::null_mut() },
                ptr::null_mut(),
                dacl,
                ptr::null(),
            );
            if code != 0 {
                return Err(std::io::Error::from_raw_os_error(code as i32).into());
            }
        }
        Ok(())
    }
}

impl Drop for Descriptor {
    fn drop(&mut self) {
        // SAFETY: both descriptor constructors below allocate with LocalAlloc.
        unsafe {
            LocalFree(self.0);
        }
    }
}

fn sid_string(sid: PSID) -> Result<String> {
    let mut string = ptr::null_mut();
    // SAFETY: callers supply a SID from a Windows-produced security descriptor.
    unsafe {
        if ConvertSidToStringSidW(sid, &mut string) == 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        let mut length = 0;
        while *string.add(length) != 0 {
            length += 1;
        }
        let result = String::from_utf16_lossy(std::slice::from_raw_parts(string, length));
        LocalFree(string.cast());
        Ok(result)
    }
}

fn inspect(file: &File, path: &Path, policy: &Policy, forbidden: u32) -> Result<()> {
    let metadata = file.metadata()?;
    ensure!(
        metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT == 0,
        "Refusing Windows storage through a reparse point: {}",
        path.display()
    );
    let mut owner = ptr::null_mut();
    let mut dacl: *mut ACL = ptr::null_mut();
    let mut descriptor = ptr::null_mut();
    // SAFETY: the handle is live; the descriptor guard owns all returned pointers.
    unsafe {
        let code = GetSecurityInfo(
            file.as_raw_handle(),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            &mut owner,
            ptr::null_mut(),
            &mut dacl,
            ptr::null_mut(),
            &mut descriptor,
        );
        if code != 0 {
            return Err(std::io::Error::from_raw_os_error(code as i32).into());
        }
        let _descriptor = Descriptor(descriptor);
        ensure!(
            !owner.is_null() && policy.trusts(owner)?,
            "Untrusted Windows storage owner: {}",
            path.display()
        );
        ensure!(
            !dacl.is_null(),
            "Windows storage has no restrictive DACL: {}",
            path.display()
        );
        for index in 0..(*dacl).AceCount as u32 {
            let mut ace: *mut c_void = ptr::null_mut();
            if GetAce(dacl, index, &mut ace) == 0 {
                return Err(std::io::Error::last_os_error().into());
            }
            let header = &*ace.cast::<ACE_HEADER>();
            if header.AceFlags & INHERIT_ONLY_ACE as u8 != 0 {
                continue;
            }
            match header.AceType as u32 {
                ACCESS_DENIED_ACE_TYPE => {}
                ACCESS_ALLOWED_ACE_TYPE => {
                    let allowed = &*ace.cast::<ACCESS_ALLOWED_ACE>();
                    let sid = (&raw const allowed.SidStart).cast_mut().cast();
                    ensure!(
                        allowed.Mask & forbidden == 0 || policy.trusts(sid)?,
                        "Unsafe Windows storage permissions: {}",
                        path.display()
                    );
                }
                _ => ensure!(
                    false,
                    "Unsupported Windows storage DACL: {}",
                    path.display()
                ),
            }
        }
    }
    Ok(())
}

fn open(path: &Path, access: u32, directory: bool) -> Result<File> {
    let file = OpenOptions::new()
        .access_mode(access | FILE_READ_ATTRIBUTES)
        // Pin directories. Leaf readers retain the inspected file object while
        // allowing a legitimate credential save to replace its directory entry.
        .share_mode(
            FILE_SHARE_READ
                | if directory {
                    FILE_SHARE_WRITE
                } else {
                    FILE_SHARE_DELETE
                },
        )
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS)
        .open(path)?;
    let metadata = file.metadata()?;
    ensure!(
        if directory {
            metadata.is_dir()
        } else {
            metadata.is_file()
        },
        "Invalid Windows storage object type: {}",
        path.display()
    );
    Ok(file)
}

/// Keep ancestors open without delete sharing until the storage operation ends.
pub(crate) struct DirectoryGuard {
    _handles: Vec<File>,
}

fn directory_chain(path: &Path) -> Result<Vec<PathBuf>> {
    let base = if let Some(app) = path.ancestors().find(|part| {
        part.file_name().is_some_and(|name| name == APP_NAME)
            && part
                .parent()
                .and_then(Path::file_name)
                .is_some_and(|name| name == ORGANIZATION)
    }) {
        app.parent().and_then(Path::parent)
    } else {
        path.ancestors().skip(1).find(|part| part.is_dir())
    }
    .context("Windows application storage has no base directory")?;
    let mut chain = path
        .ancestors()
        .take_while(|part| *part != base)
        .map(Path::to_owned)
        .collect::<Vec<_>>();
    chain.push(base.to_owned());
    chain.reverse();
    Ok(chain)
}

fn directory(path: &Path, create: bool, public: bool, policy: &Policy) -> Result<DirectoryGuard> {
    let chain = directory_chain(path)?;
    let mut handles = Vec::new();
    for (index, part) in chain.iter().enumerate() {
        let last = index + 1 == chain.len();
        let access =
            READ_CONTROL | FILE_READ_ATTRIBUTES | if create && last { WRITE_DAC } else { 0 };
        let file = match open(part, access, true) {
            Ok(file) => file,
            Err(error)
                if create
                    && error
                        .downcast_ref::<std::io::Error>()
                        .is_some_and(|error| error.kind() == std::io::ErrorKind::NotFound) =>
            {
                let descriptor =
                    policy.descriptor(if last { public } else { policy.owner == "BA" })?;
                let attributes = SECURITY_ATTRIBUTES {
                    nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
                    lpSecurityDescriptor: descriptor.0,
                    bInheritHandle: 0,
                };
                let wide: Vec<u16> = part.as_os_str().encode_wide().chain(Some(0)).collect();
                // SAFETY: the parent is already validated and pinned; security is assigned at creation.
                if unsafe { CreateDirectoryW(wide.as_ptr(), &attributes) } == 0 {
                    let error = std::io::Error::last_os_error();
                    if error.raw_os_error() != Some(ERROR_ALREADY_EXISTS as i32) {
                        return Err(error.into());
                    }
                }
                open(part, access, true)?
            }
            Err(error) => return Err(error),
        };
        inspect(
            &file,
            part,
            policy,
            if index == 0 { BASE_MUTATION } else { MUTATION },
        )?;
        if create && last {
            policy.descriptor(public)?.apply(&file, false)?;
        }
        handles.push(file);
    }
    Ok(DirectoryGuard { _handles: handles })
}

pub(super) fn prepare_directory(path: &Path, public: bool) -> Result<DirectoryGuard> {
    directory(path, true, public, &Policy::process()?)
}

pub(crate) fn machine_directory(path: &Path, create: bool) -> Result<DirectoryGuard> {
    directory(path, create, false, &Policy::machine())
}

pub(crate) fn open_machine_file(path: &Path) -> Result<(File, DirectoryGuard)> {
    let guard = machine_directory(
        path.parent().context("Credential file has no parent")?,
        false,
    )?;
    let file = open(path, READ_CONTROL | 0x8000_0000, false)?;
    inspect(&file, path, &Policy::machine(), u32::MAX)?;
    Ok((file, guard))
}

pub(crate) fn write_machine_file(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().context("Credential file has no parent")?;
    let _guard = machine_directory(parent, true)?;
    match open_machine_file(path) {
        Ok(_) => {}
        Err(error)
            if error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|error| error.kind() == std::io::ErrorKind::NotFound) => {}
        Err(error) => return Err(error),
    }
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    // Assign the common machine owner before writing, even with a custom token default owner.
    let security = OpenOptions::new()
        .access_mode(READ_CONTROL | WRITE_DAC | WRITE_OWNER)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(temp.path())?;
    Policy::machine()
        .descriptor(false)?
        .apply(&security, true)?;
    drop(security);
    temp.write_all(bytes)?;
    temp.as_file().sync_all()?;
    temp.persist(path).map_err(|error| error.error)?;
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::io::Read;

    pub(crate) fn machine_storage_fixture() -> Result<Option<tempfile::TempDir>> {
        if !scope::elevated()? {
            eprintln!("machine storage tests require an elevated Windows test process");
            return Ok(None);
        }
        // A hosted runner's temporary directory can be owned by its user even
        // when the process is elevated. Use a trusted owner for the test base.
        let dir = tempfile::tempdir()?;
        let file = open(dir.path(), READ_CONTROL | WRITE_DAC | WRITE_OWNER, true)?;
        Policy::machine().descriptor(false)?.apply(&file, true)?;
        Ok(Some(dir))
    }

    fn create(path: &Path, descriptor: &Descriptor) -> Result<()> {
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let attributes = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: descriptor.0,
            bInheritHandle: 0,
        };
        // SAFETY: this creates only a fresh fixture directory with a live descriptor.
        if unsafe { CreateDirectoryW(wide.as_ptr(), &attributes) } == 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        Ok(())
    }

    #[test]
    fn machine_file_round_trip_and_missing_storage() -> Result<()> {
        let Some(dir) = machine_storage_fixture()? else {
            return Ok(());
        };
        let secrets = dir.path().join("SEUCXCS/wlanlogin/secrets");
        let path = secrets.join("login.cred");
        assert!(open_machine_file(&path).is_err());
        assert!(
            !secrets.exists(),
            "inspection must not create missing storage"
        );
        write_machine_file(&path, b"first")?;
        write_machine_file(&path, b"replacement")?;
        let (mut file, _guard) = open_machine_file(&path)?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        assert_eq!(bytes, b"replacement");
        assert!(
            std::fs::rename(&secrets, secrets.with_extension("moved")).is_err(),
            "a validated directory must remain pinned during the read"
        );
        Ok(())
    }

    #[test]
    fn attacker_owned_directories_and_ancestors_are_rejected_before_publication() -> Result<()> {
        for untrusted in 0..3 {
            let Some(dir) = machine_storage_fixture()? else {
                return Ok(());
            };
            let user = scope::current_sid()?;
            let attacker = Descriptor::new(&format!(
                "O:{user}D:P(A;OICI;FA;;;{user})(A;OICI;FA;;;BA)(A;OICI;FA;;;SY)"
            ))?;
            let mut path = dir.path().to_owned();
            for (index, name) in ["SEUCXCS", "wlanlogin", "secrets"].iter().enumerate() {
                path.push(name);
                if index == untrusted {
                    create(&path, &attacker)?;
                    break;
                }
                create(&path, &Policy::machine().descriptor(false)?)?;
            }
            let secret = dir.path().join("SEUCXCS/wlanlogin/secrets/login.cred");
            let error = write_machine_file(&secret, b"must not be published")
                .err()
                .context("attacker-owned directory was accepted")?;
            assert!(error.to_string().contains("owner"));
            assert!(!secret.exists());
            // The attacker still owns this object: rejection must not mutate its DACL.
            let file = open(&path, READ_CONTROL | FILE_READ_ATTRIBUTES, true)?;
            let user_policy = Policy {
                owner: user.clone(),
                trusted: vec![user, ADMINISTRATORS.into(), SYSTEM.into()],
            };
            inspect(&file, &path, &user_policy, MUTATION)?;
        }
        Ok(())
    }

    #[test]
    fn broadly_readable_existing_credentials_are_rejected() -> Result<()> {
        let Some(dir) = machine_storage_fixture()? else {
            return Ok(());
        };
        let path = dir.path().join("SEUCXCS/wlanlogin/secrets/login.cred");
        write_machine_file(&path, b"protected bytes")?;
        let file = open(&path, READ_CONTROL | WRITE_DAC, false)?;
        Descriptor::new("D:P(A;;FA;;;SY)(A;;FA;;;BA)(A;;GR;;;WD)")?.apply(&file, false)?;
        drop(file);
        assert!(open_machine_file(&path).is_err());
        assert!(write_machine_file(&path, b"must not replace unsafe storage").is_err());
        assert_eq!(std::fs::read(path)?, b"protected bytes");
        Ok(())
    }

    #[tokio::test]
    async fn changed_permissions_invalidate_a_cached_machine_credential_read() -> Result<()> {
        let Some(dir) = machine_storage_fixture()? else {
            return Ok(());
        };
        let mut paths =
            crate::infra::paths::Paths::test(dir.path().join("SEUCXCS/wlanlogin/secrets"));
        paths.scope = crate::infra::scope::Scope::System;
        let store =
            crate::system::windows::credentials::machine::MachineStore::new(paths.clone(), None)?;
        store
            .save(wlanlogin_lib::Credentials {
                username: "test-student".into(),
                password: "test-secret".into(),
            })
            .await?;
        assert!(store.load(true).await?.is_some());
        let path = paths.secrets.join("login.cred");
        let before = std::fs::metadata(&path)?;
        let file = open(&path, READ_CONTROL | WRITE_DAC, false)?;
        Descriptor::new("D:P(A;;FA;;;SY)(A;;FA;;;BA)(A;;GR;;;WD)")?.apply(&file, false)?;
        drop(file);
        let after = std::fs::metadata(&path)?;
        assert_eq!(before.len(), after.len());
        assert_eq!(before.modified()?, after.modified()?);
        assert!(store.load(true).await.is_err());
        Ok(())
    }

    #[test]
    fn ordinary_process_storage_remains_writable() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("SEUCXCS/wlanlogin/config/preferences.json");
        super::super::write_private(&path, b"preferences")?;
        assert_eq!(std::fs::read(path)?, b"preferences");
        Ok(())
    }

    #[test]
    fn redirected_ancestors_are_rejected_without_touching_the_target() -> Result<()> {
        let Some(dir) = machine_storage_fixture()? else {
            return Ok(());
        };
        let outside = dir.path().join("outside");
        create(&outside, &Policy::machine().descriptor(false)?)?;
        let link = dir.path().join("SEUCXCS");
        std::os::windows::fs::symlink_dir(&outside, &link)?;
        let path = link.join("wlanlogin/secrets/login.cred");
        assert!(write_machine_file(&path, b"must not follow the link").is_err());
        assert!(!outside.join("wlanlogin").exists());
        Ok(())
    }
}
