#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestOperation {
    Status,
    Login,
    Logout,
}

impl std::fmt::Display for RequestOperation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Status => "校园网状态查询",
            Self::Login => "校园网登录请求",
            Self::Logout => "校园网注销请求",
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{operation}失败")]
    Request {
        operation: RequestOperation,
        #[source]
        source: Box<Error>,
    },
    #[error("网络请求失败: {0}")]
    Transport(#[source] reqwest::Error),
    #[error("门户响应格式不受支持: {0}")]
    Protocol(&'static str),
    #[error("门户响应中没有有效的设备 IPv4 地址")]
    MissingIp,
    #[error("账号或密码不能为空")]
    EmptyCredentials,
    #[error("门户拒绝操作: {message}")]
    Rejected {
        message: String,
        credential_rejected: bool,
    },
}

impl Error {
    pub fn is_credential_rejected(&self) -> bool {
        match self {
            Self::Rejected {
                credential_rejected,
                ..
            } => *credential_rejected,
            Self::Request { source, .. } => source.is_credential_rejected(),
            _ => false,
        }
    }

    pub(crate) fn during(self, operation: RequestOperation) -> Self {
        Self::Request {
            operation,
            source: Box::new(self),
        }
    }

    /// Only transient failures of the read-only status request qualify.
    pub fn is_transient_status_failure(&self) -> bool {
        matches!(self, Self::Request { operation: RequestOperation::Status, source }
            if matches!(source.as_ref(), Self::Transport(error)
                if error.is_timeout() || error.is_connect() || error.is_request()
                    || error.is_body() || error.status().is_some_and(|s| s.is_server_error())))
    }
}

impl From<reqwest::Error> for Error {
    fn from(error: reqwest::Error) -> Self {
        // Request URLs contain the password for this legacy GET protocol.
        Self::Transport(error.without_url())
    }
}
