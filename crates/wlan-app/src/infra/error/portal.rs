//! English summaries without rendering portal response text or request URLs.
use cxxy_wlanlogin_lib::{Error, RequestOperation};

#[derive(Debug, thiserror::Error)]
#[error("{summary}")]
pub struct PortalError {
    summary: String,
    #[source]
    pub(super) source: Error,
}
impl PortalError {
    pub fn new(source: Error) -> Self {
        fn summary(error: &Error) -> String {
            match error {
                Error::Request { operation, source } => format!(
                    "{} failed: {}",
                    match operation {
                        RequestOperation::Status => "Portal status request",
                        RequestOperation::Login => "Portal login request",
                        RequestOperation::Logout => "Portal logout request",
                    },
                    summary(source)
                ),
                Error::Transport(error) => if error.is_timeout() {
                    "Request timed out"
                } else if error.is_connect() {
                    "Connection failed"
                } else {
                    "Network request failed"
                }
                .into(),
                Error::Protocol(_) => "Unsupported portal response".into(),
                Error::MissingIp => "Portal response has no valid device IPv4 address".into(),
                Error::EmptyCredentials => "Username and password must not be empty".into(),
                Error::Rejected {
                    credential_rejected: true,
                    ..
                } => "Credentials rejected".into(),
                Error::Rejected { .. } => "Portal rejected the operation".into(),
            }
        }
        Self {
            summary: summary(&source),
            source,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn summaries_preserve_classification_without_rendering_untrusted_details() {
        let source = Error::Request {
            operation: RequestOperation::Login,
            source: Box::new(Error::Rejected {
                message: "密码错误 https://example.invalid/?password=secret".into(),
                credential_rejected: true,
            }),
        };
        let error = crate::infra::error::Error::from(source);
        assert_eq!(
            error.to_string(),
            "Portal login request failed: Credentials rejected"
        );
        assert!(
            error
                .downcast_ref::<Error>()
                .is_some_and(Error::is_credential_rejected)
        );
        assert!(!error.to_string().contains("secret"));
        assert!(error.to_string().is_ascii());
    }
}
