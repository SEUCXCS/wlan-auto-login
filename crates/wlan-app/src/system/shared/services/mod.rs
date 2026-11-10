//! Shared service transactions and dispatch to native managers.

#[allow(unused_imports)]
pub(crate) use crate::system::define::services::{
    Action, Manager, Plan, ServiceManager, ServiceState,
};
use std::collections::HashMap;
pub(crate) mod artifacts;
pub(crate) mod steps;
pub(crate) use artifacts::ensure_no_conflicting_install;

pub(crate) fn fields(text: &str) -> HashMap<&str, &str> {
    text.lines()
        .filter_map(|line| line.split_once('='))
        .map(|(key, value)| (key.trim(), value.trim()))
        .collect()
}
pub(crate) mod hooks;
pub(crate) mod logs;
pub(crate) mod transaction;

pub(crate) mod payload;
