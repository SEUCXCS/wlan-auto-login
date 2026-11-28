//! Operation entrypoints. Native adapters provide only platform differences.
pub(crate) mod monitor;
pub(crate) mod state;

#[derive(Clone, Copy)]
pub(crate) enum Step {
    Monitor,
    State,
}
