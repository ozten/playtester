//! `GreatGyrePublicView` — placeholder redacted view.
//!
//! **Scope note**: proper per-viewer redaction (hide opponents' hands,
//! hide face-down Current identities from everyone including the
//! owner, etc. — see `docs/greatgyre.md`'s "Hidden information &
//! views" section) is Unit 5 work. Units 1-2 only need *some*
//! `Game::PublicView` to satisfy the trait and let `GameLoop::run`
//! drive agents through a full game; `RandomAgent` and the other
//! Unit-1/2 test agents never read `view` at all, so a full-information
//! passthrough is harmless here and gets replaced wholesale in Unit 5.

use playtest_core::PlayerId;
use serde::{Deserialize, Serialize};

use crate::state::GameState;

/// Full-information passthrough. **Not** the spec's redacted view —
/// see module docs. Every field is a direct clone of `GameState`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GreatGyrePublicView {
    pub observer: PlayerId,
    pub state: GameState,
}

#[must_use]
pub fn public_view(state: &GameState, observer: PlayerId) -> GreatGyrePublicView {
    GreatGyrePublicView {
        observer,
        state: state.clone(),
    }
}
