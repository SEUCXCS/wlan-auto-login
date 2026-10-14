//! Use the shared status workflow; no additional platform steps.
pub(crate) use crate::system::shared::flow::status::run;

#[cfg(test)]
pub(crate) use crate::system::shared::flow::status::inspect;
