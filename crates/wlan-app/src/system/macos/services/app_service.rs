//! Register signed, bundle-contained jobs and keep approval separate from launchd liveness.
use crate::infra::{
    command::{CommandOutput, Invocation, Runner},
    error::{Context, Message, Result, ensure},
    identity::APP_ID,
};
use crate::system::define::services::{Action, Plan, ServiceState};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Registration {
    NotRegistered,
    Enabled,
    RequiresApproval,
    NotFound,
}

#[derive(Serialize, Deserialize)]
struct Response {
    version: u32,
    service: String,
    scope: String,
    status: Registration,
}

pub(crate) fn command(executable: &Path, domain: &str, action: &str) -> Invocation {
    Invocation::new(
        executable,
        [
            if domain == "system" {
                "--system"
            } else {
                "--user"
            },
            "service",
            "native",
            action,
        ],
    )
}

/// Reject unsupported signed definitions before executing any native mutation.
pub(crate) fn require_current(executable: &Path, system: bool) -> Result<()> {
    let definition = super::definition::bundle_definition(executable, system)
        .context(Message::UnsupportedInstalledProtocol)?;
    let content = std::fs::read_to_string(definition)?;
    ensure!(
        content == crate::infra::identity::macos_service_plist(system),
        Message::UnsupportedInstalledProtocol
    );
    Ok(())
}

pub(crate) async fn output(invocation: &Invocation, runner: &impl Runner) -> Result<CommandOutput> {
    if invocation.args.len() == 4
        && invocation.args[1] == "service"
        && invocation.args[2] == "native"
    {
        require_current(
            Path::new(&invocation.program),
            invocation.args[0] == "--system",
        )?;
    }
    let output = runner.output(invocation).await?;
    if !invocation.optional {
        output.check(invocation)?;
    }
    Ok(output)
}

fn decode(text: &str, domain: &str) -> Result<Registration> {
    let response: Response = serde_json::from_str(text).context(Message::MacosServiceProtocol)?;
    ensure!(
        response.version == 1
            && response.service == APP_ID
            && response.scope == if domain == "system" { "system" } else { "user" },
        Message::MacosServiceProtocol
    );
    Ok(response.status)
}

pub(crate) async fn query(
    domain: &str,
    target: &str,
    executable: &Path,
    runner: &impl Runner,
) -> Result<ServiceState> {
    let definition = super::definition::bundle_definition(executable, domain == "system");
    // A missing definition is reported as incomplete without executing a bundle.
    // Existing unsupported definitions are rejected by output().
    let registration = if definition.is_some_and(|p| p.is_file()) {
        let invocation = command(executable, domain, "status");
        let output = output(&invocation, runner).await?;
        output.check(&invocation)?;
        Some(decode(&output.stdout, domain)?)
    } else {
        None
    };
    let live = super::manager::query(domain, target, runner).await?;
    Ok(merge(registration, live))
}

fn merge(registration: Option<Registration>, mut live: ServiceState) -> ServiceState {
    let loaded = live.registered;
    if let Some(registration) = registration {
        match registration {
            Registration::Enabled => {
                live.registered = true;
                live.enabled = Some(true);
                if !loaded && !live.running && !live.pending {
                    live.state = "registered; stopped for this session".into();
                }
            }
            Registration::RequiresApproval => {
                live.registered = true;
                live.enabled = Some(false);
                // Approval is not a pending spawn. Uninstall must not wait for approval.
                if !live.running {
                    live.pending = false;
                    live.state = "requires background activity approval".into();
                }
            }
            Registration::NotFound if !live.registered => {
                live.state = "bundle service definition not found".into();
                live.enabled = Some(false);
            }
            Registration::NotRegistered | Registration::NotFound => {}
        }
    }
    live
}

