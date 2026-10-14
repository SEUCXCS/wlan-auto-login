//! Credential failures retain their source while exposing only safe diagnostic fields.
use super::Error;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    #[error("credential store is locked")]
    Locked,
    #[error("credential access requires foreground interaction")]
    NeedsInteraction,
    #[error("credential access was denied")]
    PermissionDenied,
    #[error("credential provider is unavailable")]
    ProviderUnavailable,
    #[error("credential operation timed out")]
    Timeout,
    #[error("saved credentials are not valid UTF-8")]
    InvalidEncoding,
    #[error("invalid saved credential format")]
    InvalidFormat,
    #[error("saved credential format version is unsupported")]
    UnsupportedVersion,
    #[error("saved credentials are incomplete")]
    Incomplete,
    #[error("duplicate saved credential entries")]
    DuplicateEntries,
    #[error("credential decryption failed")]
    DecryptionFailed,
    #[error("credential storage permissions or path are unsafe")]
    UnsafeStorage,
    #[error("background credential task failed")]
    TaskFailed,
    #[error("credential backend is unavailable")]
    BackendUnavailable,
    #[error("native credential operation failed")]
    NativeFailure,
    #[error("unclassified credential operation failed")]
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    Session,
    Connect,
    Search,
    Inspect,
    Read,
    Decode,
    Decrypt,
    Task,
}
impl Stage {
    pub fn label(self) -> &'static str {
        match self {
            Self::Session => "session",
            Self::Connect => "connect",
            Self::Search => "search",
            Self::Inspect => "inspect",
            Self::Read => "read",
            Self::Decode => "decode",
            Self::Decrypt => "decrypt",
            Self::Task => "task",
        }
    }
}

/// Recognized protocol error names only. Provider-supplied free text is never serialized.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BusCode {
    NoReply,
    NoOwner,
    ServiceUnknown,
    AccessDenied,
    Locked,
    Disconnected,
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Diagnostic {
    pub kind: Kind,
    pub stage: Stage,
    pub native_code: Option<i64>,
    pub bus_code: Option<BusCode>,
}
impl Diagnostic {
    pub fn new(kind: Kind, stage: Stage) -> Self {
        Self {
            kind,
            stage,
            native_code: None,
            bus_code: None,
        }
    }
}
impl std::fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} during {}", self.kind, self.stage.label())?;
        if let Some(code) = self.native_code {
            write!(f, " (native code {code})")?;
        }
        if let Some(code) = self.bus_code {
            write!(f, " (D-Bus {code:?})")?;
        }
        Ok(())
    }
}

#[derive(thiserror::Error)]
#[error("{diagnostic}")]
pub struct CredentialError {
    pub diagnostic: Diagnostic,
    #[source]
    source: Option<Box<Error>>,
}
impl CredentialError {
    pub fn new(kind: Kind, stage: Stage) -> Self {
        Self {
            diagnostic: Diagnostic::new(kind, stage),
            source: None,
        }
    }
    pub fn with_source(diagnostic: Diagnostic, source: Error) -> Self {
        Self {
            diagnostic,
            source: Some(Box::new(source)),
        }
    }
    pub(super) fn source_error(&self) -> Option<&Error> {
        self.source.as_deref()
    }
}

impl std::fmt::Debug for CredentialError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CredentialError")
            .field("diagnostic", &self.diagnostic)
            .field("has_source", &self.source.is_some())
            .finish()
    }
}
