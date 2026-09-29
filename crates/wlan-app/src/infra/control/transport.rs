//! Local IPC restricted to the selected user or system scope. No TCP port is opened.
#[cfg(unix)]
use crate::infra::error::Message;
use crate::infra::error::Result;
use crate::infra::paths::Paths;

#[cfg(unix)]
pub type Stream = tokio::net::UnixStream;
#[cfg(unix)]
pub struct Listener {
    inner: tokio::net::UnixListener,
}
#[cfg(unix)]
impl Listener {
    pub fn bind(paths: &Paths, _nonce: &str) -> Result<(Self, String)> {
        use std::os::unix::fs::{FileTypeExt, PermissionsExt};
        let path = paths.runtime.join("daemon.sock");
        match std::fs::symlink_metadata(&path) {
            Ok(meta) if meta.file_type().is_socket() => std::fs::remove_file(&path)?,
            Ok(_) => crate::infra::error::bail!(Message::ControlSocketPathIsOccupiedByAnotherFile),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
        let inner = tokio::net::UnixListener::bind(&path)?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        Ok((Self { inner }, path.to_string_lossy().into_owned()))
    }
    pub async fn accept(&mut self) -> Result<Stream> {
        Ok(self.inner.accept().await?.0)
    }
}
#[cfg(unix)]
pub async fn connect(address: &str) -> Result<Stream> {
    Ok(Stream::connect(address).await?)
}

#[cfg(windows)]
pub type Stream = tokio::net::windows::named_pipe::NamedPipeServer;
#[cfg(windows)]
pub struct Listener {
    inner: Stream,
    address: String,
    scope: crate::infra::scope::Scope,
}
#[cfg(windows)]
impl Listener {
    pub fn bind(paths: &Paths, nonce: &str) -> Result<(Self, String)> {
        let address = format!(r"\\.\pipe\{}-{nonce}", crate::infra::identity::APP_ID);
        let inner = pipe(&address, true, paths.scope)?;
        Ok((
            Self {
                inner,
                address: address.clone(),
                scope: paths.scope,
            },
            address,
        ))
    }
    pub async fn accept(&mut self) -> Result<Stream> {
        self.inner.connect().await?;
        let next = pipe(&self.address, false, self.scope)?;
        Ok(std::mem::replace(&mut self.inner, next))
    }
}
#[cfg(windows)]
pub async fn connect(address: &str) -> Result<tokio::net::windows::named_pipe::NamedPipeClient> {
    Ok(tokio::net::windows::named_pipe::ClientOptions::new().open(address)?)
}

#[cfg(windows)]
fn pipe(address: &str, first: bool, scope: crate::infra::scope::Scope) -> Result<Stream> {
    use std::ptr;
    use windows_sys::Win32::{
        Foundation::LocalFree,
        Security::{
            Authorization::ConvertStringSecurityDescriptorToSecurityDescriptorW,
            SECURITY_ATTRIBUTES,
        },
    };
    // System IPC is shared by LocalSystem and elevated administrators only.
    // User IPC remains restricted to the current user's SID and LocalSystem.
    // PIPE_REJECT_REMOTE_CLIENTS is available on Windows 7.
    let sid = match scope {
        crate::infra::scope::Scope::User => crate::infra::scope::current_sid()?,
        crate::infra::scope::Scope::System => "BA".into(),
    };
    let sddl: Vec<u16> = format!("D:P(A;;GA;;;{sid})(A;;GA;;;SY)")
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let mut descriptor = ptr::null_mut();
    // SAFETY: descriptor is allocated by Windows, used only during CreateNamedPipe,
    // and unconditionally freed after creation. The pipe copies its security data.
    unsafe {
        if ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            1,
            &mut descriptor,
            ptr::null_mut(),
        ) == 0
        {
            return Err(std::io::Error::last_os_error().into());
        }
        let mut attributes = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: descriptor,
            bInheritHandle: 0,
        };
        let result = tokio::net::windows::named_pipe::ServerOptions::new()
            .first_pipe_instance(first)
            .reject_remote_clients(true)
            .create_with_security_attributes_raw(
                address,
                (&mut attributes as *mut SECURITY_ATTRIBUTES).cast(),
            );
        LocalFree(descriptor);
        Ok(result?)
    }
}
