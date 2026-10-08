//! Shared service transactions and dispatch to native managers.
use crate::infra::{command::Runner, error::Result};
pub(crate) use crate::system::define::services::{
    Action, Manager, Plan, ServiceManager, ServiceState,
};
use std::collections::HashMap;
pub(crate) mod artifacts;
pub(crate) mod install;
pub(crate) mod lifecycle;
pub(crate) mod uninstall;
pub(crate) use artifacts::ensure_no_conflicting_install;
pub(crate) use install::install;
pub(crate) use lifecycle::manage;
pub(crate) use uninstall::uninstall;
impl ServiceManager for Manager {
    fn label(&self) -> &'static str {
        match self {
            #[cfg(any(target_os = "linux", test))]
            Self::Systemd { system } => {
                if *system {
                    "systemd"
                } else {
                    "systemd --user"
                }
            }
            #[cfg(any(target_os = "macos", test))]
            Self::Launchd { domain, .. } if domain == "system" => "launchd LaunchDaemon",
            #[cfg(any(target_os = "macos", test))]
            Self::Launchd { .. } => "launchd LaunchAgent",
            #[cfg(any(target_os = "macos", test))]
            Self::MacosAppService { domain, .. } if domain == "system" => {
                "SMAppService LaunchDaemon"
            }
            #[cfg(any(target_os = "macos", test))]
            Self::MacosAppService { .. } => "SMAppService LaunchAgent",
            #[cfg(any(target_os = "windows", test))]
            Self::TaskScheduler { .. } => "Windows Task Scheduler",
            #[cfg(any(windows, test))]
            Self::WindowsService => "Windows Service Control Manager",
        }
    }

    fn name(&self) -> &str {
        match self {
            #[cfg(any(target_os = "linux", test))]
            Self::Systemd { .. } => crate::infra::identity::SYSTEMD_UNIT,
            #[cfg(any(target_os = "macos", test))]
            Self::Launchd { target, .. } => target,
            #[cfg(any(target_os = "macos", test))]
            Self::MacosAppService { target, .. } => target,
            #[cfg(any(target_os = "windows", test))]
            Self::TaskScheduler { name } => name,
            #[cfg(any(windows, test))]
            Self::WindowsService => crate::infra::identity::APP_ID,
        }
    }

    fn installed(&self, state: &ServiceState) -> bool {
        match self {
            // bootout stops this login session; the installed plist loads next login.
            #[cfg(any(target_os = "macos", test))]
            Self::Launchd { .. } => {
                let _ = state;
                true
            }
            #[cfg(any(target_os = "macos", test))]
            Self::MacosAppService { .. } => state.registered,
            #[cfg(any(target_os = "linux", test))]
            Self::Systemd { .. } => state.registered,
            #[cfg(any(target_os = "windows", test))]
            Self::TaskScheduler { .. } => state.registered,
            #[cfg(any(windows, test))]
            Self::WindowsService => state.registered,
        }
    }

    async fn query(&self, runner: &impl Runner) -> Result<ServiceState> {
        match self {
            #[cfg(any(target_os = "linux", test))]
            Self::Systemd { system } => {
                crate::system::linux::services::manager::query(*system, self.name(), runner).await
            }
            #[cfg(any(target_os = "macos", test))]
            Self::Launchd { domain, target } => {
                crate::system::macos::services::manager::query(domain, target, runner).await
            }
            #[cfg(any(target_os = "macos", test))]
            Self::MacosAppService {
                domain,
                target,
                executable,
            } => {
                crate::system::macos::services::app_service::query(
                    domain, target, executable, runner,
                )
                .await
            }
            #[cfg(any(target_os = "windows", test))]
            Self::TaskScheduler { name } => {
                crate::system::windows::services::manager::query_task(name, runner).await
            }
            #[cfg(any(target_os = "windows", test))]
            Self::WindowsService => {
                crate::system::windows::services::manager::query_service(runner).await
            }
        }
    }
    async fn manage(
        &self,
        plan: &Plan,
        action: Action,
        state: ServiceState,
        runner: &impl Runner,
    ) -> Result<()> {
        match self {
            #[cfg(any(target_os = "linux", test))]
            Self::Systemd { system } => {
                let _ = state;
                crate::system::linux::services::manager::manage(*system, plan, action, runner).await
            }
            #[cfg(any(target_os = "macos", test))]
            Self::Launchd { domain, target } => {
                crate::system::macos::services::manager::manage(
                    domain, target, plan, action, state, runner,
                )
                .await
            }
            #[cfg(any(target_os = "macos", test))]
            Self::MacosAppService {
                domain,
                target,
                executable,
            } => {
                let _ = state;
                crate::system::macos::services::app_service::manage(
                    domain, target, executable, plan, action, runner,
                )
                .await
            }
            #[cfg(any(target_os = "windows", test))]
            Self::TaskScheduler { name } => {
                crate::system::windows::services::manager::manage_task(
                    name, plan, action, state, runner,
                )
                .await
            }
            #[cfg(any(target_os = "windows", test))]
            Self::WindowsService => {
                crate::system::windows::services::manager::manage_service(
                    plan, action, state, runner,
                )
                .await
            }
        }
    }
}
pub(crate) fn fields(text: &str) -> HashMap<&str, &str> {
    text.lines()
        .filter_map(|line| line.split_once('='))
        .map(|(key, value)| (key.trim(), value.trim()))
        .collect()
}
