//! Static card-pool construction.
//!
//! [`build_catalog`] is the single place that assigns
//! [`CardInstanceId`]s — every physical card in a `num_players`-seat
//! game is created exactly once here, at setup, per the "every
//! physical card gets a stable instance id" requirement. Nothing else
//! in the crate should construct a [`Card`] from scratch.
//!
//! ## Ids are permuted per `(num_players, seed)`, not handed out in
//! ## catalog order
//!
//! Earlier versions of this module assigned ids via a plain 0.. counter
//! walked in fixed catalog order (raft pairs, then survivors, then
//! modifications/dead-fish/resources, then events, then extensions).
//! That made id→kind a **public codebook**: id 12, say, was always the
//! first survivor in every single game, seed or no seed, so any client
//! that had ever seen one game could infer a face-down card's identity
//! in every other game purely from its id. `build_catalog` now takes
//! `seed` and shuffles a `0..total_catalog_size(num_players)` id
//! permutation (via a seed derived from `seed`, domain-separated from
//! the harness's `chance_rng` — see `ID_PERMUTATION_SALT`) before
//! handing ids out in that shuffled order instead. Same `(num_players,
//! seed)` still always yields the same catalog — determinism and
//! replay are unaffected — but the mapping now varies per game.
//! `setup::initial_state` and `determinize::full_universe` are the two
//! callers; both must pass the *same* seed (the latter reads it back
//! from `GameState::id_permutation_seed`, which `initial_state` stores)
//! so they reconstruct the identical id→kind mapping instead of each
//! re-deriving their own catalog order.
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

use rand_chacha::rand_core::{RngCore, SeedableRng};

use playtest_ports::Rng;

use crate::card::{Card, CardInstanceId, CardKind, EventKind, ModificationKind, SurvivorId};
use crate::resource::Resource;

/// Number of Dead Fish reaction cards in the deck.
pub const DEAD_FISH_COUNT: u8 = 2;

/// Number of raft extensions in the shared pile.
pub const RAFT_EXTENSION_COUNT: u16 = 10;

/// Domain-separation salt mixed into the master `seed` before deriving
/// the id-permutation sub-seed. Keeps this RNG stream independent of
/// `chance_rng` (also seeded straight from the same master `seed` — see
/// `playtest-registry::play::run_greatgyre_into_sink`), even though
/// both ultimately derive from one `u64`. Arbitrary odd 64-bit
/// constant, same "mix the seed with a fixed constant" convention
/// already used for per-agent seed derivation in
/// `playtest-registry::play::build_ctx_for_seat`.
const ID_PERMUTATION_SALT: u64 = 0xBF58_476D_1CE4_E5B9;

/// Minimal seeded [`Rng`] port impl built directly on
/// `rand_chacha::ChaCha20Rng`. Lives here (rather than depending on
/// `playtest-adapters`, which games take only as a dev-dependency —
/// see `CLAUDE.md`'s ports-and-adapters invariant) mirroring
/// `shipwreck::rules::SeededRng`.
struct SeededRng {
    inner: rand_chacha::ChaCha20Rng,
}

impl SeededRng {
    fn from_seed(seed: u64) -> Self {
        Self {
            inner: rand_chacha::ChaCha20Rng::seed_from_u64(seed),
        }
    }
}

impl Rng for SeededRng {
    fn next_u64(&mut self) -> u64 {
        self.inner.next_u64()
    }

    fn gen_range(&mut self, range: core::ops::Range<u64>) -> Result<u64, playtest_ports::RngError> {
        if range.start >= range.end {
            return Err(playtest_ports::RngError::InvalidRange {
                start: range.start,
                end: range.end,
            });
        }
        let span = range.end - range.start;
        Ok(range.start + self.inner.next_u64() % span)
    }
}

/// Assigns [`CardInstanceId`]s by walking a `perm`utation table
/// (`perm[canonical index] = assigned id`) instead of a raw 0.. counter
/// — see the module doc comment for why.
struct IdCounter<'a> {
    next: usize,
    perm: &'a [u32],
}

