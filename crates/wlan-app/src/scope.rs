use anyhow::Result;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Scope {
    #[default]
    User,
    System,
}

impl Scope {
    pub fn select(user: bool, system: bool, elevated: bool) -> Self {
        if system || (!user && elevated) {
            Self::System
        } else {
            Self::User
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::User => "用户",
            Self::System => "系统",
        }
    }

    pub fn require_access(self) -> Result<()> {
        let elevated = elevated()?;
        if self == Self::System && !elevated {
            anyhow::bail!("管理系统安装和凭据需要管理员权限，请使用 sudo 或管理员终端");
        }
        if self == Self::User && elevated {
            anyhow::bail!("请使用普通用户终端运行 --user 命令，以访问该用户的服务和密钥库");
        }
        Ok(())
    }
}

pub fn elevated() -> Result<bool> {
    #[cfg(unix)]
    {
        // SAFETY: geteuid has no preconditions and does not change process identity.
        Ok(unsafe { libc::geteuid() } == 0)
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::{
            Foundation::CloseHandle,
            Security::{GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation},
            System::Threading::{GetCurrentProcess, OpenProcessToken},
        };
        // SAFETY: token is closed on every path and the output buffer has the API's exact layout.
        unsafe {
            let mut token = std::ptr::null_mut();
            if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
                return Err(std::io::Error::last_os_error().into());
            }
            let mut elevation: TOKEN_ELEVATION = std::mem::zeroed();
            let mut length = 0;
            let result = GetTokenInformation(
                token,
                TokenElevation,
                (&mut elevation as *mut TOKEN_ELEVATION).cast(),
                std::mem::size_of_val(&elevation) as u32,
                &mut length,
            );
            let error = std::io::Error::last_os_error();
            CloseHandle(token);
            if result == 0 {
                return Err(error.into());
            }
            Ok(elevation.TokenIsElevated != 0)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn actual_elevation_selects_default_scope_and_flags_override_it() {
        assert_eq!(Scope::select(false, false, false), Scope::User);
        assert_eq!(Scope::select(false, false, true), Scope::System);
        assert_eq!(Scope::select(true, false, true), Scope::User);
        assert_eq!(Scope::select(false, true, false), Scope::System);
    }
}
