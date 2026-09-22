use anyhow::{Context, Result};
use cxxy_wlanlogin_lib::Line;
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Clone)]
pub struct Paths {
    pub scope: crate::scope::Scope,
    pub config: PathBuf,
    pub runtime: PathBuf,
    pub secrets: PathBuf,
}

impl Paths {
    pub fn new(scope: crate::scope::Scope) -> Result<Self> {
        use crate::scope::Scope;
        if scope == Scope::System {
            #[cfg(target_os = "linux")]
            return Ok(Self {
                scope,
                config: "/var/lib/cxxy-wlanlogin".into(),
                runtime: "/run/cxxy-wlanlogin".into(),
                secrets: "/etc/cxxy-wlanlogin".into(),
            });
            #[cfg(target_os = "macos")]
            return Ok(Self {
                scope,
                config: "/Library/Application Support/cxxy-wlanlogin/state".into(),
                runtime: "/var/run/cxxy-wlanlogin".into(),
                secrets: "/Library/Application Support/cxxy-wlanlogin/secrets".into(),
            });
            #[cfg(windows)]
            {
                let base = known_folder(&windows_sys::Win32::UI::Shell::FOLDERID_ProgramData)?
                    .join("cxxy-wlanlogin");
                return Ok(Self {
                    scope,
                    config: base.join("state"),
                    runtime: base.join("run"),
                    secrets: base.join("secrets"),
                });
            }
        }
        let dirs = ProjectDirs::from("org", "seucxcs", "cxxy-wlanlogin")
            .context("无法确定用户配置目录")?;
        Ok(Self {
            scope,
            config: dirs.config_dir().into(),
            runtime: dirs.config_dir().into(),
            secrets: dirs.config_dir().into(),
        })
    }

    #[cfg(test)]
    pub fn test(config: impl Into<PathBuf>) -> Self {
        let config = config.into();
        Self {
            scope: Default::default(),
            runtime: config.clone(),
            secrets: config.clone(),
            config,
        }
    }
}

#[cfg(windows)]
pub fn known_folder(id: &windows_sys::core::GUID) -> Result<PathBuf> {
    use std::os::windows::ffi::OsStringExt;
    // SAFETY: the shell allocates a NUL-terminated UTF-16 string, released with CoTaskMemFree.
    unsafe {
        let mut value = std::ptr::null_mut();
        let result = windows_sys::Win32::UI::Shell::SHGetKnownFolderPath(
            id,
            0,
            std::ptr::null_mut(),
            &mut value,
        );
        anyhow::ensure!(result >= 0, "无法读取系统目录: {result:#x}");
        let mut length = 0;
        while *value.add(length) != 0 {
            length += 1;
        }
        let path = std::ffi::OsString::from_wide(std::slice::from_raw_parts(value, length));
        windows_sys::Win32::System::Com::CoTaskMemFree(value.cast());
        Ok(path.into())
    }
}

#[derive(Default, Serialize, Deserialize)]
pub struct Preferences {
    pub line: Line,
}

impl Preferences {
    pub fn load(paths: &Paths) -> Result<Self> {
        match fs::read(paths.config.join("preferences.json")) {
            Ok(bytes) => serde_json::from_slice(&bytes).context("线路配置损坏"),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e).context("无法读取线路配置"),
        }
    }

    pub fn save(&self, paths: &Paths) -> Result<()> {
        write_private(
            &paths.config.join("preferences.json"),
            &serde_json::to_vec_pretty(self)?,
        )
    }
}

pub fn private_dir(path: &Path) -> Result<()> {
    fs::create_dir_all(path)?;
    let metadata = fs::symlink_metadata(path)?;
    anyhow::ensure!(
        metadata.is_dir() && !metadata.file_type().is_symlink(),
        "配置目录不能是符号链接：{}",
        path.display()
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        anyhow::ensure!(
            metadata.uid() == unsafe { libc::geteuid() },
            "配置目录不属于当前运行身份：{}",
            path.display()
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
    let identity = if crate::scope::elevated()? {
        "BA".into()
    } else {
        crate::scope::current_sid()?
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
    let parent = path.parent().context("文件没有父目录")?;
    private_dir(parent)?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    temp.write_all(bytes)?;
    temp.as_file().sync_all()?;
    temp.persist(path).map_err(|e| e.error)?;
    Ok(())
}
