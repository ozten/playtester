# Card Game Playtesting CLI — Product Roadmap

A phased plan for building a Rust CLI that uses LLMs as player agents to playtest card-focused board games. Each phase has a clear "what you can actually learn from a game design after this ships" definition of done, so you can stop at any phase and still have a useful tool.

## Guiding principles

**Deterministic Rust engine is non-negotiable infrastructure.** Every later phase depends on it, and retrofitting determinism is painful. Build it once, well, before anything else.

**Each phase must independently produce actionable design insight.** No phase is "just plumbing." If phase N ships and phase N+1 never does, the designer should still get real value from phase N.

**Bias toward cheap agents early, expensive agents late.** A random agent in phase 1 finds real bugs. Opus-4.7 playing in phase 6 finds subtle balance issues. Don't invert this — you'll burn money proving things a scripted bot could've told you for free.

**LLMs never adjudicate rules.** This is the single most important architectural rule, carried across every phase. The engine is authoritative; agents propose, engine disposes.

**ROI ranking convention used below:** ★ = nice to have, ★★ = clearly worth it, ★★★ = disproportionate payoff relative to build cost, ★★★★ = the unfair-advantage phases that justify the whole project.

> **2026-04-23 re-ordering.** Phases 0–3 (plus Phase 2.5 HTTP remote agent) are shipped. The remaining build order is now **P5 → P6 → P4 → P8**. Phase 7 (MAP-Elites deckbuilding) is **dropped** — both shipped games (Cribbage, ShipWreck) lack a deck-construction mechanic, so the behavioral-descriptor space degenerates; re-introduce only if a deckbuilding game joins the harness. Phase numbers below are kept as-is to preserve references from shipped plans and commits.

---

## Phase 0 — Engine foundations (2–3 weeks)

**Goal.** A deterministic Rust game engine that can play a game end-to-end against itself with random agents, with full replay and inspection.

**Build.** Effect DSL as data (`deal_damage`, `draw`, `destroy`, etc.) with an interpreter. Legal-move enumeration. Seeded RNG for all stochasticity. Structured event log per turn. JSON snapshot of game state at any point. `cargo test` suite for rule correctness. A `replay` subcommand that replays any past game tick-by-tick.

**Agent interface.** Narrow trait: given a game state and a list of legal actions, return one. First implementation: `RandomAgent`. Second: `ScriptedAgent` that takes a priority list ("play highest-cost creature if possible; otherwise cycle"). That's it.

**Insight produced.** Rule bugs. Crashes. Cards that don't terminate. Games that never end. Degenerate states where no legal move exists. This is unglamorous and will find 20+ real issues on any non-trivial game.

**Implementation difficulty.** Medium. The effect DSL is the hard part — get it right and everything downstream is easy; get it wrong and you'll be fighting it in phase 4.

**ROI.** ★★★★ — Without this, nothing else works. With this alone, you have a better testbed than most indie card games ever get.

**Exit criteria.** Can run 10,000 self-play games in under 60 seconds on one core. Every game produces a complete, replayable log. Zero panics over a 100K-game soak test.

---

## Phase 1 — Metrics and analytics spine (1–2 weeks)

**Goal.** Turn raw game logs into the numbers designers actually use.

**Build.** Per-card metrics: inclusion rate, win rate when included, win rate when drawn by turn N, play rate when drawn, mulligan keep rate. Per-deck metrics: win rate, avg game length, avg turn-of-concede. Per-game metrics: length, lead changes, decision count. Archetype clustering (HDBSCAN over deck co-occurrence). Shannon entropy over archetype distribution as a single meta-health scalar. Export to DuckDB or Parquet — don't invent a storage format. A `report` subcommand that produces a markdown summary of a batch of games.

**Insight produced.** With random agents only, this already surfaces: cards that are never playable (zero play rate), cards that always win when drawn (suspiciously high win rate), games that never terminate (length distribution tail), and archetype clustering that validates or refutes the designer's intended archetypes.

**Implementation difficulty.** Low. DuckDB does the heavy lifting; write SQL and Rust glue.

