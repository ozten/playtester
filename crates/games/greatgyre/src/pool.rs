//! Static card-pool construction.
//!
//! [`build_catalog`] is the single place that assigns
//! [`CardInstanceId`]s — every physical card in a `num_players`-seat
//! game is created exactly once here, at setup, per the "every
//! physical card gets a stable instance id" requirement. Nothing else
//! in the crate should construct a [`Card`] from scratch.
//!
//! Quantities are sourced from `docs/greatgyre.md`'s deck-composition
//! table:
//!
//! | Category | Count |
//! |---|---|
//! | Survivors | 12 (1 each) |
//! | Modifications | 23 (Net×6, Quarterdeck×3, Sail×3, Garden×3, Rain Trap×3, Fishing Rod×2, Telescope×2, Toolkit×1) |
//! | Dead Fish | 2 |
//! | Event cards | 10 |
//! | Resources | 77 (Rope×31, Wood×25, Plastic×21) |
//! | Raft Extensions | 10 (shared pile) |
//! | Raft Left/Right pairs | `num_players` (one pair dealt per seat) |
//!
//! Main Deck = 12 survivors + 23 modifications + 2 Dead Fish = 37,
//! matching the spec's `Main Deck (37)` line exactly.

use crate::card::{Card, CardInstanceId, CardKind, EventKind, ModificationKind, SurvivorId};
use crate::resource::Resource;

/// Number of Dead Fish reaction cards in the deck.
pub const DEAD_FISH_COUNT: u8 = 2;

/// Number of raft extensions in the shared pile.
pub const RAFT_EXTENSION_COUNT: u16 = 10;

/// Assigns sequential [`CardInstanceId`]s as cards are constructed.
struct IdCounter(u32);

impl IdCounter {
    const fn new() -> Self {
        Self(0)
    }

    fn next(&mut self) -> CardInstanceId {
        let id = CardInstanceId(self.0);
        self.0 += 1;
        id
    }
}

/// Every physical card needed for a `num_players`-seat game, freshly
/// instantiated with stable ids. Nothing here is shuffled — that's the
/// caller's job (via the `Rng` port), since instance-id assignment
/// must stay independent of shuffle order for determinism to be
/// legible in the log.
#[derive(Debug, Clone)]
pub struct Catalog {
    /// One (left, right) base-raft pair per seat, in seat order.
    pub raft_pairs: Vec<(Card, Card)>,
    /// All 12 survivors, offered at the draft.
    pub survivors: Vec<Card>,
    /// The 23 modifications + 2 Dead Fish + 77 resources that get
    /// shuffled together with draft leftovers into the Deep Sea Deck.
    pub shuffle_pool: Vec<Card>,
    /// The 10 event cards, shuffled and dealt separately (1 per seat).
    pub events: Vec<Card>,
    /// The 10 shared raft-extension cards.
    pub extensions: Vec<Card>,
}

/// Build the full catalog for a `num_players`-seat game.
#[must_use]
pub fn build_catalog(num_players: u8) -> Catalog {
    let mut ids = IdCounter::new();

    let raft_pairs = (0..num_players)
        .map(|_| {
            let left = Card::new(ids.next(), CardKind::RaftLeft);
            let right = Card::new(ids.next(), CardKind::RaftRight);
            (left, right)
        })
        .collect();

    let survivors = SurvivorId::ALL
        .iter()
        .map(|&s| Card::new(ids.next(), CardKind::Survivor(s)))
        .collect();

    let mut shuffle_pool = Vec::new();
    for &m in &ModificationKind::ALL {
        for _ in 0..m.print_count() {
            shuffle_pool.push(Card::new(ids.next(), CardKind::Modification(m)));
        }
    }
    for _ in 0..DEAD_FISH_COUNT {
        shuffle_pool.push(Card::new(ids.next(), CardKind::DeadFish));
    }
    for &r in &Resource::ALL {
        for _ in 0..r.print_count() {
            shuffle_pool.push(Card::new(ids.next(), CardKind::Resource(r)));
        }
    }

    let mut events = Vec::new();
    for &e in &EventKind::ALL {
        for _ in 0..e.print_count() {
            events.push(Card::new(ids.next(), CardKind::Event(e)));
        }
    }

    let extensions = (0..RAFT_EXTENSION_COUNT)
        .map(|_| Card::new(ids.next(), CardKind::RaftExtension))
        .collect();

    Catalog {
        raft_pairs,
        survivors,
        shuffle_pool,
        events,
        extensions,
    }
}

/// Total number of physical cards in a `num_players`-seat game's
/// catalog. `37` (Main Deck) `+ 10` (events) `+ 77` (resources)
/// `+ 10` (extensions) `+ 2 * num_players` (raft base pairs).
#[must_use]
pub fn total_catalog_size(num_players: u8) -> u32 {
    134 + 2 * u32::from(num_players)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog_len(c: &Catalog) -> u32 {
        let sizes = [
            c.raft_pairs.len() * 2,
            c.survivors.len(),
            c.shuffle_pool.len(),
            c.events.len(),
            c.extensions.len(),
        ];
        sizes
            .into_iter()
            .map(|n| u32::try_from(n).expect("catalog sizes fit in u32"))
            .sum()
    }

    #[test]
    fn catalog_size_matches_total_for_all_player_counts() {
        for n in 2..=4u8 {
            let c = build_catalog(n);
            assert_eq!(catalog_len(&c), total_catalog_size(n), "n={n}");
        }
    }

    #[test]
    fn shuffle_pool_has_one_hundred_two_cards() {
        // 23 modifications + 2 dead fish + 77 resources = 102.
        let c = build_catalog(2);
        assert_eq!(c.shuffle_pool.len(), 102);
    }

    #[test]
    fn main_deck_equals_thirty_seven() {
        // survivors (12) + modifications (23) + dead fish (2) = 37.
        let c = build_catalog(2);
        let mods_and_deadfish = c.shuffle_pool.len() - 77;
        assert_eq!(c.survivors.len() + mods_and_deadfish, 37);
    }

    #[test]
    fn all_instance_ids_are_unique() {
        let c = build_catalog(4);
        let mut ids = Vec::new();
        for (l, r) in &c.raft_pairs {
            ids.push(l.id);
            ids.push(r.id);
        }
        ids.extend(c.survivors.iter().map(|c| c.id));
        ids.extend(c.shuffle_pool.iter().map(|c| c.id));
        ids.extend(c.events.iter().map(|c| c.id));
        ids.extend(c.extensions.iter().map(|c| c.id));
        let mut sorted = ids.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), ids.len(), "duplicate CardInstanceId found");
    }
}
