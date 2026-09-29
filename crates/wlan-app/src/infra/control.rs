//! Shared state and process coordination for commands, daemon and service management.
mod ipc;
mod lock;
mod state;
mod transport;

pub(crate) use ipc::{Signal, serve, stop, wake};
pub(crate) use lock::{operation_lock, service_lock, singleton, wait_stopped};
pub(crate) use state::State;
