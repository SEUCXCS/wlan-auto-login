//! Classify typed native causes without formatting provider messages or secret payloads.
#[cfg(target_os = "linux")]
use crate::infra::error::credentials::BusCode;
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
        #[cfg(target_os = "linux")]
        if let Some(secret) = source.downcast_ref::<dbus_secret_service::Error>() {
            let kind = match secret {
                dbus_secret_service::Error::Locked => Some(Kind::Locked),
                dbus_secret_service::Error::Prompt => Some(Kind::NeedsInteraction),
                dbus_secret_service::Error::Unavailable => Some(Kind::ProviderUnavailable),
                dbus_secret_service::Error::Crypto(_) => Some(Kind::DecryptionFailed),
                dbus_secret_service::Error::Dbus(bus) => return bus_diagnostic(bus.name(), stage),
                _ => Some(Kind::NativeFailure),
            };
            if let Some(kind) = kind {
                return Diagnostic::new(kind, stage);
            }
        }
        #[cfg(target_os = "linux")]
        if let Some(zbus::Error::MethodError(name, _, _)) = source.downcast_ref::<zbus::Error>() {
            return bus_diagnostic(Some(name.as_str()), stage);
        }
        #[cfg(target_os = "windows")]
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
        #[cfg(target_os = "macos")]
        if let Some(native) = source.downcast_ref::<security_framework::base::Error>() {
            let kind = match native.code() {
                -25308 | -25315 => Kind::NeedsInteraction,
                -25293 | -128 => Kind::PermissionDenied,
                -25291 | -25294 => Kind::BackendUnavailable,
                _ => Kind::NativeFailure,
            };
            let mut diagnostic = Diagnostic::new(kind, stage);
            diagnostic.native_code = Some(i64::from(native.code()));
            return diagnostic;
        }
        cause = source.source();
    }
    // An unidentified native failure is not evidence of a permission problem.
    Diagnostic::new(Kind::Unknown, stage)
}

#[cfg(target_os = "linux")]
fn bus_diagnostic(name: Option<&str>, stage: Stage) -> Diagnostic {
    let (kind, code) = match name {
        Some("org.freedesktop.Secret.Error.IsLocked") => (Kind::Locked, BusCode::Locked),
        Some("org.freedesktop.DBus.Error.AccessDenied") => {
            (Kind::PermissionDenied, BusCode::AccessDenied)
        }
        Some(
            "org.freedesktop.DBus.Error.NoReply"
            | "org.freedesktop.DBus.Error.Timeout"
            | "org.freedesktop.DBus.Error.TimedOut",
        ) => (Kind::Timeout, BusCode::NoReply),
        Some("org.freedesktop.DBus.Error.NameHasNoOwner") => {
            (Kind::ProviderUnavailable, BusCode::NoOwner)
        }
        Some("org.freedesktop.DBus.Error.ServiceUnknown") => {
            (Kind::ProviderUnavailable, BusCode::ServiceUnknown)
        }
        Some("org.freedesktop.DBus.Error.Disconnected" | "org.freedesktop.DBus.Error.NoServer") => {
            (Kind::BackendUnavailable, BusCode::Disconnected)
        }
        _ => (Kind::NativeFailure, BusCode::Other),
    };
    let mut diagnostic = Diagnostic::new(
        if stage == Stage::Session && kind == Kind::ProviderUnavailable {
            Kind::BackendUnavailable
        } else {
            kind
        },
        stage,
    );
    diagnostic.bus_code = Some(code);
    diagnostic
}

