//! Typed application errors with source-preserving boundary conversion.
pub(crate) mod credentials;
pub(crate) mod message;
pub(crate) mod portal;
pub(crate) use message::Message;

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Credential(#[from] credentials::CredentialError),
    #[error("{0}")]
    Message(#[from] Message),
    #[error("{message}: {source}")]
    Context {
        message: Message,
        #[source]
        source: Box<Error>,
    },
    #[error(transparent)]
    Portal(#[from] portal::PortalError),
    #[error("{0}")]
    External(#[source] anyhow::Error),
}

impl Error {
    pub fn message(message: impl Into<Message>) -> Self {
        Self::Message(message.into())
    }

    fn external(error: anyhow::Error) -> Self {
        let error = match error.downcast::<Self>() {
            Ok(error) => return error,
            Err(error) => error,
        };
        let error = match error.downcast::<credentials::CredentialError>() {
            Ok(error) => return Self::Credential(error),
            Err(error) => error,
        };
        match error.downcast::<wlanlogin_lib::Error>() {
            Ok(error) => Self::from(error),
            Err(error) => Self::External(error),
        }
    }

    pub fn downcast_ref<E: std::error::Error + Send + Sync + 'static>(&self) -> Option<&E> {
        match self {
            Self::Context { source, .. } => source.downcast_ref(),
            Self::Credential(error) => {
                (error as &dyn std::error::Error)
                    .downcast_ref()
                    .or_else(|| {
                        error
                            .source_error()
                            .and_then(|source| source.downcast_ref())
                    })
            }
            Self::External(error) => error.downcast_ref(),
            Self::Portal(error) => (&error.source as &dyn std::error::Error).downcast_ref(),
            Self::Message(message) => (message as &dyn std::error::Error).downcast_ref(),
        }
    }
}

impl From<wlanlogin_lib::Error> for Error {
    fn from(source: wlanlogin_lib::Error) -> Self {
        Self::Portal(portal::PortalError::new(source))
    }
}
macro_rules! native_errors {
    ($($error:ty),* $(,)?) => {$ (
        impl From<$error> for Error {
            fn from(error: $error) -> Self { Self::external(error.into()) }
        }
    )*};
}
native_errors!(
    std::io::Error,
    serde_json::Error,
    tokio::task::JoinError,
    tokio::time::error::Elapsed,
    tokio::sync::oneshot::error::RecvError,
    std::str::Utf8Error,
    std::ffi::NulError,
    std::string::FromUtf8Error,
    keyring::Error,
    std::sync::mpsc::RecvTimeoutError,
    std::sync::mpsc::SendError<()>,
    anyhow::Error
);
#[cfg(target_os = "linux")]
native_errors!(
    zbus::Error,
    zbus::zvariant::Error,
    dbus_secret_service::Error
);
#[cfg(target_os = "macos")]
native_errors!(security_framework::base::Error);

pub trait Context<T> {
    fn context(self, message: impl Into<Message>) -> Result<T>;
    fn with_context(self, message: impl FnOnce() -> String) -> Result<T>;
}
impl<T, E: Into<anyhow::Error>> Context<T> for std::result::Result<T, E> {
    fn context(self, message: impl Into<Message>) -> Result<T> {
        self.map_err(|error| Error::Context {
            message: message.into(),
            source: Box::new(Error::external(error.into())),
        })
    }
    fn with_context(self, message: impl FnOnce() -> String) -> Result<T> {
        self.map_err(|error| Error::Context {
            message: message().into(),
            source: Box::new(Error::external(error.into())),
        })
    }
}
impl<T> Context<T> for Option<T> {
    fn context(self, message: impl Into<Message>) -> Result<T> {
        self.ok_or_else(|| Error::message(message))
    }
    fn with_context(self, message: impl FnOnce() -> String) -> Result<T> {
        self.ok_or_else(|| Error::message(message()))
    }
}

macro_rules! error {
    ($format:literal $(,)?) => { $crate::infra::error::Error::message(format!($format)) };
    ($message:expr $(,)?) => { $crate::infra::error::Error::message($message) };
    ($format:literal, $($args:tt)*) => { $crate::infra::error::Error::message(format!($format, $($args)*)) };
}
macro_rules! bail {
    ($($args:tt)*) => { return Err($crate::infra::error::error!($($args)*)) };
}
macro_rules! ensure {
    ($condition:expr, $($args:tt)*) => { if !$condition { $crate::infra::error::bail!($($args)*); } };
}
pub(crate) use {bail, ensure, error};
