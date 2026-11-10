//! Native wireless adapter selected by the common monitor flow.
mod generations;
#[cfg(windows)]
mod native;
#[cfg(windows)]
pub(crate) use native::Monitor;
