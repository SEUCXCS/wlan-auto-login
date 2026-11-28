use super::Step;
use crate::infra::error::Result;
use crate::system::define::credentials::Store;
use crate::system::flow::{NativeCredentials, hooks::step};

pub(crate) async fn run(store: &impl Store) -> Result<Option<wlanlogin_lib::Credentials>> {
    // Delegate storage policy to the selected scope's adapter.
    step(&NativeCredentials, Step::Load, store, store.load()).await
}
