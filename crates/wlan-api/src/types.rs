use serde::{Deserialize, Serialize};
use std::{fmt, net::Ipv4Addr, str::FromStr};
use zeroize::{Zeroize, ZeroizeOnDrop};

#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct Credentials {
    pub username: String,
    pub password: String,
}

impl fmt::Debug for Credentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Credentials { <redacted> }")
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Line {
    #[default]
    Free1,
    Free2,
    Campus,
}

impl Line {
    pub const ALL: [Self; 3] = [Self::Free1, Self::Free2, Self::Campus];

    pub fn suffix(self) -> &'static str {
        match self {
            Self::Free1 => "@dianxin",
            Self::Free2 => "@free",
            Self::Campus => "@xyw",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Free1 => "临时免费 1",
            Self::Free2 => "免费 2",
            Self::Campus => "内网",
        }
    }
}

impl fmt::Display for Line {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Free1 => "free1",
            Self::Free2 => "free2",
            Self::Campus => "campus",
        })
    }
}

impl FromStr for Line {
    type Err = &'static str;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "free1" => Ok(Self::Free1),
            "free2" => Ok(Self::Free2),
            "campus" => Ok(Self::Campus),
            _ => Err("线路必须为 free1、free2 或 campus"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthState {
    Online,
    Offline,
    /// An unavailable or unfamiliar response must never trigger a login loop.
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoginOutcome {
    /// The login action accepted the supplied credentials.
    Authenticated,
    /// The device was already online; supplied credentials were not checked.
    AlreadyOnline,
}

#[derive(Clone, Debug)]
pub struct Status {
    pub auth: AuthState,
    pub ip: Ipv4Addr,
    pub ipv6: String,
    pub mac: String,
    pub vlan: String,
    pub ac_ip: Option<String>,
    pub ac_name: Option<String>,
}
