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

pub(crate) use ipc::{CredentialState, RuntimeStatus, Signal, serve_with_mode, status, stop, wake};
#[cfg(test)]
pub(crate) use ipc::{Endpoint, request, serve};
#[cfg(test)]
pub(crate) use protocol::{Command, Request, Response, VERSION, read, write};
#[cfg(test)]
pub(crate) use transport::connect as test_connect;

pub(crate) use ipc::{managed_pid, managed_running, stop_managed};

pub(crate) use lock::managed_lock;
pub(crate) use lock::{operation_lock, service_lock, singleton, wait_stopped};
pub(crate) use state::State;

#[cfg(test)]
pub(crate) const RETAIN_LOCKS_ON_PURGE: bool = true;
