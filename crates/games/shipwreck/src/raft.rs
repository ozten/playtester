//! The [`Raft`] data structure — a player's assembled salvage vessel.
//!
//! A raft is `base_left ++ extensions ++ base_right`. Every slot (both
//! bases, every extension) can optionally hold one equipment upgrade.
//! Extensions can be inserted anywhere between two adjacent slots;
//! they cannot be inserted after the right base (that would put them
//! outside the raft).
//!
//! The data model: [`Raft`] stores a `Vec<RaftExtensionCard>` for the
//! middle and a `BTreeMap<SlotId, EquipmentCard>` for sparsely-populated
//! upgrades. `BTreeMap` gives deterministic serialization order across
//! all platforms — a soft requirement of the JSONL log.
//!
//! `SlotId::Extension(idx)` refers to the extension at position `idx`
//! in the `extensions` vector. When an extension is inserted in the
//! middle, later extensions shift right: if upgrades were installed on
//! a shifted extension, their `SlotId`s must shift too — this module
//! handles that re-indexing.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::card::{BaseRaftCard, EquipmentCard, RaftExtensionCard};

/// Identifies one slot on a raft.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum SlotId {
    /// The left base raft card.
    BaseLeft,
    /// The extension at position `.0` in the raft's `extensions` vector.
    Extension(u16),
    /// The right base raft card.
    BaseRight,
}

/// Errors returned by [`Raft`] methods.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum RaftError {
    /// Attempted to insert an extension *after* `SlotId::BaseRight`.
    /// The right base is always the rightmost card on the raft, so
    /// there is no valid "after" position.
    #[error("cannot insert an extension after BaseRight")]
    CannotInsertAfterBaseRight,
    /// The referenced extension index is out of range.
    #[error("unknown slot: {0:?}")]
    UnknownSlot(SlotId),
    /// An upgrade was built on a slot that already has one.
    #[error("slot already has an upgrade: {0:?}")]
    SlotOccupied(SlotId),
}

/// One player's raft.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Raft {
    pub base_left: BaseRaftCard,
    /// Ordered left→right between `base_left` and `base_right`.
    pub extensions: Vec<RaftExtensionCard>,
    pub base_right: BaseRaftCard,
    /// Sparse: only slots with an upgrade installed are present.
    pub upgrades: BTreeMap<SlotId, EquipmentCard>,
}

impl Raft {
    /// New empty raft — no extensions, no upgrades.
    #[must_use]
    pub fn new(base_left: BaseRaftCard, base_right: BaseRaftCard) -> Self {
        Self {
            base_left,
            extensions: Vec::new(),
            base_right,
            upgrades: BTreeMap::new(),
        }
    }

    /// Total number of cards on the raft: 2 bases + every extension.
    #[must_use]
    pub fn length(&self) -> usize {
        2 + self.extensions.len()
    }

    /// Number of equipment upgrades installed.
    #[must_use]
    pub fn invention_count(&self) -> usize {
        self.upgrades.len()
    }

    /// True if `slot` refers to a currently-existing position on this raft.
    #[must_use]
    pub fn has_slot(&self, slot: SlotId) -> bool {
        match slot {
            SlotId::BaseLeft | SlotId::BaseRight => true,
            SlotId::Extension(i) => usize::from(i) < self.extensions.len(),
        }
    }

    /// Insert a new extension immediately after `insert_after`. Extensions
    /// to the right of the insertion point shift by one, and upgrades
    /// anchored to those extensions shift with them.
    ///
    /// # Errors
    /// - [`RaftError::CannotInsertAfterBaseRight`] if `insert_after == BaseRight`.
    /// - [`RaftError::UnknownSlot`] if `insert_after == Extension(i)` and
    ///   `i >= extensions.len()`.
    pub fn extend(
        &mut self,
        ext: RaftExtensionCard,
        insert_after: SlotId,
    ) -> Result<(), RaftError> {
        let new_pos: usize = match insert_after {
            SlotId::BaseLeft => 0,
            SlotId::Extension(i) => {
                let idx = usize::from(i);
                if idx >= self.extensions.len() {
                    return Err(RaftError::UnknownSlot(insert_after));
                }
                idx + 1
            }
            SlotId::BaseRight => return Err(RaftError::CannotInsertAfterBaseRight),
        };

        // Re-index any upgrades anchored to extensions at or past new_pos.
        // We rebuild the upgrades map — `BTreeMap` doesn't let us mutate
        // keys in place.
        let old_upgrades = std::mem::take(&mut self.upgrades);
        for (slot, eq) in old_upgrades {
            let new_slot = match slot {
                SlotId::Extension(i) if usize::from(i) >= new_pos => {
                    SlotId::Extension(i.checked_add(1).expect("extension index overflow"))
                }
                other => other,
            };
            self.upgrades.insert(new_slot, eq);
        }

        self.extensions.insert(new_pos, ext);
        Ok(())
    }

    /// Install an equipment upgrade on `slot`.
    ///
    /// # Errors
    /// - [`RaftError::UnknownSlot`] if `slot` doesn't exist on this raft.
    /// - [`RaftError::SlotOccupied`] if `slot` already has an upgrade.
    pub fn build_upgrade(
        &mut self,
        slot: SlotId,
        eq: EquipmentCard,
    ) -> Result<(), RaftError> {
        if !self.has_slot(slot) {
            return Err(RaftError::UnknownSlot(slot));
        }
        if self.upgrades.contains_key(&slot) {
            return Err(RaftError::SlotOccupied(slot));
        }
        self.upgrades.insert(slot, eq);
        Ok(())
    }

    /// Peek at the equipment installed on `slot`, if any.
    #[must_use]
    pub fn upgrade_at(&self, slot: SlotId) -> Option<&EquipmentCard> {
        self.upgrades.get(&slot)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::card::{BaseRaftSide, EquipmentKind};

    fn base_pair() -> (BaseRaftCard, BaseRaftCard) {
        (
            BaseRaftCard::new(BaseRaftSide::Left),
            BaseRaftCard::new(BaseRaftSide::Right),
        )
    }

    #[test]
    fn new_raft_has_length_two() {
        let (l, r) = base_pair();
        let raft = Raft::new(l, r);
        assert_eq!(raft.length(), 2);
        assert_eq!(raft.invention_count(), 0);
    }

    #[test]
    fn extend_after_base_left_places_at_front() {
        let (l, r) = base_pair();
        let mut raft = Raft::new(l, r);
        raft.extend(RaftExtensionCard::new(42), SlotId::BaseLeft)
            .unwrap();
        assert_eq!(raft.length(), 3);
        assert_eq!(raft.extensions[0].serial, 42);
    }

    #[test]
    fn extend_reindexes_existing_upgrades() {
        let (l, r) = base_pair();
        let mut raft = Raft::new(l, r);
        raft.extend(RaftExtensionCard::new(1), SlotId::BaseLeft)
            .unwrap();
        raft.build_upgrade(
            SlotId::Extension(0),
            EquipmentCard::new(EquipmentKind::Telescope, 0),
        )
        .unwrap();
        // Insert a new extension at position 0 — old Extension(0) should
        // become Extension(1), and its upgrade should follow.
        raft.extend(RaftExtensionCard::new(2), SlotId::BaseLeft)
            .unwrap();
        assert_eq!(raft.extensions[0].serial, 2);
        assert_eq!(raft.extensions[1].serial, 1);
        assert!(raft.upgrade_at(SlotId::Extension(0)).is_none());
        assert!(raft.upgrade_at(SlotId::Extension(1)).is_some());
    }
}
