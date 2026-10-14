//! Extensible platform permission and credential-access diagnostics.
#![allow(dead_code)] // Frontends may use descriptors without exposing a permission command.
use crate::infra::error::{Error, Message, Result};
use std::{collections::BTreeMap, future::Future};

/// An open identifier namespace: new permission kinds require no shared enum change.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize)]
#[serde(transparent)]
pub struct PermissionId(pub String);

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct PermissionDescriptor {
    pub id: PermissionId,
    pub name: String,
    pub description: Option<String>,
    pub metadata: BTreeMap<String, String>,
}

impl PermissionDescriptor {
    pub fn new(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            id: PermissionId(id.into()),
            name: name.into(),
            description: None,
            metadata: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum PermissionStatus {
    NotImplemented,
    Unknown,
    NotDetermined,
    Granted,
    Denied,
    Restricted,
    Unavailable,
    Missing,
    NeedsInteraction,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PermissionRequestOutcome {
    NotImplemented,
    Completed(PermissionStatus),
}

/// Versioned diagnostics shared by frontends and installed-process checks.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PermissionResult {
    pub id: String,
    pub status: PermissionStatus,
    pub error: Option<String>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PermissionReport {
    pub version: u32,
    pub scope: String,
    pub permissions: Vec<PermissionResult>,
}

impl PermissionReport {
    pub fn new(scope: &str, permissions: Vec<PermissionResult>) -> Self {
        Self {
            version: 1,
            scope: scope.into(),
            permissions,
        }
    }

    pub fn into_status(self, scope: &str, id: &str) -> Result<PermissionStatus> {
        crate::infra::error::ensure!(
            self.version == 1 && self.scope == scope && self.permissions.len() == 1,
            Message::PermissionProtocol
        );
        let result = self
            .permissions
            .into_iter()
            .next()
            .ok_or(Message::PermissionProtocol)?;
        crate::infra::error::ensure!(
            result.id == id && result.error.is_none(),
            Message::PermissionProtocol
        );
        Ok(result.status)
    }
}

pub trait PermissionProvider {
    /// May be empty. Descriptors and metadata are owned so providers can discover them dynamically.
    fn permissions(&self) -> Vec<PermissionDescriptor>;

    fn status(&self, id: &PermissionId) -> impl Future<Output = Result<PermissionStatus>> {
        async move {
            self.validate(id)?;
            Ok(PermissionStatus::NotImplemented)
        }
    }

    fn request(&self, id: &PermissionId) -> impl Future<Output = Result<PermissionRequestOutcome>> {
        async move {
            self.validate(id)?;
            Ok(PermissionRequestOutcome::NotImplemented)
        }
    }

    fn validate(&self, id: &PermissionId) -> Result<()> {
        if self
            .permissions()
            .iter()
            .any(|permission| &permission.id == id)
        {
            Ok(())
        } else {
            Err(Error::message(Message::UnknownPermission {
                id: id.0.clone(),
            }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installed_reports_validate_identity_and_never_infer_a_grant() -> Result<()> {
        let make = || {
            PermissionReport::new(
                "user",
                vec![PermissionResult {
                    id: "macos.keychain".into(),
                    status: PermissionStatus::NeedsInteraction,
                    error: None,
                }],
            )
        };
        let json = serde_json::to_string(&make())?;
        assert!(!json.contains("password"));
        assert_eq!(
            serde_json::from_str::<PermissionReport>(&json)?
                .into_status("user", "macos.keychain")?,
            PermissionStatus::NeedsInteraction
        );
        for case in 0..7 {
            let mut report = make();
            match case {
                0 => report.version = 2,
                1 => report.scope = "system".into(),
                2 => report.permissions[0].id = "macos.location".into(),
                3 => report.permissions.clear(),
                4 => report.permissions.push(PermissionResult {
                    id: "macos.keychain".into(),
                    status: PermissionStatus::Granted,
                    error: None,
                }),
                5 => report.permissions[0].error = Some("Keychain unavailable".into()),
                _ => report.permissions[0].error = Some(String::new()),
            }
            assert!(report.into_status("user", "macos.keychain").is_err());
        }
        assert!(
            serde_json::from_str::<PermissionReport>(
                &json.replace("\"version\":1", "\"unexpected\":true,\"version\":1")
            )
            .is_err()
        );
        Ok(())
    }

    #[tokio::test]
    async fn platform_descriptors_are_open_and_unknown_ids_are_rejected() -> Result<()> {
        assert!(
            crate::system::linux::permission::Permissions
                .permissions()
                .is_empty()
        );
        assert!(
            crate::system::windows::permission::Permissions
                .permissions()
                .is_empty()
        );
        let mac = crate::system::macos::permission::Permissions;
        let permissions = mac.permissions();
        assert_eq!(
            permissions
                .iter()
                .map(|permission| permission.id.0.as_str())
                .collect::<Vec<_>>(),
            ["macos.location", "macos.keychain"]
        );
        #[cfg(not(target_os = "macos"))]
        for permission in permissions {
            assert_eq!(
                mac.status(&permission.id).await?,
                PermissionStatus::NotImplemented
            );
            assert_eq!(
                mac.request(&permission.id).await?,
                PermissionRequestOutcome::NotImplemented
            );
        }
        let unknown = PermissionId("unknown.custom.permission".into());
        assert!(mac.status(&unknown).await.is_err());
        assert!(mac.request(&unknown).await.is_err());
        Ok(())
    }

    #[tokio::test]
    async fn custom_permission_collections_allow_empty_and_arbitrary_entries() -> Result<()> {
        struct Custom(usize);
        impl PermissionProvider for Custom {
            fn permissions(&self) -> Vec<PermissionDescriptor> {
                (0..self.0)
                    .map(|index| {
                        let mut descriptor = PermissionDescriptor::new(
                            format!("custom.{index}"),
                            format!("Capability {index}"),
                        );
                        descriptor
                            .metadata
                            .insert("provider-specific-key".into(), "value".into());
                        descriptor
                    })
                    .collect()
            }
        }
        assert!(Custom(0).permissions().is_empty());
        let provider = Custom(100);
        assert_eq!(provider.permissions().len(), 100);
        assert_eq!(
            provider.status(&PermissionId("custom.99".into())).await?,
            PermissionStatus::NotImplemented
        );
        assert!(
            provider
                .request(&PermissionId("custom.100".into()))
                .await
                .is_err()
        );
        Ok(())
    }
}
