use crate::infra::command::Invocation;
// Dispatch native service operations at the selected platform boundary.
use crate::infra::{command::Runner, error::Result};
use crate::system::define::services::{Action, Manager, Plan, ServiceManager, ServiceState};
impl ServiceManager for Manager {
    fn label(&self) -> &'static str {
        match self {
            Self::Systemd { system } => {
                if *system {
                    "systemd"
                } else {
                    "systemd --user"
                }
            }
            Self::MacosAppService { domain, .. } if domain == "system" => {
                "SMAppService LaunchDaemon"
            }
            Self::MacosAppService { .. } => "SMAppService LaunchAgent",
            Self::TaskScheduler { .. } => "Windows Task Scheduler",
            Self::WindowsService => "Windows Service Control Manager",
        }
    }

    fn name(&self) -> &str {
        match self {
            Self::Systemd { .. } => crate::infra::identity::SYSTEMD_UNIT,
            Self::MacosAppService { target, .. } => target,
            Self::TaskScheduler { name } => name,
            Self::WindowsService => crate::infra::identity::APP_ID,
        }
    }

    fn installed(&self, state: &ServiceState) -> bool {
        match self {
            Self::MacosAppService { .. } => state.registered,
            Self::Systemd { .. } => state.registered,
            Self::TaskScheduler { .. } => state.registered,
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
            #[cfg(not(test))]
            _ => crate::infra::error::bail!("Service manager is unavailable on this platform"),
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
            Self::MacosAppService {
                domain,
                target,
                executable,
            } => {
                let _ = (plan, state);
                crate::system::macos::services::app_service::manage(
                    domain, target, executable, action, runner,
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
            #[cfg(not(test))]
            _ => crate::infra::error::bail!("Service manager is unavailable on this platform"),
        }
    }
}

pub(crate) async fn execute_optional(runner: &impl Runner, command: &Invocation) -> Result<()> {
    #[cfg(any(target_os = "macos", test))]
    {
        crate::system::macos::services::app_service::output(command, runner).await?;
    }
    #[cfg(not(any(target_os = "macos", test)))]
    {
        let output = runner.output(command).await?;
        if !command.optional {
            output.check(command)?;
        }
    }
    Ok(())
}
