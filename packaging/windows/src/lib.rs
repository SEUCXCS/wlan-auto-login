//! Packaging-only integration with the public wlanlogin CLI.
#[cfg(windows)]
pub mod cli;
#[cfg(windows)]
mod credentials;
#[cfg(windows)]
pub mod diagnostics;
#[cfg(all(windows, test))]
#[path = "../initialize.rs"]
mod initialize;
#[cfg(windows)]
pub mod ipc;
#[cfg(windows)]
mod services;
#[cfg(windows)]
pub mod state;
#[cfg(windows)]
pub mod transaction;
#[cfg(windows)]
pub mod ui;
#[cfg(windows)]
pub mod verify;
#[cfg(windows)]
pub mod win;

#[allow(dead_code)]
#[path = "../../../crates/wlan-app/src/infra/identity.rs"]
pub mod identity;

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use zeroize::ZeroizeOnDrop;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum Scope {
    User,
    Machine,
}

impl Scope {
    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "user" => Ok(Self::User),
            "machine" => Ok(Self::Machine),
            _ => bail!("Invalid installation scope"),
        }
    }
    pub fn cli_flag(self) -> &'static str {
        match self {
            Self::User => "--user",
            Self::Machine => "--system",
        }
    }
}

#[derive(Serialize, Deserialize, ZeroizeOnDrop)]
pub struct Form {
    pub username: String,
    pub password: String,
    pub line: String,
}

impl Form {
    pub fn validate(&self) -> Result<()> {
        anyhow::ensure!(
            !self.username.trim().is_empty() && !self.password.is_empty(),
            "Username and password are required"
        );
        anyhow::ensure!(
            !self.password.contains(['\r', '\n', '\0']),
            "Password contains an unsupported control character"
        );
        anyhow::ensure!(
            ["free1", "free2", "campus"].contains(&self.line.as_str()),
            "Invalid connection line"
        );
        Ok(())
    }
}

pub fn validate_id(value: &str) -> Result<()> {
    anyhow::ensure!(
        value.len() >= 32
            && value.len() <= 38
            && value
                .chars()
                .all(|ch| ch.is_ascii_hexdigit() || "-{}".contains(ch)),
        "Invalid installation transaction ID"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_path_traversal_and_multiline_passwords() {
        assert!(validate_id("../../outside").is_err());
        assert!(validate_id("{397F9B01-F368-468B-AD7C-23822E646F33}").is_ok());
        assert!(
            Form {
                username: "test".into(),
                password: "first\nsecond".into(),
                line: "free1".into()
            }
            .validate()
            .is_err()
        );
    }
}
