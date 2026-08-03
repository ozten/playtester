# The Great Gyre — rules specification (engine-facing)

Source of truth: `GreatGyre_Instructions_v6.pdf` (rulebook) + the printed
card decks (April 2026 print files, extracted at
`/home/admin/gyre/design/`). Where the rulebook and a printed card
disagree, **the printed card wins** (the web UI shows card scans).
Assumptions the source material leaves open are marked **[A]** and were
decided during engine design; they are part of this spec.

The existing `shipwreck` crate implements an older prototype sketch
(`docs/shipwreck.md`) and is a *different game*. `greatgyre` is a new,
separate game crate implementing this spec.

## Overview

- 2–4 players **[A** — the rulebook never states a count; component limits
  support up to 4 comfortably and the harness targets 4**]**.
- Each player runs a raft: `Raft Left` + `Raft Right` base cards (never
  destroyed/discarded) with survivor/modification cards on **spaces**
  between them. Base raft provides 2 spaces; each built Raft Extension
  adds 2 more.
- Base raft stats: Add 1 card to Current / turn, Draw 1 card from Current
  / turn, 1 Action / turn, Max Hand Size 3, +1 Food (from Raft Right).
- Win: most **Hope** (sum of face-up hope icons on your raft, plus end-game
  bonuses) after the Final Round. Ties: **all tied players win.**

## Zones

- **Deep Sea Deck** (face-down draw pile): the shuffled Main Deck
  (survivors not chosen + modifications + Dead Fish) **plus all Resource
  cards** **[A** — the rulebook never says resources are shuffled in, but
  the game is unplayable otherwise; the print files separate them only for
  manufacturing**]**, minus starting hands, minus starting Currents, minus
  the Final Round Deck.
- **Final Round Deck**: 2 × num_players cards set aside face-down at setup.
- **Event Deck** (face-down) + one Event card dealt to each player at setup.
- **Discard Pile** (face-up, shared): spent resources, resolved events,
  shark-eaten survivors. Top card is public; Porter may draw from it.
- **Per-player Current**: pile in front of each player. Cards are
  individually face-up or face-down. Setup deals 3 face-up; Phase-1 adds
  are face-down; hand-limit discards go here **face-down [A]**.
  Face-down Current cards are hidden from *everyone* until drawn (the
  owner effectively sees them only as draw options during their Phase 2).
- **Raft Extension pile** (shared, finite). Octopus-discarded extensions
  return to this pile **[A]**.
- **Hand** (hidden). Event cards in hand do not count toward Max Hand Size.

## Round structure

Each round: every player, in seat order starting with the First Player,
plays Phases 1–4; then Phase 5 happens once, simultaneously; the First
Player card passes one seat in its direction and a new round begins.
Phase 5 is skipped in the Final Round.

### Phase 1 — Add Cards to the Current (automatic)

The active player draws `1 + add_bonus` cards from the Deep Sea Deck and
places them face-down on their own Current. No decisions; the engine
auto-executes. `add_bonus`: Sail (+1 each), Millionaire (+1).
If the Deep Sea Deck empties during anyone's Phase 1, remaining adds come
from the Final Round Deck and the **Final Round is triggered** (this round
becomes the last; see End of Game).

### Phase 2 — Draw Cards from the Current

The active player may draw up to `1 + draw_bonus` cards into hand
(`draw_bonus`: Net +1 each, Athlete +1). Each draw is a choice of source:

- any card of their own Current (face-up or face-down; face-down identity
  is revealed to the drawer on selection),
- **Porter**: the top card of the Discard Pile,
- **Swimmer**: any card of an adjacent player's Current (face-down picks
  are blind — choose by position),
- **Pirate**: a uniformly-random card from another player's hand
  (chance-resolved).

Each special source can substitute for any of the player's draws
**[A** — cards say "instead of the Current"; each named ability is usable
at most once per Phase 2 **[A]****]**. The player may stop early
(`finish_drawing`).

### Phase 3 — Take Actions

`1 + action_bonus` actions (`action_bonus`: Toolkit +1, First Mate +1).
Work Day makes actions (and extra Phase-3 draws) unlimited this turn.
Available actions:

