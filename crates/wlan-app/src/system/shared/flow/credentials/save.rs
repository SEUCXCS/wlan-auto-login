//! Invalidate cached reads before saving credentials in the selected scope.
use super::Step;
use crate::infra::{control::State, error::Result, paths::Paths};
use crate::system::{
    define::credentials::Store,
    flow::{NativeCredentials, hooks::step},
};

pub(crate) async fn run(
    paths: &Paths,
    store: &impl Store,
    credentials: wlanlogin_lib::Credentials,
) -> Result<()> {
    // Caller holds the operation lock across both steps.
    step(&NativeCredentials, Step::Invalidate, paths, async {
        State::invalidate_credentials(paths)
    })
    .await?;
    step(
        &NativeCredentials,
        Step::Save,
        paths,
        store.save(credentials),
    )
    .await
}
