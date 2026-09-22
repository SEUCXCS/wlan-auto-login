//! Async wrappers for the CX campus ePortal API.
//!
//! No credential persistence, network monitoring or automatic retries happen here.
mod client;
mod error;
mod types;

pub use client::{Client, PortalConfig};
pub use error::Error;
pub use types::{AuthState, Credentials, Line, Status};
