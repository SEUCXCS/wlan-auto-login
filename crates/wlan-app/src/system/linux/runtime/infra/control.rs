//! Shared state and process coordination for commands, daemon and service management.
#[path = "control/ipc.rs"]
mod ipc;
#[path = "control/lock.rs"]
mod lock;
#[path = "../../../../infra/control/protocol.rs"]
mod protocol;
#[path = "../../../../infra/control/state.rs"]
mod state;
#[path = "control/transport.rs"]
mod transport;

#[cfg(test)]
pub(crate) use ipc::serve;
pub(crate) use ipc::{CredentialState, RuntimeStatus, Signal, serve_with_mode, status, stop, wake};

pub(crate) use lock::{operation_lock, service_lock, singleton, wait_stopped};
pub(crate) use state::State;

pub(crate) use ipc::{session_end, session_ready};
