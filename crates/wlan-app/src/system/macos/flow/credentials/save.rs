use crate::infra::error::Result;
use crate::system::define::credentials::Store;

pub(crate) async fn run(
    paths: &crate::infra::paths::Paths,
    store: &impl Store,
    credentials: wlanlogin_lib::Credentials,
) -> Result<()> {
    // Invalidate cached results before even a partially successful write. Caller holds the operation lock.
    crate::infra::control::State::invalidate_credentials(paths)?;

    // Delegate storage policy to the selected scope's adapter.
    store.save(credentials).await
}
