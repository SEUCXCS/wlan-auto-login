//! Enter managed hosting before the application publishes its daemon endpoint.
use crate::infra::{error::Result, paths::Paths, scope::Scope};
use crate::system::macos::runtime::{host, service};

pub(crate) async fn run(
    paths: &Paths,
    managed: bool,
    operation: impl std::future::Future<Output = Result<()>>,
) -> Result<()> {
    // A delayed launch after bootout must fail before creating a helper or IPC.
    let _guard = if managed {
        Some(service::guard(paths).await?)
    } else {
        None
    };

    // The signed Wi-Fi helper belongs to the desktop user, including system clients.
    let _host = if managed && paths.scope == Scope::User {
        Some(host::Host::start(host::Role::Service).await?)
    } else {
        None
    };

    operation.await
}
