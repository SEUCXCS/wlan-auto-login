use crate::app::{context::Context, daemon};
use crate::infra::error::Result;
#[cfg(not(windows))]
use crate::infra::error::{Message, ensure};
pub async fn run(context: &Context, background: bool, service: bool) -> Result<()> {
    #[cfg(target_os = "macos")]
    let _host = if background && context.paths.scope == crate::infra::scope::Scope::User {
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
    daemon::run(&context.client, &context.paths).await
}
