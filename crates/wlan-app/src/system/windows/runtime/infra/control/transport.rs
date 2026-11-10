//! Local IPC restricted to the selected user or system scope. No TCP port is opened.

use crate::infra::error::Result;
use crate::infra::paths::Paths;

pub type Stream = tokio::net::windows::named_pipe::NamedPipeServer;

pub struct Listener {
    inner: Stream,
    address: String,
    scope: crate::infra::scope::Scope,
}

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

pub async fn connect(address: &str) -> Result<tokio::net::windows::named_pipe::NamedPipeClient> {
    // The enclosing IPC request supplies the deadline. A connected instance may
    // be busy until the server accepts it and creates its next listening instance.
    loop {
        match tokio::net::windows::named_pipe::ClientOptions::new().open(address) {
            Ok(client) => return Ok(client),
            Err(error)
                if error.raw_os_error()
                    == Some(windows_sys::Win32::Foundation::ERROR_PIPE_BUSY as i32) =>
            {
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
            Err(error) => return Err(error.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn busy_pipe_waits_for_an_available_instance() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let paths = Paths::test(dir.path());
        let (mut listener, address) =
            Listener::bind(&paths, &format!("busy-pipe-test-{}", std::process::id()))?;
        let _first = connect(&address).await?;
        let waiting = connect(&address);
        tokio::pin!(waiting);
        assert!(
            tokio::time::timeout(Duration::from_millis(20), &mut waiting)
                .await
                .is_err()
        );
        let _accepted = listener.accept().await?;
        let _second = tokio::time::timeout(Duration::from_secs(1), waiting).await??;
        Ok(())
    }

    #[tokio::test]
    async fn missing_pipe_fails_without_retrying() -> Result<()> {
        let address = format!(
            r"\\.\pipe\wlanlogin-missing-pipe-test-{}",
            std::process::id()
        );
        assert!(
            tokio::time::timeout(Duration::from_millis(200), connect(&address))
                .await?
                .is_err()
        );
        Ok(())
    }
}

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
