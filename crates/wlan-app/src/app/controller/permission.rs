//! Query every requested capability without losing independent diagnostics.
use crate::infra::{
    error::{Context, Message, Result},
    scope::Scope,
};
use crate::system::define::permission::{
    PermissionId, PermissionProvider, PermissionReport, PermissionRequestOutcome, PermissionResult,
    PermissionStatus,
};

pub async fn report(
    provider: &impl PermissionProvider,
    scope: Scope,
    id: Option<&str>,
    request: bool,
    interactive: bool,
) -> Result<PermissionReport> {
    let ids = match id {
        Some(id) => {
            let id = PermissionId(id.into());
            provider.validate(&id)?;
            vec![id]
        }
        None => provider.permissions().into_iter().map(|p| p.id).collect(),
    };
    let mut permissions = Vec::new();
    for id in ids {
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(if request { 90 } else { 10 }),
            async {
                if request && !interactive {
                    Ok(PermissionStatus::NeedsInteraction)
                } else if request {
                    provider.request(&id).await.map(|outcome| match outcome {
                        PermissionRequestOutcome::Completed(status) => status,
                        PermissionRequestOutcome::NotImplemented => {
                            PermissionStatus::NotImplemented
                        }
                    })
                } else {
                    provider.status(&id).await
                }
            },
        )
        .await
        .context(Message::PermissionTimedOut)
        .and_then(|result| result);
        let (status, error) = match result {
            Ok(status) => (status, None),
            Err(error) => (PermissionStatus::Unavailable, Some(format!("{error:#}"))),
        };
        permissions.push(PermissionResult {
            id: id.0,
            status,
            error,
        });
    }
    Ok(PermissionReport::new(scope.label(), permissions))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::define::permission::{PermissionDescriptor, PermissionRequestOutcome};
    use std::cell::Cell;

    #[derive(Default)]
    struct Provider {
        checks: Cell<usize>,
        requests: Cell<usize>,
    }
    impl PermissionProvider for Provider {
        fn permissions(&self) -> Vec<PermissionDescriptor> {
            vec![
                PermissionDescriptor::new("test.unavailable", "Unavailable"),
                PermissionDescriptor::new("test.granted", "Granted"),
            ]
        }
        async fn status(&self, id: &PermissionId) -> Result<PermissionStatus> {
            self.checks.set(self.checks.get() + 1);
            if id.0 == "test.unavailable" {
                crate::infra::error::bail!("Native provider unavailable");
            }
            Ok(PermissionStatus::Granted)
        }
        async fn request(&self, _: &PermissionId) -> Result<PermissionRequestOutcome> {
            self.requests.set(self.requests.get() + 1);
            Ok(PermissionRequestOutcome::Completed(
                PermissionStatus::Denied,
            ))
        }
    }

    #[tokio::test]
    async fn check_keeps_independent_results_and_never_requests_permission() -> Result<()> {
        let provider = Provider::default();
        let report = report(&provider, Scope::System, None, false, true).await?;
        assert_eq!(provider.requests.get(), 0);
        assert_eq!(provider.checks.get(), 2);
        assert_eq!(report.scope, "system");
        assert_eq!(report.permissions[0].status, PermissionStatus::Unavailable);
        assert!(report.permissions[0].error.is_some());
        assert_eq!(report.permissions[1].status, PermissionStatus::Granted);
        Ok(())
    }

    #[tokio::test]
    async fn request_requires_interaction_and_reports_actual_decision() -> Result<()> {
        let provider = Provider::default();
        let result = report(&provider, Scope::User, Some("test.granted"), true, false).await?;
        assert_eq!(
            result.permissions[0].status,
            PermissionStatus::NeedsInteraction
        );
        assert_eq!(provider.requests.get(), 0);
        let result = report(&provider, Scope::User, Some("test.granted"), true, true).await?;
        assert_eq!(result.permissions[0].status, PermissionStatus::Denied);
        assert_eq!(provider.requests.get(), 1);
        assert!(
            report(&provider, Scope::User, Some("unknown"), true, true)
                .await
                .is_err()
        );
        assert_eq!(provider.requests.get(), 1);
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn stalled_provider_has_a_deadline_and_does_not_hide_other_permissions() -> Result<()> {
        struct Stalled;
        impl PermissionProvider for Stalled {
            fn permissions(&self) -> Vec<PermissionDescriptor> {
                vec![
                    PermissionDescriptor::new("stalled", "Stalled"),
                    PermissionDescriptor::new("ready", "Ready"),
                ]
            }
            async fn status(&self, id: &PermissionId) -> Result<PermissionStatus> {
                if id.0 == "stalled" {
                    std::future::pending::<()>().await;
                }
                Ok(PermissionStatus::Granted)
            }
        }
        let result = report(&Stalled, Scope::User, None, false, false).await?;
        assert_eq!(result.permissions[0].status, PermissionStatus::Unavailable);
        assert!(
            result.permissions[0]
                .error
                .as_ref()
                .is_some_and(|error| error.contains("Timed out"))
        );
        assert_eq!(result.permissions[1].status, PermissionStatus::Granted);
        Ok(())
    }
}
