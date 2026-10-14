use crate::infra::command::Invocation;
use crate::infra::error::Message;
use crate::infra::error::{Context, Result};
#[cfg(target_os = "macos")]
use crate::infra::paths::Paths;
use crate::system::define::services::{LABEL, Manager, Plan};
#[cfg(target_os = "macos")]
use crate::system::shared::services::artifacts::original_user_home;
use std::path::PathBuf;

pub(crate) fn bundle_definition(executable: &std::path::Path, system: bool) -> Option<PathBuf> {
    let macos = executable.parent()?;
    let contents = macos.parent()?;
    let app = contents.parent()?;
    (macos.file_name()? == "MacOS"
        && contents.file_name()? == "Contents"
        && app.extension()? == "app")
        .then(|| app.join(crate::infra::identity::macos_service_path(system)))
}

pub(crate) fn app_service_plan(executable: PathBuf, system: bool, uid: u32) -> Result<Plan> {
    let definition =
        bundle_definition(&executable, system).context(Message::MacosBundleRequired)?;
    let domain = if system {
        "system".into()
    } else {
        format!("gui/{uid}")
    };
    let target = format!("{domain}/{LABEL}");
    Ok(Plan {
        scope: if system {
            crate::infra::scope::Scope::System
        } else {
            crate::infra::scope::Scope::User
        },
        manager: Manager::MacosAppService {
            domain: domain.clone(),
            target: target.clone(),
            executable: executable.clone(),
        },
        definition,
        content: crate::infra::identity::macos_service_plist(system),
        stop: vec![Invocation::new("launchctl", ["bootout", &target]).optional()],
        start: vec![super::app_service::command(
            &executable,
            &domain,
            "register",
        )],
        remove: vec![super::app_service::command(
            &executable,
            &domain,
            "unregister",
        )],
        reload: vec![],
        executable,
    })
}

#[cfg(target_os = "macos")]
pub(crate) fn native_plan(paths: &Paths) -> Result<Plan> {
    let system = paths.scope == crate::infra::scope::Scope::System;
    app_service_plan(
        crate::infra::paths::macos_bundle(paths.scope)?
            .join("Contents/MacOS")
            .join(crate::infra::identity::APP_NAME),
        system,
        unsafe { libc::geteuid() },
    )
}

#[cfg(target_os = "macos")]
pub(crate) fn plan(paths: &Paths) -> Result<Plan> {
    let plan = native_plan(paths)?;
    if plan.definition.try_exists()? {
        super::app_service::validate_definition(
            &plan.executable,
            paths.scope == crate::infra::scope::Scope::System,
        )?;
    }
    Ok(plan)
}
#[cfg(target_os = "macos")]
pub(crate) fn conflicting_definition(paths: &Paths) -> Result<Option<PathBuf>> {
    use crate::infra::scope::Scope;
    let bundled = match paths.scope {
        Scope::User => crate::infra::paths::macos_bundle(Scope::System)?
            .join(crate::infra::identity::macos_service_path(true)),
        Scope::System => {
            let Some(home) = original_user_home()? else {
                return Ok(None);
            };
            crate::infra::paths::app_dir(home.join("Library/Application Support"))
                .join(crate::infra::identity::APP_BUNDLE)
                .join(crate::infra::identity::macos_service_path(false))
        }
    };
    Ok(Some(bundled))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_services_run_signed_bundle_programs_without_a_launcher() -> Result<()> {
        let exe =
            PathBuf::from("/Users/Test User/Applications/wlanlogin.app/Contents/MacOS/wlanlogin");
        for system in [false, true] {
            let plan = app_service_plan(exe.clone(), system, 501)?;
            assert!(matches!(plan.manager, Manager::MacosAppService { .. }));
            assert!(plan.content.contains("<key>BundleProgram</key>"));
            assert!(!plan.content.contains("/usr/bin/open"));
            assert!(!plan.content.contains("/Users/Test User"));
            assert_eq!(
                plan.definition,
                bundle_definition(&exe, system).context(Message::MacosBundleRequired)?
            );
            assert!(
                plan.start
                    .iter()
                    .all(|c| c.program == exe && c.args.last().is_some_and(|a| a == "register"))
            );
            assert!(
                plan.remove
                    .iter()
                    .all(|c| c.program == exe && c.args.last().is_some_and(|a| a == "unregister"))
            );
        }
        assert!(app_service_plan("/tmp/loose-binary".into(), false, 501).is_err());
        Ok(())
    }
}
