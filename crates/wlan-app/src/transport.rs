//! User-local IPC. No TCP port is opened on any platform.
use crate::config::Paths;
use anyhow::Result;

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
        let path = paths.config.join("daemon.sock");
        match std::fs::symlink_metadata(&path) {
            Ok(meta) if meta.file_type().is_socket() => std::fs::remove_file(&path)?,
            Ok(_) => anyhow::bail!("控制 socket 路径被其他文件占用"),
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
}
#[cfg(windows)]
impl Listener {
    pub fn bind(_paths: &Paths, nonce: &str) -> Result<(Self, String)> {
        let address = format!(r"\\.\pipe\cxxy-wlanlogin-{nonce}");
        let inner = pipe(&address, true)?;
        Ok((
            Self {
                inner,
                address: address.clone(),
            },
            address,
        ))
    }
    pub async fn accept(&mut self) -> Result<Stream> {
        self.inner.connect().await?;
        let next = pipe(&self.address, false)?;
        Ok(std::mem::replace(&mut self.inner, next))
    }
}
#[cfg(windows)]
pub async fn connect(address: &str) -> Result<tokio::net::windows::named_pipe::NamedPipeClient> {
    Ok(tokio::net::windows::named_pipe::ClientOptions::new().open(address)?)
}

#[cfg(windows)]
fn pipe(address: &str, first: bool) -> Result<Stream> {
    use std::ptr;
    use windows_sys::Win32::{
        Foundation::LocalFree,
        Security::{
            Authorization::ConvertStringSecurityDescriptorToSecurityDescriptorW,
            SECURITY_ATTRIBUTES,
        },
    };
    // Protected ACL: only the current user's SID and LocalSystem.
    // PIPE_REJECT_REMOTE_CLIENTS is available on Windows 7.
    let sid = crate::platform::current_sid()?;
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
