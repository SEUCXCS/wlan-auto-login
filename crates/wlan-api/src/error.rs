#[derive(Debug, thiserror::Error)]
pub enum Error {
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

impl From<reqwest::Error> for Error {
    fn from(error: reqwest::Error) -> Self {
        // Request URLs contain the password for this legacy GET protocol.
        Self::Transport(error.without_url())
    }
}
