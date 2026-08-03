---
date: 2026-08-03
status: in-progress
plan: 003
type: feat
title: Great Gyre engine (official rules) as a new game crate
---

# Great Gyre engine plan

Implement `crates/games/greatgyre` per `docs/greatgyre.md` (the rules spec —
read it first; it is authoritative, including its **[A]** assumption
rulings). `shipwreck` stays untouched; it is a different game. Follow all
architecture invariants in `CLAUDE.md` (ports/adapters, determinism, agents
return indices, no game code in `playtest-server`, release-profile cargo
only, commit each unit to main).

Design decisions already made (do not relitigate):

- **Survivor selection is in-game**: after setup deal, a `SurvivorDraft`
  phase runs — each seat in order picks one of the 12 survivors via a
  normal legal-actions prompt. This lets the human pick via `http-remote`
  and AI seats pick via their agents, with no server/API changes.
- **Phases with no decisions auto-advance** (Phase 1 add, Phase 5 pass,
  automatic parts of Phase 4). No no-op prompts. Chance (Pirate's random
  steal) goes through `Actor::Chance` / `resolve_chance`.
- **Pending-decision stack** (shipwreck's `ResolvingEvent` pattern) models:
  event targeting/resolution, Dead Fish / Fisher reactions, Storm's
  around-the-table choices, Octopus relocation, Love Boat's give-a-survivor,
  discard-down-to-hand-size, and Phase-4 hungry/stand-up choices.
- **Ties**: all tied players win. `end_game` event carries
  `winners: Vec<PlayerId>` + `final_scores`; map to core `GameResult` as
  best it allows (lowest-seat winner if it forces a single winner) — the
  UI reads the event.
- Raft spaces are a fungible count (occupied/capacity), not positions —
  except Walrus placements, which block 1 space each and are tracked as
  distinct entries.

## Units

- [x] **U1 — skeleton, cards, state, setup.** Crate + `Config`
  (`num_players 2..=4`), card catalog with exact print counts from the
  spec, `State`/`Event` types, seeded setup (hands, face-up Currents,
  Final Round Deck 2×N, event cards, decks), `SurvivorDraft` phase,
  `initial_state`/`apply_event` fold discipline. Unit tests on setup
  invariants + draft. Commit: `247de1d`.
- [x] **U2 — core turn machine.** Phases 1–5 sans events/abilities:
  auto-add, draw-from-own-Current + finish, actions (play survivor, build
  modification incl. space rules, build extension, finish), discard-down,
  food confirm with hungry/stand choices, simultaneous pass, first-player
  rotation, final-round trigger + hope scoring + `end_game`. Small
  random-self-play soak (1k games, no panics, all games terminate).
  Commit: `5d7f5f7`.
 
- [x] **U3 — abilities & modifiers.** Stat aggregation from raft
  (add/draw/action/hand-size/food), Purser/First Mate/Athlete/Millionaire/
  Survivalist/Swimmer(+1 food)/Stowaway(no space)/Quarterdeck(net +1),
  special draw sources (Porter discard-top, Swimmer adjacent Current,
  Pirate chance steal), Fishing Rod/Telescope hand-size effects.
  Commit: `<pending>` (filled in below).
 
- [ ] **U4 — events & reactions.** Event deck + play-event action +
  targeting legality, Shark/Octopus/Walrus (+removal action)/Love Boat/
  Storm/Work Day/Land Sighting, Dead Fish + Fisher reaction windows,
  Telescope activation. Soak again with events on.
- [ ] **U5 — views, registry, agents, CLI.** `PublicView` redaction per
  spec + property test (view never contains hidden identities),
  registry integration (`greatgyre` game + `greedy-greatgyre` /
  `heuristic-greatgyre` linear eval: raft hope, buildable-hope potential,
  food-deficit penalty, resource/tempo terms), `api-schema` regen,
  10k-game `#[ignore]` soak, determinism byte-identity check,
  `heuristic beats random` sanity (>70% target).
- [ ] **U6 — docs.** README game table row, `docs/api-contract.md`
  greatgyre note (4-seat example), BENCHMARKS entry.
