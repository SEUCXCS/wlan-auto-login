//! Service contracts, installation descriptions and normalized manager state.
pub use crate::infra::identity::APP_ID as LABEL;
use crate::infra::scope::Scope;
use crate::infra::{
    command::{Invocation, Runner},
    error::Result,
};
use std::{future::Future, path::PathBuf};
#[derive(Clone, Debug)]
pub(crate) enum Manager {
    #[cfg(any(target_os = "linux", test))]
    Systemd { system: bool },
    #[cfg(any(target_os = "macos", test))]
    MacosAppService {
        domain: String,
        target: String,
        executable: PathBuf,
    },
    #[cfg(any(target_os = "windows", test))]
    TaskScheduler { name: String },
    #[cfg(any(windows, test))]
    WindowsService,
}

#[derive(Debug)]
pub(crate) struct ServiceState {
    pub registered: bool,
    pub running: bool,
    pub pending: bool,
    pub enabled: Option<bool>,
    pub state: String,
    pub pid: Option<u32>,
    pub detail: Option<String>,
}

impl ServiceState {
    pub(crate) fn absent() -> Self {
        Self {
            registered: false,
            running: false,
            pending: false,
            enabled: None,
            state: "unregistered/unloaded".into(),
            pid: None,
            detail: None,
        }
    }
}

pub(crate) struct Plan {
    pub(crate) scope: crate::infra::scope::Scope,
    pub(crate) manager: Manager,
    pub(crate) executable: PathBuf,
    pub(crate) definition: PathBuf,
    pub(crate) content: String,
    pub(crate) stop: Vec<Invocation>,
    pub(crate) start: Vec<Invocation>,
    pub(crate) remove: Vec<Invocation>,
    pub(crate) reload: Vec<Invocation>,
}

#[derive(Clone, Copy, Debug)]
pub enum Action {
    Start,
    Stop,
    Restart,
}

pub(crate) trait ServiceManager {
    fn label(&self) -> &'static str;
    fn name(&self) -> &str;
    fn installed(&self, state: &ServiceState) -> bool;
    fn query(&self, runner: &impl Runner) -> impl Future<Output = Result<ServiceState>>;
    fn manage(
        &self,
        plan: &Plan,
        action: Action,
        state: ServiceState,
        runner: &impl Runner,
    ) -> impl Future<Output = Result<()>>;
}

#[derive(Debug)]
pub enum LogSource {
    File(PathBuf),
}

pub(crate) use crate::infra::file_log::LogOptions;

pub struct InstallationReport {
    pub scope: Scope,
    pub manager_label: &'static str,
    pub service_name: String,
    pub executable: PathBuf,
    pub definition: PathBuf,
    pub files: Artifacts,
    pub registration_missing: bool,
    pub state: Option<Result<ServiceState>>,
    pub version: Option<Result<String>>,
    pub runtime: Option<Result<crate::infra::control::RuntimeStatus>>,
}

pub(crate) struct Artifacts {
    pub definition: bool,
    pub executable: bool,
}
impl Artifacts {
    pub fn complete(&self) -> bool {
        self.definition && self.executable
    }

    pub(crate) fn any(&self) -> bool {
        self.definition || self.executable
    }
}

pub(crate) trait LogBackend {
    fn source(
        &self,
        paths: &crate::infra::paths::Paths,
        plan: &Plan,
        options: LogOptions,
    ) -> Result<LogSource>;
    fn read(
        &self,
        source: LogSource,
        options: LogOptions,
        output: &mut impl std::io::Write,
        stop: impl Future<Output = std::io::Result<()>>,
    ) -> impl Future<Output = Result<()>>;
}
