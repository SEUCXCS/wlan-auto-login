//! English summaries without rendering portal response text or request URLs.
use wlanlogin_lib::{Error, RequestOperation};

#[derive(Debug, thiserror::Error)]
#[error("{summary}")]
pub struct PortalError {
    summary: Summary,
    #[source]
    pub(super) source: Error,
}

#[derive(Debug, thiserror::Error)]
enum Summary {
    #[error("{operation} failed: {reason}")]
    Request {
        operation: &'static str,
        reason: Box<Self>,
    },
    #[error("Request timed out")]
    Timeout,
    #[error("Connection failed")]
    Connection,
    #[error("Network request failed")]
    Transport,
    #[error("Unsupported portal response")]
    Protocol,
    #[error("Portal response has no valid device IPv4 address")]
    MissingIp,
    #[error("Username and password must not be empty")]
    EmptyCredentials,
    #[error("Credentials rejected")]
    CredentialsRejected,
    #[error("Portal rejected the operation")]
    Rejected,
    #[error("{0}")]
    KnownRejection(KnownRejection),
}

#[derive(Debug, thiserror::Error)]
enum KnownRejection {
    #[error("Credentials rejected (ret_code=1)")]
    Credentials,
    #[error("Device IP is already online (ret_code=2)")]
    AlreadyOnline,
    #[error("Authentication server is busy (ret_code=3)")]
    Busy,
    #[error("Unknown authentication server error (ret_code=4)")]
    UnknownServerError,
    #[error("Challenge request failed (ret_code=5)")]
    ChallengeFailed,
    #[error("Challenge request timed out (ret_code=6)")]
    ChallengeTimeout,
    #[error("RADIUS authentication failed (ret_code=7)")]
    RadiusAuthenticationFailed,
    #[error("RADIUS authentication timed out (ret_code=8)")]
    RadiusAuthenticationTimeout,
    #[error("RADIUS logout failed (ret_code=9)")]
    RadiusLogoutFailed,
    #[error("RADIUS logout timed out (ret_code=10)")]
    RadiusLogoutTimeout,
    #[error("Authentication server error (ret_code=11)")]
    ServerError,
    #[error("Portal protocol parameters are incomplete (ret_code=998)")]
    IncompleteParameters,
}

impl KnownRejection {
    fn from_api_message(message: &str) -> Option<Self> {
        // The unchanged API exposes codes only inside its localized error text.
        // Recognize its canonical explanation plus trailing code as a pair; emit
        // only static English templates, never server text or arbitrary digits.
        // Redacted/truncated/unfamiliar messages fall back to the generic summary.
        let (body, code) = message.strip_suffix(')')?.rsplit_once(" (ret_code=")?;
        let (explanation, rejection) = match code {
            "1" => ("账号或密码不正确", Self::Credentials),
            "2" => ("终端 IP 已经在线；无需重复登录", Self::AlreadyOnline),
            "3" => ("系统繁忙，请稍后重试", Self::Busy),
            "4" => ("认证服务器发生未知错误", Self::UnknownServerError),
            "5" => ("REQ_CHALLENGE 失败", Self::ChallengeFailed),
            "6" => ("REQ_CHALLENGE 超时", Self::ChallengeTimeout),
            "7" => ("Radius 认证失败", Self::RadiusAuthenticationFailed),
            "8" => ("Radius 认证超时", Self::RadiusAuthenticationTimeout),
            "9" => ("Radius 下线失败", Self::RadiusLogoutFailed),
            "10" => ("Radius 下线超时", Self::RadiusLogoutTimeout),
            "11" => ("认证服务器发生其他错误", Self::ServerError),
            "998" => ("Portal 协议参数不完整", Self::IncompleteParameters),
            _ => return None,
        };
        let remainder = body.strip_prefix(explanation)?;
        (remainder.is_empty() || remainder.starts_with("；") || remainder.starts_with(" (result="))
            .then_some(rejection)
    }
}

