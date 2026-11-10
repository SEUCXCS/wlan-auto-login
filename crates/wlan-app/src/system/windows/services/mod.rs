pub(crate) mod definition;
#[cfg(test)]
pub(crate) mod logs;
pub(crate) mod manager;

#[cfg(target_os = "windows")]
pub(crate) mod artifacts;
#[cfg(target_os = "windows")]
mod dispatch;

#[cfg(target_os = "windows")]
pub(crate) use dispatch::execute_optional;

pub(crate) mod wait;

#[cfg(target_os = "windows")]
pub(crate) mod task;
