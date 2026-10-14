//! The persisted credential format; no native storage or activation here.
use crate::infra::error::{
    Message, Result,
    credentials::{CredentialError, Kind, Stage},
};
use serde::{Deserialize, Serialize};
use wlanlogin_lib::Credentials;
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

#[derive(Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
pub(crate) struct Secret {
    version: Option<u8>,
    username: Option<String>,
    password: Option<String>,
}

pub(crate) fn encode(credentials: &Credentials) -> Result<Zeroizing<String>> {
    crate::infra::error::ensure!(
        !credentials.username.trim().is_empty() && !credentials.password.is_empty(),
        Message::UsernameAndPasswordMustNotBeEmpty
    );
    Ok(Zeroizing::new(serde_json::to_string(&Secret {
        version: Some(1),
        username: Some(credentials.username.clone()),
        password: Some(credentials.password.clone()),
    })?))
}

pub(crate) fn decode(text: &str) -> Result<Credentials> {
    let secret: Secret = serde_json::from_str(text).map_err(|error| {
        CredentialError::with_source(
            crate::infra::error::credentials::Diagnostic::new(Kind::InvalidFormat, Stage::Decode),
            error.into(),
        )
    })?;
    let Some(version) = secret.version else {
        return Err(CredentialError::new(Kind::Incomplete, Stage::Decode).into());
    };
    if version != 1 {
        return Err(CredentialError::new(Kind::UnsupportedVersion, Stage::Decode).into());
    }
    let Some((username, password)) = secret.username.as_ref().zip(secret.password.as_ref()) else {
        return Err(CredentialError::new(Kind::Incomplete, Stage::Decode).into());
    };
    if username.trim().is_empty() || password.is_empty() {
        return Err(CredentialError::new(Kind::Incomplete, Stage::Decode).into());
    }
    Ok(Credentials {
        username: username.clone(),
        password: password.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn current_storage_format_round_trips_without_changing_the_saved_schema() -> Result<()> {
        let credentials = Credentials {
            username: "test@free".into(),
            password: "test-only".into(),
        };
        let encoded = encode(&credentials)?;
        let value: serde_json::Value = serde_json::from_str(&encoded)?;
        assert_eq!(value["version"], 1);
        let decoded = decode(&encoded)?;
        assert_eq!(decoded.username, credentials.username);
        assert_eq!(decoded.password, credentials.password);
        Ok(())
    }
    #[test]
    fn rejects_incomplete_or_unknown_saved_credentials() {
        assert!(decode(r#"{"version":1,"username":"student","password":""}"#).is_err());
        assert!(decode(r#"{"version":2,"username":"student","password":"secret"}"#).is_err());
        assert!(decode("not json").is_err());
    }
}