pub(crate) async fn manage(
    domain: &str,
    target: &str,
    executable: &Path,
    plan: &Plan,
    action: Action,
    runner: &impl Runner,
) -> Result<()> {
    let live = super::manager::query(domain, target, runner).await?;
    if matches!(action, Action::Stop | Action::Restart) {
        super::manager::manage(domain, target, plan, Action::Stop, live, runner).await?;
    } else if live.registered {
        return super::manager::manage(domain, target, plan, action, live, runner).await;
    }
    if !matches!(action, Action::Stop) {
        let invocation = command(executable, domain, "register");
        let output = output(&invocation, runner).await?;
        output.check(&invocation)?;
        ensure!(
            decode(&output.stdout, domain)? == Registration::Enabled,
            Message::MacosBackgroundApprovalRequired
        );
    }
    Ok(())
}

#[cfg(target_os = "macos")]
pub(crate) fn perform(scope: crate::infra::scope::Scope, action: &str) -> Result<String> {
    use crate::infra::{identity::macos_service_path, scope::Scope};
    use block2::RcBlock;
    use objc2::{msg_send, rc::Retained, runtime::AnyObject};
    use objc2_foundation::{NSBundle, NSError, NSString};
    use std::{ptr, sync::mpsc, time::Duration};

    #[link(name = "ServiceManagement", kind = "framework")]
    unsafe extern "C" {}

    objc2::rc::autoreleasepool(|_| {
        let app = crate::system::macos::services::bundle::current()?;
        let installed = crate::infra::paths::macos_bundle(scope)?.canonicalize()?;
        let main = NSBundle::mainBundle();
        ensure!(
            app == installed
                && main
                    .bundleIdentifier()
                    .is_some_and(|id| id.to_string() == APP_ID)
                && Path::new(&main.bundlePath().to_string()).canonicalize()? == app,
            Message::MacosServiceBundleIdentity
        );
        let definition = app.join(macos_service_path(scope == Scope::System));
        ensure!(definition.is_file(), Message::MacosServiceDefinitionMissing);
        let name = NSString::from_str(crate::infra::identity::LAUNCHD_PLIST);
        // SMAppService derives its parent identity from the main bundle of
        // this installed signed process, including when invoked from a terminal.
        let service: Retained<AnyObject> = unsafe {
            if scope == Scope::System {
                msg_send![objc2::class!(SMAppService), daemonServiceWithPlistName: &*name]
            } else {
                msg_send![objc2::class!(SMAppService), agentServiceWithPlistName: &*name]
            }
        };
        let status = || -> Result<Registration> {
            let value: usize = unsafe { msg_send![&*service, status] };
            match value {
                0 => Ok(Registration::NotRegistered),
                1 => Ok(Registration::Enabled),
                2 => Ok(Registration::RequiresApproval),
                3 => Ok(Registration::NotFound),
                _ => Err(Message::MacosServiceProtocol.into()),
            }
        };
        let unregister = || -> Result<()> {
            if matches!(
                status()?,
                Registration::NotRegistered | Registration::NotFound
            ) {
                return Ok(());
            }
            let (sender, receiver) = mpsc::sync_channel(1);
            let completion = RcBlock::new(move |error: *mut NSError| {
                let result =
                    unsafe { error.as_ref() }.map(|error| error.localizedDescription().to_string());
                let _ = sender.send(result);
            });
            // Completion is Apple's guarantee that re-registration is safe.
            unsafe {
                let _: () = msg_send![&*service, unregisterWithCompletionHandler: &*completion];
            }
            let completion_error = receiver
                .recv_timeout(Duration::from_secs(20))
                .context(Message::MacosServiceUnregisterTimedOut)?;
            // bootout can already have removed the live job. Treat an
            // idempotent completion as success only if native registration is gone.
            if status()? == Registration::NotRegistered {
                return Ok(());
            }
            if let Some(error) = completion_error {
                return Err(Message::MacosServiceOperationFailed {
                    action: "unregister".into(),
                    error,
                }
                .into());
            }
            Err(Message::MacosServiceProtocol.into())
        };
        match action {
            "status" => {}
            "unregister" => unregister()?,
            "register" => {
                // Re-bootstrap a job explicitly stopped for the current session.
                unregister()?;
                let mut error: *mut NSError = ptr::null_mut();
                let registered: bool =
                    unsafe { msg_send![&*service, registerAndReturnError: &mut error] };
                if !registered && status()? != Registration::RequiresApproval {
                    let detail = unsafe { error.as_ref() }
                        .map(|error| error.localizedDescription().to_string())
                        .unwrap_or_else(|| "SMAppService returned no error details".into());
                    return Err(Message::MacosServiceOperationFailed {
                        action: "register".into(),
                        error: detail,
                    }
                    .into());
                }
            }
            _ => return Err(Message::MacosServiceProtocol.into()),
        }
        Ok(serde_json::to_string(&Response {
            version: 1,
            service: APP_ID.into(),
            scope: scope.label().into(),
            status: status()?,
        })?)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::command::tests::FakeRunner;

    const ENABLED: &str =
        r#"{"version":1,"service":"dev.cxcs.tools.wlanlogin","scope":"user","status":"enabled"}"#;
    const APPROVAL: &str = r#"{"version":1,"service":"dev.cxcs.tools.wlanlogin","scope":"user","status":"requires_approval"}"#;
    const LOADED: &str = "gui/501/dev.cxcs.tools.wlanlogin = {\nstate = running\npid = 42\n}";
    const DOMAIN: &str = "gui/501 = {\nservices = {\n}\n}";

    fn plan(root: &Path) -> Result<Plan> {
        let plan = super::super::definition::app_service_plan(
            root.join("wlanlogin.app/Contents/MacOS/wlanlogin"),
            false,
            501,
        )?;
        std::fs::create_dir_all(
            plan.definition
                .parent()
                .context("fixture definition parent")?,
        )?;
        std::fs::write(&plan.definition, &plan.content)?;
        Ok(plan)
    }

    #[tokio::test]
    async fn unsupported_bundle_is_rejected_before_native_mutation() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let plan = plan(dir.path())?;
        std::fs::create_dir_all(
            plan.definition
                .parent()
                .context(Message::MacosServiceDefinitionMissing)?,
        )?;
        let invocation = command(&plan.executable, "gui/501", "unregister");
        std::fs::write(
            &plan.definition,
            "<string>daemon</string><string>--background</string>",
        )?;
        let runner = FakeRunner::default();
        assert!(output(&invocation, &runner).await.is_err());
        assert!(runner.calls.borrow().is_empty());
        std::fs::write(&plan.definition, &plan.content)?;
        let runner = FakeRunner::new([(true, ENABLED)]);
        output(&invocation, &runner).await?;
        assert_eq!(
            runner.calls.borrow()[0].args,
            ["--user", "service", "native", "unregister"]
        );
        Ok(())
    }

    #[tokio::test]
    async fn native_approval_is_reported_when_launchd_has_no_job() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let plan = plan(dir.path())?;
        std::fs::create_dir_all(
            plan.definition
                .parent()
                .context(Message::MacosServiceDefinitionMissing)?,
        )?;
        std::fs::write(&plan.definition, &plan.content)?;
        let runner = FakeRunner::new([(true, APPROVAL), (false, "missing"), (true, DOMAIN)]);
        let state = query(
            "gui/501",
            "gui/501/dev.cxcs.tools.wlanlogin",
            &plan.executable,
            &runner,
        )
        .await?;
        assert!(state.registered);
        assert!(!state.running && !state.pending);
        assert_eq!(state.enabled, Some(false));
        assert_eq!(runner.calls.borrow()[0].program, plan.executable);
        Ok(())
    }

    #[tokio::test]
    async fn missing_definition_queries_only_native_liveness() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let plan = plan(dir.path())?;
        std::fs::remove_file(&plan.definition)?;
        let runner = FakeRunner::new([(true, LOADED)]);
        let state = query(
            "gui/501",
            "gui/501/dev.cxcs.tools.wlanlogin",
            &plan.executable,
            &runner,
        )
        .await?;
        assert!(state.running);
        assert!(
            runner
                .calls
                .borrow()
                .iter()
                .all(|c| c.program == "launchctl")
        );
        Ok(())
    }

    #[tokio::test]
    async fn stopping_an_unloaded_native_service_preserves_registration() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let plan = plan(dir.path())?;
        let runner = FakeRunner::new([(false, "missing"), (true, DOMAIN)]);
        manage(
            "gui/501",
            "gui/501/dev.cxcs.tools.wlanlogin",
            &plan.executable,
            &plan,
            Action::Stop,
            &runner,
        )
        .await?;
        assert_eq!(runner.calls.borrow().len(), 2);
        assert!(runner.calls.borrow().iter().all(|c| c.args[0] == "print"));
        Ok(())
    }

    #[tokio::test]
    async fn restart_removes_launchd_job_before_native_registration() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let plan = plan(dir.path())?;
        let runner = FakeRunner::new([(true, LOADED), (true, ""), (true, ENABLED)]);
        manage(
            "gui/501",
            "gui/501/dev.cxcs.tools.wlanlogin",
            &plan.executable,
            &plan,
            Action::Restart,
            &runner,
        )
        .await?;
        let calls = runner.calls.borrow();
        assert_eq!(calls[1].args[0], "bootout");
        assert_eq!(calls[2].program, plan.executable);
        assert_eq!(
            calls[2]
                .args
                .last()
                .context(Message::UnexpectedServiceCommand)?,
            "register"
        );
        assert!(
            calls
                .iter()
                .all(|c| !c.args.iter().any(|a| a == "bootstrap"))
        );
        Ok(())
    }

    #[tokio::test]
    async fn start_never_claims_readiness_when_approval_is_missing() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let plan = plan(dir.path())?;
        let runner = FakeRunner::new([(false, "missing"), (true, DOMAIN), (true, APPROVAL)]);
        assert!(
            manage(
                "gui/501",
                "gui/501/dev.cxcs.tools.wlanlogin",
                &plan.executable,
                &plan,
                Action::Start,
                &runner
            )
            .await
            .is_err()
        );
        Ok(())
    }

    #[test]
    fn approval_is_registered_but_never_reported_as_running_or_spawning() {
        let state = merge(Some(Registration::RequiresApproval), ServiceState::absent());
        assert!(state.registered);
        assert!(!state.running && !state.pending);
        assert_eq!(state.enabled, Some(false));
        let stopped = merge(Some(Registration::Enabled), ServiceState::absent());
        assert!(stopped.registered && !stopped.running);
        assert_eq!(stopped.enabled, Some(true));
        let mut failed = ServiceState::absent();
        failed.registered = true;
        failed.state = "not running".into();
        failed.detail = Some("last exit code 1".into());
        let failed = merge(Some(Registration::Enabled), failed);
        assert_eq!(failed.state, "not running");
        assert_eq!(failed.detail.as_deref(), Some("last exit code 1"));
    }

    #[test]
    fn protocol_rejects_other_scope_identity_version_and_unknown_status() -> Result<()> {
        let response = r#"{"version":1,"service":"dev.cxcs.tools.wlanlogin","scope":"user","status":"enabled"}"#;
        assert_eq!(decode(response, "gui/501")?, Registration::Enabled);
        assert!(decode(response, "system").is_err());
        assert!(
            decode(
                &response.replace("\"version\":1", "\"version\":2"),
                "gui/501"
            )
            .is_err()
        );
        assert!(decode(&response.replace(APP_ID, "unrelated.app"), "gui/501").is_err());
        assert!(decode(&response.replace("enabled", "future_value"), "gui/501").is_err());
        Ok(())
    }
}
