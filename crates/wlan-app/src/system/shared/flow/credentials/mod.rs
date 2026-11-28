//! Operation entrypoints. Native adapters provide only platform differences.
pub(crate) mod background;
pub(crate) mod delete;
pub(crate) mod load;
pub(crate) mod save;

#[derive(Clone, Copy)]
pub(crate) enum Step {
    Load,
    Invalidate,
    Save,
    Delete,
    BackgroundRead,
    ValidatePermit,
}
