use crate::app::{context::Context, daemon};
use crate::infra::error::Result;
#[cfg(not(windows))]
use crate::infra::error::{Message, ensure};
pub async fn run(context: &Context, background: bool, service: bool) -> Result<()> {
    let managed = background && std::env::var_os("WLANLOGIN_MANAGED").is_some_and(|v| v == "1");
    #[cfg(target_os = "macos")]
    let _managed_guard = if managed {
        Some(crate::system::macos::services::runtime::guard(&context.paths).await?)
    } else {
        None
    };
    #[cfg(target_os = "macos")]
    let _host = if managed && context.paths.scope == crate::infra::scope::Scope::User {
        Some(
            crate::system::macos::host::Host::start(crate::system::macos::host::Role::Service)
                .await?,
        )
    } else {
        None
    };
    #[cfg(not(windows))]
    {
        let _ = background;
        ensure!(!service, Message::ServiceIsOnlySupportedByWindowsService);
    }
    #[cfg(windows)]
    {
        let _ = service;
        if background {
            crate::system::windows::services::runtime::detach_console();
        }
    }
    daemon::run(&context.client, &context.paths, managed).await
}
