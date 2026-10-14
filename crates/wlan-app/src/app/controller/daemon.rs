use crate::app::{context::Context, daemon};
use crate::infra::error::Result;

pub async fn run(context: &Context, managed: bool) -> Result<()> {
    crate::system::flow::run::run(
        &context.paths,
        managed,
        daemon::run(&context.client, &context.paths, managed),
    )
    .await
}