**ROI.** ★★★★ — Every later phase produces data that flows through this layer. Building it now means every agent improvement in later phases immediately produces richer reports.

**Exit criteria.** Running `playtest report --games 10000 --deck-pool standard.json` produces a readable markdown report in under 30 seconds.

---

## Phase 2 — Competent heuristic agents (2–3 weeks)

**Goal.** Agents that play the game the way a mediocre but non-stupid human would, fast enough to run millions of games cheaply.

**Build.** Hand-written heuristic agent with a small evaluation function (board state, tempo, card advantage, life total). Greedy-lookahead variant that scores each legal action one ply deep. **ISMCTS** (Information Set MCTS, Cowling/Powley/Whitehouse 2012) as the strong baseline — handles hidden information properly via determinization. Parameterize by time-per-decision or iterations. All agents share the same trait from Phase 0.

**Insight produced.** *This is where the first real balance insights appear.* With ISMCTS on both sides playing 10K games per deck matchup, you'll see: overpowered cards (win rate > 60%), dead cards (play rate < 5%), dominant openings, and degenerate loops. The "obviously overpowered card" acceptance criterion is satisfied at this phase — no LLM required.

**Implementation difficulty.** Medium-high. ISMCTS is well-documented but subtly tricky — chance nodes, determinization correctness, and tree-reuse across turns all have gotchas. Budget a full week for tuning.

**ROI.** ★★★★ — This phase is honestly where 80% of the value lives for the *mechanical balance* use case. If you only built through Phase 2, you'd have a professional-quality card balance tool.

**Exit criteria.** ISMCTS beats the heuristic agent >65% and the heuristic beats random >90%. 10K-game matchup matrix for a 20-deck pool runs in under 30 minutes on a laptop.

---

## Phase 3 — Stdio agent protocol and LLM integration (1–2 weeks)

**Goal.** External processes can play the game over a stable protocol. LLMs become one of many possible agents.

**Build.** JSON-over-stdio protocol (CommunicationMod pattern from Slay the Spire is the proven template). Per-turn message contains: public state, this player's private state, legal actions indexed 0..N, and the player's scratch buffer. Agent replies with an action index, an updated scratch buffer, and optional rationale text. A reference Python client. A first `LLMAgent` in Rust that calls Anthropic's API. Aggressive **prompt caching** of rules text and card catalog — this is cost-critical.

**The scratch buffer split from the earlier conversation goes here.** Three slots: `plan` (rewritten occasionally), `notes` (updated on surprise), and `turn_log` (auto-appended by engine). LLM writes the first two; engine owns the third.

**Cost guardrails.** Per-game token budget cap. A cheap model (Haiku-class) as default. A `--model` flag to opt into expensive runs. Log every LLM call with token counts so cost is observable, not inferred.

**Insight produced.** Not much new balance insight yet — phase 2 already found the mechanical issues. What this phase *unlocks* is phase 4+. Useful intermediate output: you can now watch an LLM play your game and see what it finds confusing from the rationales.

**Implementation difficulty.** Medium. Protocol design is easy; getting the prompt right is the hard part and benefits from iteration.

**ROI.** ★★★ — Pure enabling phase. Its value is entirely through the phases it unlocks. Do not let scope creep inflate this phase.

**Exit criteria.** A Haiku-class LLM plays a full game legally, end-to-end, with scratch buffer updates, for under $0.20. A local Llama-class model plays legally (slower, free).

---

## Phase 4 — Procedural personas (2 weeks)

**Goal.** Agents that play the game in *different* ways, not just the best way. This is where the insight quality makes a step change.

**Build.** Persona as a data file: utility weights (aggression, card advantage, tempo, board presence, life preservation), a description string, optionally a skill level. Apply personas two ways: (a) weighted evaluation function for heuristic/MCTS agents, (b) system-prompt injection for LLM agents. Ship 5–8 default personas: *aggro-beatdown, control-grinder, combo-hunter, midrange-pragmatist, noob-deals-damage, mulligan-perfectionist, hand-hoarder, timmy-plays-big-things*. Per-persona win rate reporting in phase 1's analytics layer.

