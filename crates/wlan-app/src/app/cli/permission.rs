//! Present platform permission diagnostics without constructing a portal client.
use super::PermissionCommand;
use crate::infra::{
    error::{Message, Result, ensure},
    scope::Scope,
};
use crate::system::define::permission::PermissionStatus;
use std::io::IsTerminal;

pub(in crate::app) async fn run(command: PermissionCommand, scope: Scope) -> Result<()> {
    let provider = crate::system::native::permission::Permissions;
    let (id, request, json) = match command {
        PermissionCommand::List { json } => {
            let permissions = crate::app::controller::permission::list(&provider);
            if json {
                println!(
                    "{}",
                    serde_json::to_string(
                        &serde_json::json!({"version": 1, "scope": scope.label(), "permissions": permissions})
                    )?
                );
            } else {
                for permission in permissions {
                    println!("{}\t{}", permission.id.0, permission.name);
                    if let Some(description) = permission.description {
                        println!("  {description}");
                    }
                }
            }
            return Ok(());
        }
        PermissionCommand::Check { id, json } => (id, false, json),
        PermissionCommand::Get { id, json } => (Some(id), true, json),
    };
    let interactive = std::io::stdin().is_terminal() && std::io::stderr().is_terminal();
    let report = if request {
        crate::app::controller::permission::get(
            &provider,
            scope,
            id.as_deref()
                .ok_or_else(|| crate::infra::error::error!(Message::PermissionNotGranted))?,
            interactive,
        )
        .await?
    } else {
        crate::app::controller::permission::check(&provider, scope, id.as_deref()).await?
    };
    if json {
        println!("{}", serde_json::to_string(&report)?);
    } else {
        for permission in &report.permissions {
            println!("{}\t{:?}", permission.id, permission.status);
            if let Some(error) = &permission.error {
                println!("  {error}");
            }
        }
    }
    ensure!(
        !request
            || report
                .permissions
                .iter()
                .all(|p| p.status == PermissionStatus::Granted),
        Message::PermissionNotGranted
    );
    ensure!(
        report.permissions.iter().all(|p| p.error.is_none()),
        Message::PermissionCheckFailed
    );
    Ok(())
}
