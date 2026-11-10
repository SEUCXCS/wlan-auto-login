//! Classify typed native causes without formatting provider messages or secret payloads.

use crate::infra::error::{
    Error,
    credentials::{CredentialError, Diagnostic, Kind, Stage},
};
use crate::system::define::credentials::{BackgroundRead, WaitReason};

pub(crate) fn at(error: Error, stage: Stage) -> Error {
    // Keep the original stage of codec or backend errors across task/context wrappers.
    if error.downcast_ref::<CredentialError>().is_some() {
        return error;
    }
    CredentialError::with_source(diagnostic(&error, stage), error).into()
}

pub(crate) fn background(error: &Error) -> BackgroundRead {
    let diagnostic = diagnostic(error, Stage::Read);
    match diagnostic.kind {
        Kind::Locked => BackgroundRead::Waiting(WaitReason::Locked),
        Kind::NeedsInteraction => BackgroundRead::Waiting(WaitReason::NeedsInteraction),
        Kind::PermissionDenied => BackgroundRead::Waiting(WaitReason::PermissionDenied),
        Kind::ProviderUnavailable => BackgroundRead::Waiting(WaitReason::Provider),
        _ => BackgroundRead::Failed(diagnostic),
    }
}

pub(crate) fn diagnostic(error: &Error, stage: Stage) -> Diagnostic {
    if let Some(typed) = error.downcast_ref::<CredentialError>() {
        return typed.diagnostic;
    }
    if error
        .downcast_ref::<tokio::time::error::Elapsed>()
        .is_some()
    {
        return Diagnostic::new(Kind::Timeout, stage);
    }
    if error.downcast_ref::<tokio::task::JoinError>().is_some() {
        return Diagnostic::new(Kind::TaskFailed, Stage::Task);
    }
    if error.downcast_ref::<std::str::Utf8Error>().is_some() {
        return Diagnostic::new(Kind::InvalidEncoding, Stage::Decode);
    }
    if error.downcast_ref::<serde_json::Error>().is_some() {
        return Diagnostic::new(Kind::InvalidFormat, Stage::Decode);
    }

    if let Some(native) = error.downcast_ref::<keyring::Error>() {
        match native {
            keyring::Error::BadEncoding(_) => {
                return Diagnostic::new(Kind::InvalidEncoding, Stage::Decode);
            }
            keyring::Error::Ambiguous(_) => {
                return Diagnostic::new(Kind::DuplicateEntries, Stage::Search);
            }
            keyring::Error::Invalid(_, _) | keyring::Error::TooLong(_, _) => {
                return Diagnostic::new(Kind::InvalidFormat, Stage::Inspect);
            }
            _ => {}
        }
    }
    // Native causes can be wrapped in keyring/SDK errors; walk their typed source chain.
    let mut cause: Option<&(dyn std::error::Error + 'static)> = Some(error);
    while let Some(source) = cause {
        if let Some(io) = source.downcast_ref::<std::io::Error>() {
            let kind = match io.kind() {
                std::io::ErrorKind::PermissionDenied => Kind::PermissionDenied,
                std::io::ErrorKind::TimedOut => Kind::Timeout,
                std::io::ErrorKind::NotFound
                | std::io::ErrorKind::ConnectionRefused
                | std::io::ErrorKind::BrokenPipe => Kind::BackendUnavailable,
                _ => Kind::NativeFailure,
            };
            let mut diagnostic = Diagnostic::new(kind, stage);
            diagnostic.native_code = io.raw_os_error().map(i64::from);
            return diagnostic;
        }

        if let Some(native) = source.downcast_ref::<keyring::windows::Error>() {
            let kind = match native.0 {
                5 => Kind::PermissionDenied,
                1312 => Kind::NeedsInteraction,
                _ => Kind::NativeFailure,
            };
            let mut diagnostic = Diagnostic::new(kind, stage);
            diagnostic.native_code = Some(i64::from(native.0));
            return diagnostic;
        }

        cause = source.source();
    }
    // An unidentified native failure is not evidence of a permission problem.
    Diagnostic::new(Kind::Unknown, stage)
}

/// A failed decoder/decryptor has a specific cause, while deadlines/denials retain theirs.
pub(crate) fn specific(error: Error, kind: Kind, stage: Stage) -> Error {
    let mut detail = diagnostic(&error, stage);
    if !matches!(
        detail.kind,
        Kind::Timeout | Kind::PermissionDenied | Kind::TaskFailed
    ) {
        detail.kind = kind;
    }
    CredentialError::with_source(detail, error).into()
}