/// A failed decoder/decryptor has a specific cause, while deadlines/denials retain theirs.
#[cfg(any(target_os = "linux", target_os = "windows", test))]
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::error::{Context, Result};

    #[test]
    fn decryption_context_preserves_timeouts_and_denials_and_labels_other_decrypt_failures() {
        for (cause, expected) in [
            (std::io::ErrorKind::TimedOut, Kind::Timeout),
            (std::io::ErrorKind::PermissionDenied, Kind::PermissionDenied),
            (std::io::ErrorKind::InvalidData, Kind::DecryptionFailed),
        ] {
            let error = specific(
                std::io::Error::from(cause).into(),
                Kind::DecryptionFailed,
                Stage::Decrypt,
            );
            let detail = diagnostic(&error, Stage::Read);
            assert_eq!(detail.kind, expected);
            assert_eq!(detail.stage, Stage::Decrypt);
            assert!(error.downcast_ref::<std::io::Error>().is_some());
        }
    }

    #[test]
    fn codec_and_native_causes_keep_distinct_kinds_and_stages() -> Result<()> {
        let cases = [
            (
                crate::system::shared::credentials::decode("not json").err(),
                Kind::InvalidFormat,
            ),
            (
                crate::system::shared::credentials::decode(
                    r#"{"version":2,"username":"a","password":"b"}"#,
                )
                .err(),
                Kind::UnsupportedVersion,
            ),
            (
                crate::system::shared::credentials::decode(r#"{"version":1,"username":"a"}"#).err(),
                Kind::Incomplete,
            ),
        ];
        for (error, kind) in cases {
            let nested = Err::<(), _>(error.context("expected codec error")?)
                .context("task wrapper")
                .err()
                .context("wrapped error")?;
            let detail = diagnostic(&nested, Stage::Read);
            assert_eq!(detail.kind, kind);
            assert_eq!(detail.stage, Stage::Decode);
            assert!(matches!(background(&nested), BackgroundRead::Failed(_)));
        }
        let denied = Error::from(std::io::Error::from(std::io::ErrorKind::PermissionDenied));
        assert!(matches!(
            background(&denied),
            BackgroundRead::Waiting(WaitReason::PermissionDenied)
        ));
        let timeout = Error::from(std::io::Error::from(std::io::ErrorKind::TimedOut));
        assert!(matches!(
            background(&timeout),
            BackgroundRead::Failed(Diagnostic {
                kind: Kind::Timeout,
                ..
            })
        ));
        let unknown = Error::message("unclassified backend failure");
        assert!(matches!(
            background(&unknown),
            BackgroundRead::Failed(Diagnostic {
                kind: Kind::Unknown,
                ..
            })
        ));
        Ok(())
    }

    #[test]
    fn sensitive_native_payload_is_retained_as_source_but_never_formatted_or_serialized()
    -> Result<()> {
        let secret = "DO_NOT_PRINT_SAVED_PASSWORD";
        let raw = Error::from(keyring::Error::BadEncoding(secret.as_bytes().to_vec()));
        let error = at(raw, Stage::Read);
        assert!(error.downcast_ref::<keyring::Error>().is_some());
        assert!(!format!("{error}").contains(secret));
        assert!(!format!("{error:?}").contains(secret));
        let detail = diagnostic(&error, Stage::Task);
        assert_eq!(detail.kind, Kind::InvalidEncoding);
        assert_eq!(detail.stage, Stage::Decode);
        assert!(!serde_json::to_string(&detail)?.contains(secret));
        Ok(())
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn dbus_timeout_and_session_query_failure_are_not_provider_or_permission_waits() -> Result<()> {
        let request = zbus::Message::method("/mock", "Mock")?.build(&())?;
        for (name, stage, kind, code) in [
            (
                "org.freedesktop.DBus.Error.NoReply",
                Stage::Connect,
                Kind::Timeout,
                BusCode::NoReply,
            ),
            (
                "org.freedesktop.DBus.Error.ServiceUnknown",
                Stage::Connect,
                Kind::ProviderUnavailable,
                BusCode::ServiceUnknown,
            ),
            (
                "org.freedesktop.DBus.Error.ServiceUnknown",
                Stage::Session,
                Kind::BackendUnavailable,
                BusCode::ServiceUnknown,
            ),
            (
                "org.freedesktop.DBus.Error.AccessDenied",
                Stage::Read,
                Kind::PermissionDenied,
                BusCode::AccessDenied,
            ),
        ] {
            let native = zbus::Error::MethodError(
                name.try_into().context("mock D-Bus error name")?,
                Some("untrusted provider detail".into()),
                request.clone(),
            );
            let error = at(native.into(), stage);
            let detail = diagnostic(&error, Stage::Task);
            assert_eq!(detail.kind, kind);
            assert_eq!(detail.stage, stage);
            assert_eq!(detail.bus_code, Some(code));
        }
        Ok(())
    }
}