**Insight produced.** This is where the frustrating-mechanic acceptance criterion starts getting traction. A card with 52% aggregate win rate but 78% against aggro-beatdown and 31% against control-grinder tells a real design story that aggregate data hid. This is precisely the pattern that hid Oko, Companions, and Skullclamp in MTG — balanced on average, broken against specific strategies.

**Implementation difficulty.** Medium. The utility-function math is trivial; designing good personas is the craft, and you'll iterate on them for the life of the project.

**ROI.** ★★★★ — Disproportionate insight per line of code. Also, personas make every later phase better: LLM critique conditioned on persona is wildly more useful than "generic LLM opinion."

**Exit criteria.** Per-persona win rate breakdowns appear in every report. At least one card's imbalance is visible only in per-persona data and not in aggregate data.

---

## Phase 5 — Post-game LLM critique (1–2 weeks)

**Goal.** Structured subjective feedback from LLM agents about what they just played, aggregable across thousands of games.

**Build.** Standardized post-game questionnaire delivered to each LLM player: 8–12 Likert-scale questions (agency, fairness, tension, pacing, variety, frustration, satisfaction, would-play-again) plus 2–3 open-ended prompts ("what was the worst moment?", "what would you change?"). Answers stored alongside the game log. A separate **coding pass** over open-ended responses using a different (or same) LLM to extract tags: `{complaint: "turn_length", card: "fireball", severity: 3}`. Persona-conditioned prompts — the aggro persona's opinions about a card differ from the control persona's, and you want both.

**Insight produced.** The "frustrating mechanic surfaces as complaints" acceptance criterion is satisfied here. Aggregate Likert scores for "agency" drop to 2.3/5 specifically in games featuring the stun-lock card — that's the signal. Tag-frequency tables from open-ended responses cluster around real pain points.

**Implementation difficulty.** Low-medium. The technical work is easy; prompt design for the questionnaire is where the quality lives. Steal liberally from the MeepleLM paper's approach.

**ROI.** ★★★★ — This is the distinctive capability that justifies using LLMs at all. Without it, you built a worse version of SabberStone. With it, you have something studios don't have off-the-shelf.

**Exit criteria.** For a deliberately frustrating card (stun-all-opponent-creatures-for-3-turns, say), the Likert "agency" score is statistically distinguishable from baseline with 100 games, and open-ended tags cluster coherently around the right complaint.

---

## Phase 6 — Comparative and counterfactual analysis (2 weeks)

**Goal.** Answer "is this change an improvement?" rather than just "what does this game look like?"

**Build.** `playtest compare --baseline game-v1.toml --variant game-v2.toml --games 10000` runs both, diffs all metrics, flags statistically significant changes. **Restricted-play analysis** (Jaffe et al. 2012 style): measure each card's contribution to win rate by comparing games with and without it available. Bradley-Terry ratings over decks. Matchup-matrix deltas. Automatic generation of a "what changed" markdown report highlighting regressions.

**Insight produced.** This is the capability that makes the tool usable in an iteration loop. Designer changes a card's cost from 4 to 3; the tool tells them win rate moved from 48% to 61%, the card is now 95th-percentile in inclusion, and aggro-beatdown's win rate against control-grinder jumped 7 points. This is the shape of feedback a designer actually wants.

**Implementation difficulty.** Medium. Statistics is the interesting part — you need proper significance testing with multiple-comparison correction or you'll drown in false positives.

**ROI.** ★★★★ — Turns the tool from "produces reports" to "guides iteration."

**Exit criteria.** A deliberately buffed card produces a flagged regression in under 10K games with appropriate confidence intervals. A cosmetic change (renaming a card) produces no flagged regressions.

---

## Phase 7 — Quality-diversity deckbuilding — **DROPPED (2026-04-23)**

Removed from the roadmap. MAP-Elites over deck space requires a deck-construction mechanic to produce non-degenerate behavioral descriptors. Cribbage has a fixed 52-card deck; ShipWreck uses a shared wreckage draft with no construction phase. With no deckbuilding game in the harness, the phase's signal collapses. Re-introduce only if a third game crate with a deck-construction mechanic lands.

