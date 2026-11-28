//! Permission operation entrypoints.
pub(crate) mod check;
pub(crate) mod get;
pub(crate) mod list;
pub(crate) mod report;

#[derive(Clone, Copy)]
pub(crate) enum Step {
    List,
    Select,
    Check,
    Request,
}
