//! Event-card resolution for ShipWreck (Unit 23).
//!
//! Event cards interrupt the ordinary turn-flow in one of three shapes:
//!
//! - **Shark** — one-shot immediate resolution. The caster nominates
//!   one target on another player's raft (an upgrade or a raft
//!   extension, never a base). If the target player has a Steel
//!   Cordage upgrade installed anywhere on their raft, the Steel
//!   Cordage is destroyed instead — the shark is *defended*.
//! - **Typhoon** — multi-player resolution. Every seat (including the
//!   caster) must sacrifice one upgrade or extension of their choice,
//!   or pass if they have nothing to lose. The engine enters
//!   [`Phase::ResolvingEvent`] during resolution; `next_actor` points
//!   at the front of a `VecDeque<PlayerId>` drained front-to-back.
//! - **Flying Fish** — one-shot immediate resolution. Grants the
//!   caster +1 food counter. Per the spec "Treat as extra food for one
//!   turn" — in our model the extra food decays naturally on the next
//!   `EndTurn` when it gets spent.
//!
//! **Nested events are out of scope for Unit 23.** During
//! [`Phase::ResolvingEvent`] the only legal actions are the
//! `ResolveEvent(...)` shapes; `PlayEventCard` cannot be stacked. See
//! `legal_actions` in `rules.rs` for the narrow-set enforcement.
//!
//! [`Phase::ResolvingEvent`]: crate::phase::Phase::ResolvingEvent

pub(crate) mod flying_fish;
pub(crate) mod shark;
pub(crate) mod typhoon;
