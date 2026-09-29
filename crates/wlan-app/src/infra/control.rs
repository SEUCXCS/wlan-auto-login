//! Shared state and process coordination for commands, daemon and service management.
mod ipc;
mod lock;
mod state;
mod transport;

#[cfg(test)]
pub(crate) use ipc::serve;
pub(crate) use ipc::{Signal, serve_with_mode, stop, wake};
#[cfg(target_os = "macos")]
pub(crate) use ipc::{managed_pid, managed_running, stop_managed};
#[cfg(target_os = "macos")]
pub(crate) use lock::managed_lock;
pub(crate) use lock::{operation_lock, service_lock, singleton, wait_stopped};
pub(crate) use state::State;