impl Summary {
    fn from_api(error: &Error) -> Self {
        match error {
            Error::Request { operation, source } => Self::Request {
                operation: match operation {
                    RequestOperation::Status => "Portal status request",
                    RequestOperation::Login => "Portal login request",
                    RequestOperation::Logout => "Portal logout request",
                },
                reason: Box::new(Self::from_api(source)),
            },
            Error::Transport(error) if error.is_timeout() => Self::Timeout,
            Error::Transport(error) if error.is_connect() => Self::Connection,
            Error::Transport(_) => Self::Transport,
            Error::Protocol(_) => Self::Protocol,
            Error::MissingIp => Self::MissingIp,
            Error::EmptyCredentials => Self::EmptyCredentials,
            Error::Rejected {
                message,
                credential_rejected,
            } => KnownRejection::from_api_message(message)
                .map(Self::KnownRejection)
                .unwrap_or(if *credential_rejected {
                    Self::CredentialsRejected
                } else {
                    Self::Rejected
                }),
        }
    }
}

impl PortalError {
    pub fn new(source: Error) -> Self {
        Self {
            summary: Summary::from_api(&source),
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

    #[tokio::test]
    async fn known_codes_from_the_api_keep_english_diagnostics_without_server_details()
    -> crate::infra::error::Result<()> {
        use std::time::Duration;
        use tokio::{
            io::{AsyncReadExt, AsyncWriteExt},
            net::TcpListener,
        };
        use wlanlogin_lib::{Client, PortalConfig};
        for code in [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 998, 987654] {
            let listener = TcpListener::bind("127.0.0.1:0").await?;
            let base = format!("http://{}", listener.local_addr()?);
            let client = Client::new(PortalConfig {
                status_url: format!("{base}/status"),
                action_url: format!("{base}/action"),
                timeout: Duration::from_secs(1),
                ..PortalConfig::default()
            })?;
            let server = async {
                for body in [
                    r#"dr1({"result":1,"v46ip":"10.0.0.12"});"#.into(),
                    format!(
                        r#"dr1({{"result":0,"ret_code":{code},"msg":"never-log-this\nforged"}});"#
                    ),
                ] {
                    let (mut stream, _) = listener.accept().await?;
                    let mut request = Vec::new();
                    while !request.ends_with(b"\r\n\r\n") {
                        request.push(stream.read_u8().await?);
                        crate::infra::error::ensure!(
                            request.len() < 16_384,
                            "oversized mock request"
                        );
                    }
                    stream.write_all(format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()
                    ).as_bytes()).await?;
                }
                Ok::<_, crate::infra::error::Error>(())
            };
            let (result, served) = tokio::time::timeout(Duration::from_secs(3), async {
                tokio::join!(client.logout(), server)
            })
            .await?;
            served?;
            let error = crate::infra::error::Error::from(
                result
                    .err()
                    .ok_or_else(|| crate::infra::error::error!("expected portal rejection"))?,
            );
            let message = error.to_string();
            assert!(message.starts_with("Portal logout request failed: "));
            assert!(message.is_ascii());
            assert!(!message.contains("never-log-this") && !message.contains("forged"));
            if code == 987654 {
                assert_eq!(
                    message,
                    "Portal logout request failed: Portal rejected the operation"
                );
            } else {
                assert!(message.contains(&format!("ret_code={code}")), "{message}");
            }
            assert_eq!(
                error
                    .downcast_ref::<Error>()
                    .is_some_and(Error::is_credential_rejected),
                code == 1
            );
        }
        Ok(())
    }

    #[test]
    fn unrecognized_or_redacted_codes_never_render_payload_text() {
        for message in [
            "arbitrary text (ret_code=3)",
            "系统繁忙，请稍后重试 (ret_code=<redacted>)",
            "系统繁忙，请稍后重试 (ret_code=3)\nforged",
            "系统繁忙，请稍后重试 (ret_code=987654)",
        ] {
            let error = PortalError::new(Error::Rejected {
                message: message.into(),
                credential_rejected: false,
            });
            assert_eq!(error.to_string(), "Portal rejected the operation");
        }
    }
}
