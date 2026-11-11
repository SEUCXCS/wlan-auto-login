//! Connection changes belong to the interface that emitted the notification.
use std::{collections::HashMap, sync::Mutex};

pub(super) type InterfaceId = [u8; 16];

#[derive(Default)]
pub(super) struct Generations(Mutex<HashMap<InterfaceId, u64>>);

impl Generations {
    pub(super) fn current(&self, interface: InterfaceId) -> u64 {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&interface)
            .copied()
            .unwrap_or_default()
    }

    pub(super) fn advance(&self, interface: InterfaceId) {
        let mut generations = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let generation = generations.entry(interface).or_default();
        *generation = generation.wrapping_add(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn other_interfaces_do_not_change_the_target_session() {
        let generations = Generations::default();
        let target = [1; 16];
        let other = [2; 16];
        generations.advance(target);
        let session = generations.current(target);
        generations.advance(other);
        generations.advance(other);
        assert_eq!(generations.current(target), session);
        generations.advance(target);
        assert_ne!(generations.current(target), session);
        assert_eq!(generations.current(other), 2);
    }
}
