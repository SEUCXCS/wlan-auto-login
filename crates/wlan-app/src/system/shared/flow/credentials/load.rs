use crate::infra::error::Result;
use crate::system::define::credentials::Store;

pub(crate) async fn run(store: &impl Store) -> Result<Option<wlanlogin_lib::Credentials>> {
    // Delegate storage policy to the selected scope's adapter.
    store.load().await
}
