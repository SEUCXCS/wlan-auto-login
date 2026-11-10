use crate::infra::error::Message;
use crate::infra::error::Result;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Scope {
    #[default]
    User,
    System,
}

impl Scope {
    pub fn select(user: bool, system: bool, elevated: bool) -> Self {
        if system || (!user && elevated) {
            Self::System
        } else {
            Self::User
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::System => "system",
        }
    }

    pub fn require_access(self) -> Result<()> {
        let elevated = elevated()?;
        if self == Self::System && !elevated {
            crate::infra::error::bail!(Message::ElevationRequired);
        }
        if self == Self::User && elevated {
            crate::infra::error::bail!(Message::UserSessionRequired);
        }
        Ok(())
    }
}

pub fn elevated() -> Result<bool> {
    {
        // SAFETY: geteuid has no preconditions and does not change process identity.
        Ok(unsafe { libc::geteuid() } == 0)
    }
}