- **Play a Survivor** from hand onto a free space (Stowaway needs no
  space; Quarterdeck's spaces hold survivors/modifications like any other).
- **Build a Modification** from hand: pay its resource cost from hand to
  the Discard Pile. If the card shows the space icon it occupies a space
  (Quarterdeck occupies 1 and *provides* 2); Net, Telescope, Fishing Rod
  and Dead Fish-in-hand do not occupy spaces. Dead Fish is never "built" —
  it is a hand-held reaction card.
- **Build a Raft Extension**: pay 1 Wood + 1 Rope + 1 Plastic from hand;
  take an extension from the pile (if any remain); +2 spaces.
- **Play an Event card** from hand; resolve, then discard it (Walrus stays
  in play; Land Sighting is never "played" — it scores from hand at end).
- **Activate Telescope** (if built): discard the Telescope, draw 1 Event
  card into hand (playable a future turn) **[card text; overrides the
  rulebook's peek/reorder version]**.
- **Remove Walrus**: discard a Dead Fish from hand to discard a Walrus
  occupying one of your spaces **[A** — per Walrus card text; costs an
  action **[A]****]**.
- Survivors/modifications may **not** otherwise be rearranged or removed
  (free rearrangement exists physically but has no gameplay effect in this
  model since spaces are fungible; the engine tracks slot occupancy as a
  count, not positions).

After actions, the player must discard down to Max Hand Size
(base 3, +1 per survivor hand-tab: most survivors +1, Purser +2;
Fishing Rod −1, Telescope −1 while built; Events exempt). Discards go
face-down to the player's own Current.

### Phase 4 — Confirm Food Supply (mostly automatic)

`food = Σ` food icons on face-up raft cards (Raft Right +1; survivors
−1 each except Survivalist 0 and Swimmer +1; Garden/Rain Trap +2 each,
Net +1, Fishing Rod +2). Hungry (sideways) survivors' stats still count
**[A]**.

- `food < 0`: the player chooses |food| of their standing survivors to
  turn Hungry (sideways). If fewer standing survivors exist than needed,
  the shortfall is covered by returning Hungry survivors to the player's
  Current (owner's choice of which) **face-up [A]**.
- `food ≥ 0`: the player stands up up to `food` Hungry survivors
  (auto-stand all if `food ≥` hungry count; otherwise owner chooses).

### Phase 5 — Pass Cards in Current (automatic, simultaneous)

All players simultaneously pass their entire Current (preserving face-up /
face-down state) to the neighbor in the First Player card's direction
(**left [A** — direction fixed; the physical card indicates it**]**). The
First Player card then passes one seat in the same direction. Skipped in
the Final Round.

## Events

| Card | Effect |
|---|---|
| **Shark Attack!** | Choose another player with ≥ 2 survivors on their raft; they discard 1 survivor of their choice to the Discard Pile. |
| **Octopus Attack!** | Choose another player with ≥ 1 raft extension; they choose 1 extension to lose (back to the extension pile). Cards on the lost spaces relocate to free spaces if available, else go to that player's Current face-up **[A]**. |
| **Walrus!** | Place on any open space of another player's raft; that space is blocked until its owner discards a Dead Fish (reaction at play time negates outright; later removal costs an action). |
| **Love Boat** | Requires a free space on your raft; choose another player with ≥ 2 survivors **[card says "more than 1" [A]]**; they choose one of their survivors to give you; place it on your raft. |
| **Storm!** | Starting with your down-current neighbor and proceeding around (excluding you), each other player chooses: remove 1 survivor-or-modification from their raft to their Current face-up **[A]**, or discard 2 hand cards (to their Current, face-down **[A]**; if they hold < 2 non-event cards they discard what they can **[A]**). |
| **Work Day** | This turn: unlimited actions, and drawing from your own Current becomes a free repeatable action. Discard after resolving. |
| **Land Sighting** | Never played. If in hand at game end: +15 Hope. |

**Reactions** (interrupt when an attack targets you):
- **Dead Fish** (from hand): negate a Shark Attack / Walrus / Octopus
  Attack played against you; the event card is still discarded.
- **Fisher** (on your raft): discard any 1 hand card to negate a Shark or
  Octopus Attack against you.
If both are available the target picks (or declines and takes the effect).

## Survivors (1 copy each; chosen or drawn)

| Survivor | Hope | Hand | Food | Other | Ability |
|---|---|---|---|---|---|
| Captain | 20 | +1 | −1 | | (his hope *is* the bonus) |
| Purser | 5 | +2 | −1 | | +2 hand size instead of +1 |
| Fisher | 10 | +1 | −1 | | reaction: discard 1 card to negate Shark/Octopus on you |
| First Mate | 10 | +1 | −1 | +1 action | |
| Survivalist | 10 | +1 | 0 | | needs no food |
| Influencer | 0 | +1 | −1 | | game end: every *other* player −5 Hope |
| Stowaway | 10 | +1 | −1 | | occupies no space |
| Pirate | 10 | +1 | −1 | draw* | may draw a random card from another player's hand |
| Millionaire | 5 | +1 | −1 | +1 add | +1 card added to Current per turn |
| Porter | 10 | +1 | −1 | draw* | may draw top of Discard Pile |
| Athlete | 0 | +1 | −1 | +1 draw | extra draw from Current |
| Swimmer | 10 | +1 | +1 | draw* | may draw from an adjacent player's Current |

(`draw*` = alternative draw source, not an extra draw. Every survivor also
implicitly grants +1 Max Hand Size via its hand tab except where shown.)

## Modifications

| Card | Copies | Cost | Space? | Effect | Hope |
|---|---|---|---|---|---|
| Quarterdeck | 3 | 2 Wood | occupies 1, provides 2 | net +1 space | 5 |
| Sail | 3 | 1 Wood + 1 Rope | yes | +1 add to Current | 10 |
| Garden | 3 | 1 Wood + 1 Plastic | yes | +2 Food | 10 |
| Rain Trap | 3 | 1 Plastic + 1 Rope | yes | +2 Food | 10 |
| Toolkit | 1 | 1 W + 1 R + 1 P | yes | +1 Action | 5 |
| Net | 6 | 2 Rope | no | +1 Draw, +1 Food | 0 |
| Fishing Rod | 2 | 1 Rope + 1 Wood | no | −1 Hand size, +2 Food | 5 |
| Telescope | 2 | 2 Plastic | no | −1 Hand size while built; activate: discard → draw 1 Event card | 5 |
| Dead Fish | 2 | — (hand-only) | never placed | reaction: negate Shark/Walrus/Octopus vs you | — |

## Deck composition

- Main Deck (37): 12 survivors ×1 + Net×6 + Quarterdeck×3 + Sail×3 +
  Garden×3 + Rain Trap×3 + Fishing Rod×2 + Telescope×2 + Toolkit×1 +
  Dead Fish×2.
- Event Deck (10): Shark Attack×2, Storm×2, Work Day×2, Land Sighting×1,
  Octopus Attack×1, Walrus×1, Love Boat×1.
- Resource Deck (77): Rope×31, Wood×25, Plastic×21.
- Raft Deck: 5 × (Left+Right) pairs, 10 Raft Extensions (shared pile).

Setup: each player takes Left+Right raft cards and **chooses** a survivor
(the UI lets the human pick; AI seats pick at random **[A]**); remaining
survivors are shuffled with the rest of the Main Deck and all resources;
deal 3 to each hand; deal 3 face-up to each Current; set aside
2×players as the Final Round Deck; the rest is the Deep Sea Deck. Deal 1
Event card to each player.

## End of Game

Triggered when the Deep Sea Deck empties during a Phase 1 (top-up from the
Final Round Deck). The current round becomes the Final Round: it completes
for all players but Phase 5 is skipped **[rulebook]**. Then scoring:

`score = Σ hope icons on face-up raft cards (survivors + modifications +
raft cards) + 15 if Land Sighting in hand − 5 × (each other player's
Influencer on their raft)`

Hungry (sideways) survivors still count their hope **[A]**. Highest score
wins; all tied players win (`winners: Vec<PlayerId>`).

## Hidden information & views

`PublicView` for observer P: own hand (full), own Current (full),
opponents' hands (counts only), any Current's face-up cards (identity) and
face-down cards (count only — including one's own, until drawn), Deep
Sea / Final Round / Event deck counts, Discard Pile (full, ordered — top
is drawable), all rafts (fully public), First Player + direction, phase,
pending decision, current player, hungry markers, walrus placements.

Note: the server's SSE/JSONL stream is complete-information (engine
invariant); per-viewer redaction is the web proxy's job.