impl<'a> IdCounter<'a> {
    const fn new(perm: &'a [u32]) -> Self {
        Self { next: 0, perm }
    }

    fn next(&mut self) -> CardInstanceId {
        let id = CardInstanceId(self.perm[self.next]);
        self.next += 1;
        id
    }
}

/// Build the `0..total_catalog_size(num_players)` id-assignment order:
/// a Fisher-Yates shuffle of the canonical index range, seeded off
/// `seed` (domain-separated via [`ID_PERMUTATION_SALT`]). Same
/// `(num_players, seed)` always yields the same permutation.
fn id_permutation(num_players: u8, seed: u64) -> Vec<u32> {
    let n = total_catalog_size(num_players);
    let mut ids: Vec<u32> = (0..n).collect();
    let mut rng = SeededRng::from_seed(seed ^ ID_PERMUTATION_SALT);
    rng.shuffle(&mut ids);
    ids
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

/// Build the full catalog for a `num_players`-seat game. `seed` drives
/// the id→kind permutation (see the module doc comment) — it must be
/// the same `seed` `Game::initial_state` was called with so
/// `determinize` can reconstruct an identical mapping later.
#[must_use]
pub fn build_catalog(num_players: u8, seed: u64) -> Catalog {
    let perm = id_permutation(num_players, seed);
    let mut ids = IdCounter::new(&perm);

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
            let c = build_catalog(n, 1);
            assert_eq!(catalog_len(&c), total_catalog_size(n), "n={n}");
        }
    }

    #[test]
    fn shuffle_pool_has_one_hundred_two_cards() {
        // 23 modifications + 2 dead fish + 77 resources = 102.
        let c = build_catalog(2, 1);
        assert_eq!(c.shuffle_pool.len(), 102);
    }

    #[test]
    fn main_deck_equals_thirty_seven() {
        // survivors (12) + modifications (23) + dead fish (2) = 37.
        let c = build_catalog(2, 1);
        let mods_and_deadfish = c.shuffle_pool.len() - 77;
        assert_eq!(c.survivors.len() + mods_and_deadfish, 37);
    }

    #[test]
    fn all_instance_ids_are_unique() {
        let c = build_catalog(4, 1);
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

    /// Same `(num_players, seed)` must always yield the same id→kind
    /// mapping — determinism/replay depend on this.
    #[test]
    fn same_seed_yields_identical_id_to_kind_mapping() {
        for n in 2..=4u8 {
            let a = id_to_kind_map(&build_catalog(n, 12345));
            let b = id_to_kind_map(&build_catalog(n, 12345));
            assert_eq!(a, b, "n={n}: same seed produced different id->kind mappings");
        }
    }

    /// The whole point of this unit: two different seeds must (almost
    /// always) assign different ids to the same kind — id is no longer
    /// a fixed public codebook. Checked across many seed pairs so the
    /// test isn't flaky if one particular pair happens to collide.
    #[test]
    fn different_seeds_usually_yield_different_id_to_kind_mappings() {
        let n = 3u8;
        let baseline = id_to_kind_map(&build_catalog(n, 1));
        let mut differs = 0;
        for seed in 2..32u64 {
            let other = id_to_kind_map(&build_catalog(n, seed));
            if other != baseline {
                differs += 1;
            }
        }
        assert!(
            differs >= 28,
            "expected nearly all of 30 alternate seeds to permute differently from seed 1, got {differs}"
        );
    }

    /// Helper: id -> kind map for every card in a catalog, used by the
    /// codebook-leak regression tests above.
    fn id_to_kind_map(c: &Catalog) -> std::collections::BTreeMap<CardInstanceId, CardKind> {
        let mut map = std::collections::BTreeMap::new();
        for (l, r) in &c.raft_pairs {
            map.insert(l.id, l.kind);
            map.insert(r.id, r.kind);
        }
        for card in c
            .survivors
            .iter()
            .chain(c.shuffle_pool.iter())
            .chain(c.events.iter())
            .chain(c.extensions.iter())
        {
            map.insert(card.id, card.kind);
        }
        map
    }
}
