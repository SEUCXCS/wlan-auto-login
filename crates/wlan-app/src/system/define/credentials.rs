//! Credential storage contract shared by controllers and native backends.
use crate::infra::error::Result;
use wlanlogin_lib::Credentials;
#[derive(Clone, Copy, Debug)]
pub enum CredentialMode {
    Native,
    ProtectedFile,
}
pub trait Store {
    fn load(&self) -> impl std::future::Future<Output = Result<Option<Credentials>>>;
    fn save(&self, credentials: Credentials) -> impl std::future::Future<Output = Result<()>>;
}

/// Normal background states are independent of portal/network failure backoff.
/// Credentials deliberately have no Debug or Serialize representation here.
pub enum BackgroundRead {
    Available(BackgroundCredentials),
    Missing,
    Waiting(WaitReason),
    Failed(crate::infra::error::credentials::Diagnostic),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WaitReason {
    Session,
    CredentialsChanged,
    Provider,
    Locked,
    PermissionDenied,
    NeedsInteraction,
}

/// Local state only: never serialize or print credential values or their permits.
pub struct BackgroundCredentials {
    pub credentials: Credentials,
    pub permit: CredentialPermit,
}

#[derive(Clone)]
pub struct CredentialPermit {
    pub scope: crate::infra::scope::Scope,
    pub config: std::path::PathBuf,
    pub generation: u64,
    // The shared permit shape is identical; only Linux user storage consumes this field.
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    pub session: Option<SessionPermit>,
}

#[derive(Clone, Default, PartialEq, Eq)]
pub struct SessionState {
    pub hint: Option<String>,
    pub generation: u64,
}
impl SessionState {
    pub fn update(&mut self, hint: Option<String>) -> Result<()> {
        self.generation = self.generation.checked_add(1).ok_or_else(|| {
            crate::infra::error::error!(crate::infra::error::Message::SessionGenerationExhausted)
        })?;
        self.hint = hint;
        Ok(())
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct SessionIdentity {
    pub id: String,
    pub started_at: u64,
}
#[derive(Clone)]
#[cfg_attr(not(target_os = "linux"), allow(dead_code))] // Native session proof is Linux-only.
pub struct SessionPermit {
    pub identity: SessionIdentity,
    pub state: SessionState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Validity {
    Valid,
    SessionExpired,
    CredentialsChanged,
}

impl BackgroundRead {
    pub fn from_saved(saved: Option<Credentials>, permit: CredentialPermit) -> Self {
        match saved {
            Some(credentials) => Self::Available(BackgroundCredentials {
                credentials,
                permit,
            }),
            None => Self::Missing,
        }
    }
}
