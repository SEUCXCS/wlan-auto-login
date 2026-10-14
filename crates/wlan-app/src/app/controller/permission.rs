//! Application-facing entrypoints; native policy belongs to system flows.
pub(crate) use crate::system::flow::permission::{
    check::run as check, get::run as get, list::run as list,
};