---

## Phase 8 — Interactive designer loop (2 weeks)

**Goal.** Make the tool pleasant enough to use every day.

**Build.** Web UI or TUI showing live matchup grids, per-card dashboards, game replays with turn-by-turn state inspection, LLM rationale viewer, Likert score distributions. `playtest watch` that re-runs analysis when card files change. Export to Notion/Obsidian/whatever the designer uses. Diff visualizations for phase 6 comparisons.

**Insight produced.** None directly — this is pure UX. But *usage frequency* goes up 10× with a good interface, and 10× usage on a tool that produces real insight is itself a step-change in design quality.

**Implementation difficulty.** Medium. Web UI work is time-consuming but well-understood.

**ROI.** ★★★ — Multiplies the value of everything earlier. Skip it and the tool gets used once a month. Build it and it gets used daily.

**Exit criteria.** A designer with zero tool training can open the UI, find the three most-changed cards in the latest build, and watch a replay of one of their games, in under 5 minutes.

---

## Phase 9 — Expensive optional capabilities

These are all "maybe, if the project is still alive and the ROI math holds up":

**9a. Multi-model ensemble play.** Run games with Opus, Sonnet, Haiku, and a local model, compare their critique. Expensive but reveals model-specific biases in critique.

**9b. CFR for combo validation.** When phase 2/4 find a suspicious combo, run CFR on a restricted game to prove/disprove it's actually game-theoretically dominant. High skill barrier; use only for the hardest balance questions.

**9c. Counterfactual card generation.** LLM proposes replacement cards when one is flagged as overpowered, then the tool auto-tests them. Closes the design loop but risks generic outputs.

**9d. Human-calibrated LLM personas.** Fine-tune small open models on actual human play logs from a beta to produce more realistic personas. Expensive; only worth it once the game has human players.

**9e. Tabletop Simulator export.** Generate a TTS mod from the game definition so human playtesters can play the same ruleset the bots do. High utility once you're ready for humans in the loop.

**ROI.** Each is ★★ to ★★★ depending on the game. Evaluate phase-by-phase.

---

## Sequencing summary and stopping points

| After phase | What you have | Who it's useful for |
|-------------|---------------|---------------------|
| 0 | A correctness-verified game engine with random play | You alone, debugging rules |
| 1 | Data-driven reports on game state | A solo designer iterating on mechanics |
| 2 | Mechanical balance detection at publication quality | Any serious card game project |
| 3 | LLM agents can play the game | (enabling — stop here only if you run out of runway) |
| 4 | Per-persona balance insights | Games with archetype variety |
| 5 | Subjective feedback extraction | Games where fun > optimization |
| 6 | Closed-loop iteration tool | Active game development |
| ~~7~~ | ~~Automated exploit discovery~~ | **Dropped 2026-04-23** — no deckbuilding game to target |
| 8 | Daily-driver UX | Sustained multi-designer teams |
| 9 | Specialized power tools | Mature projects with specific needs |

**The honest advice:** phases 0–2 are table stakes and should be non-negotiable. Phase 4 is where this project becomes *interestingly* better than prior art. Phase 5 is where it becomes unique. Phase 6 is where it earns its keep in a design process. Everything after is optimization.

If runway is tight, ship 0–2 first as a standalone "card balance bench," then come back for the LLM phases (3–5) once the engine has proven its worth.

## Rough cost and time estimate

Assuming a solo technical founder at full focus:

- **Phases 0–2:** ~6–8 weeks, compute cost negligible (local CPU)
- **Phases 3–5:** ~4–6 weeks, compute cost ~$50–$500 per game-iteration cycle depending on model choice
- **Phases 6–8:** ~6–8 weeks, compute cost scales with iteration frequency
- **Phase 9:** open-ended

Total to "useful daily driver": **4–5 months** of focused work, roughly **$500–$5,000** in LLM spend during development (dominated by prompt tuning, not final runs). Ongoing cost per full game-evaluation cycle once built: **$5–$50** for 10K games with a mix of cheap and expensive agents, which is cheap enough to run on every meaningful design change.
