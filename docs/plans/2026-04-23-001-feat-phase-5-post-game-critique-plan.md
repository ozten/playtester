---
title: "feat: Post-game LLM critique (Phase 5)"
type: feat
status: shipped
date: 2026-04-23
shipped: 2026-04-23
origin: playtest-roadmap.md § "Phase 5 — Post-game LLM critique"
supersedes_section: "Phase 5 (Post-game LLM critique) in the roadmap — this plan is the first plan of the re-ordered remaining sequence P5 → P6 → P4 → P8 (P7 dropped 2026-04-23)."
---

# feat: Post-game LLM critique (Phase 5)

> **Status: shipped.** Units 1–8 all landed. Automated coverage is
> stub-only (per Phase-3 precedent); the R5.9 exit-criterion
> benchmark is manual and documented in `docs/BENCHMARKS.md`.

## Overview

Every `LlmAgent` in a run answers a standardized questionnaire immediately after the game ends — 8–12 Likert items (agency, fairness, tension, pacing, variety, frustration, satisfaction, would-play-again) plus 2–3 open-ended prompts. Responses land in a new per-game `<gid>.critique.jsonl` sidecar, separate from the existing `<gid>.llm.jsonl` cost-observability sidecar. A follow-up `playtest critique-code` subcommand reads those sidecars and runs a coder LLM pass over the open-ended text, emitting structured tags of shape `{tag, severity, ref_card?}` into the same sidecar. The ingest pipeline loads both record kinds into two new SQLite tables (`critique_likert`, `critique_tags`), and the markdown reporter surfaces per-question means and tag-frequency histograms. The exit-criterion benchmark compares 100 ShipWreck games with Typhoon event cards enabled vs. disabled; the Likert "agency" score must differ measurably and open-ended tags must cluster coherently around the forced-sacrifice pain point.

This is the first phase in the re-ordered remaining sequence (**P5 → P6 → P4 → P8**; P7 dropped). It enables the comparative analysis in Phase 6 (critique deltas become one more metric family that `playtest compare` can diff) and leaves a clean seam for Phase 4 persona injection via a `system_prompt_addendum: Option<Arc<str>>` field on the critique config.

## Problem Frame

Phase 3 shipped the `LlmAgent` — an LLM can now play Cribbage and ShipWreck legally end-to-end. What the harness cannot yet extract from those games is **subjective feedback**. Win rates and pacing counters tell the designer *what happened*; they don't tell the designer what felt frustrating, tense, or unfair. The roadmap identifies post-game critique as "the distinctive capability that justifies using LLMs at all — without it, you built a worse version of SabberStone."

Three capability gaps remain before Phase 5 is satisfied:

1. **No questionnaire pipeline.** `LlmAgent` ends its contribution when `GameLoop::run` returns. There is no code path that asks the agent "how did that feel?" and stores the answer.
2. **No coding pass.** Free-text LLM responses are high-signal but structurally messy; the "agency Likert dropped because of Typhoon" conclusion depends on extracting `{complaint: "forced_sacrifice", card: "typhoon", severity: 3}` from prose. No such extractor exists today.
3. **No metrics surface for subjective data.** `playtest-metrics` aggregates numeric metrics from the main event log only. Likert scores and coded tags don't fit the existing `MetricValue` taxonomy and need new tables.

The roadmap's Phase 5 scope (8–12 Likert items, 2–3 open-ended prompts, coding pass, statistical separability for a frustrating card) is tight enough to land in one plan.

## Requirements Trace

Drawn from `playtest-roadmap.md` § "Phase 5" and the architectural invariants in `CLAUDE.md`.

- **R5.1** — Every `LlmAgent` seat answers a standardized post-game questionnaire when a game ends. Non-LLM agents (Random, Scripted, Greedy, Heuristic, ISMCTS, HTTP-remote, stdio) are skipped — they are not subjective reporters.
- **R5.2** — The questionnaire has 8–12 Likert (1–5) items and 2–3 open-ended prompts. The default item set is the roadmap's eight: agency, fairness, tension, pacing, variety, frustration, satisfaction, would_play_again. The default open-ended prompts are "worst moment" and "what would you change". The schema is a versioned, hashed static (SHA-256 in the sidecar header, same cache-stability pattern as `rules_text_sha256`).
- **R5.3** — Questionnaire responses are stored in a new per-game `<gid>.critique.jsonl` sidecar, one header line + one `questionnaire_response` record per LLM seat. The main JSONL event log is untouched (three-categories-of-recording discipline; see `docs/solutions/architecture-patterns/ephemeral-coordination-frame-vs-logged-event-2026-04-22.md`).
- **R5.4** — A `playtest critique-code <run-dir>` subcommand reads each game's critique sidecar, issues one coder-LLM call per `questionnaire_response` that has non-empty open-ended fields, and appends `coded_tag` records to the same sidecar. Re-running is idempotent (overwrites existing `coded_tag` records keyed by `(game_id, seat)`).
- **R5.5** — The ingest pipeline loads `.critique.jsonl` alongside the main log, populating new tables `critique_likert(game_id, seat, question, score)` and `critique_tags(game_id, seat, tag, severity, ref_card NULL-able)`. Idempotent via the existing `INSERT OR REPLACE` pattern.
- **R5.6** — The markdown reporter gains a "Subjective critique" section with (a) per-question Likert means + 95% CI, (b) tag-frequency histograms game-wide and per-card.
- **R5.7** — The main JSONL event log remains deterministic and replayable; critique records never appear in it. The determinism audit test is extended: `!log.contains("questionnaire_response")`, `!log.contains("coded_tag")`.
- **R5.8** — Persona seam: the critique prompt builder accepts a `system_prompt_addendum: Option<Arc<str>>` that Phase 4 will populate; Phase 5 always passes `None`.
- **R5.9** — **Exit criterion.** ShipWreck run with 100 games where Typhoon event cards are enabled (`ShipWreckConfig { events_enabled: true, .. }`) vs. 100 games where they are disabled (`events_enabled: false`). Likert "agency" mean differs by ≥ 0.5 on the 5-point scale with non-overlapping 95% CIs. Coded tags from the `events_enabled = true` run contain a dominant frustration cluster (≥ 25% of open-ended responses coded `{tag: "forced_sacrifice" or "random_loss", severity: ≥ 3}`). This is a **manual** benchmark (real LLM calls); automated tests use stubs to prove the pipeline end-to-end.

## Scope Boundaries

Everything below is explicitly deferred so Phase 5 stays focused on "extract subjective signal from a single game" — not "compare across runs," "inject personas," or "iterate prompts automatically."

- **No personas.** Phase 4. The questionnaire prompt builder exposes a `system_prompt_addendum` seam but the Phase 5 default is `None`.
- **No `playtest compare` subcommand or comparative statistics.** Phase 6. The exit-criterion benchmark runs two configs and eyeballs the delta manually; it does not ship a compare pipeline.
- **No prompt-engineering iteration framework.** Ship one questionnaire prompt and one coder prompt; tune by hand. Automated prompt optimization is Phase 5+ territory.
- **No multi-model critique ensembles.** Phase 9a. One model runs critique, one (possibly the same) runs the coder pass.
- **No card-generation proposals or counterfactual variants.** Phase 9c.
- **No auto-trigger on `playtest report`.** Report reads whatever ingest has loaded; critique-code stays an explicit operator step so coder-model / coder-prompt changes don't silently re-enter reports.

### Deferred to Separate Tasks

- **Persona-conditioned critique prompts** (Phase 4): separate plan. Wires `system_prompt_addendum` to a persona registry.
- **`playtest compare --critique` flag** (Phase 6): separate plan. Adds statistical significance testing over critique deltas.
- **UI dashboards for critique** (Phase 8): separate plan. The SvelteKit frontend gains a per-game Likert viewer and a tag-cluster explorer.
- **HTTP API surfacing critique** (post-Phase 5): exposing questionnaire responses via the `playtest-server` SSE/REST surface is out of scope. Localhost CLI only for this phase.

## Context & Research

### Relevant Code and Patterns

- **`LlmAgent<G>`** — `crates/playtest-agents/src/llm/agent.rs`. Owns `scratch: ScratchBuffer`, `cfg: LlmAgentConfig` with `llm: Arc<dyn LlmClient>`, `rules_text`, `card_catalog`, `sidecar`, `model`. Already holds every primitive needed for a post-game call: the LlmClient is live, the system blocks are byte-identical to what play used (prompt cache stays warm), and the sidecar handle is mutex-appendable. Adding `post_game_critique(&mut self, result, view, spec, critique_sidecar)` is a purely additive method.
- **`ScratchBuffer`** — `crates/playtest-agents/src/llm/scratch.rs`. `plan`, `notes`, `turn_log: VecDeque<String>` (capped at 64). The critique prompt embeds the final scratch in the user message so the model has its own running context, not just the public view.
- **`LlmSidecar`** — `crates/playtest-agents/src/llm/sidecar.rs`. Append-only JSONL with a `SidecarHeader` first line and `llm_call` records. The new `CritiqueSidecar` mirrors the shape (same `fs: Arc<Mutex<dyn FileSystem + Send>>` + `path` owner, same line-atomic append discipline, same cache-stability SHA-256 on the header).
- **Sidecar file naming** — `crates/playtest-registry/src/play.rs` builds `<run-dir>/games/<gid>.jsonl` (main log) and `<run-dir>/games/<gid>.llm.jsonl` (LLM cost). Adding `<run-dir>/games/<gid>.critique.jsonl` follows the established convention.
- **`LlmCliDeps` + `RunExtras`** — `crates/playtest-registry/src/play.rs`. Optional per-seat dependencies bundled for the registry dispatcher. Extending with `critique_sidecar: Option<Arc<CritiqueSidecar>>` + `questionnaire_spec: Option<Arc<QuestionnaireSpec>>` is purely additive; CLI passes `None` when critique is disabled.
- **`MetricRegistry<G>`** — `crates/playtest-metrics/src/registry.rs`. Pure function `(game_id, GameLog) → Vec<MetricValue>`. Critique does **not** fit this interface — it's a fact about agent *reports*, not game state. Rather than stretch `MetricValue` into a new kind, Phase 5 adds two dedicated SQLite tables that ingest populates directly from the critique sidecar. The `MetricRegistry` trait stays as-is; its invariants are unaffected.
- **Ingest pipeline** — `crates/playtest-metrics/src/ingest.rs`. Reads each main-log file, derives the game UUID from header stable fields, runs registries, `INSERT OR REPLACE`s. Phase 5 adds a second pass inside the same transaction: for each ingested game, look for `<gid>.critique.jsonl` and load Likert + tag records into the new tables.
- **Markdown reporter** — `crates/playtest-metrics/src/markdown.rs` + `reporter.rs`. Owns the current report section layout. Phase 5 adds a new section renderer that queries the two new tables via `crates/playtest-metrics/src/query.rs` helpers.
- **`ShipWreckConfig`** — `crates/games/shipwreck/src/config.rs`. Phase 5 adds an `events_enabled: bool` (default `true`) flag. When `false`, the setup step that seeds event cards into the wreckage pool is skipped. Determinism is preserved: the ChaCha seed still controls everything else; a run with `events_enabled: false` is a different config (different `config_hash`), not a different RNG arc.
- **Typhoon resolution** — `crates/games/shipwreck/src/events/typhoon.rs`. Multi-player forced-sacrifice event card; already implemented and spec-frustrating. No changes to the card itself — the exit-criterion benchmark simply toggles the pool seed.
- **CLI subcommand pattern** — `crates/playtest-cli/src/` (look for `play.rs`, `report.rs`, `replay.rs`). Each subcommand is a module with `cli_args` → registry dispatch → sink wiring. `critique-code.rs` follows the same shape: parse args, open each `.critique.jsonl` in the run dir, issue one coder LLM call per questionnaire-response with non-empty open-ended fields, append `coded_tag` records.

### Institutional Learnings

- **`docs/solutions/architecture-patterns/ephemeral-coordination-frame-vs-logged-event-2026-04-22.md`** — main JSONL log is a determinism contract; non-event data (coordination frames in that learning; critique records here) must not enter it. Phase 5's `!log.contains("questionnaire_response")` invariant is the same discipline re-applied.
- **`docs/solutions/architecture-patterns/sharing-mut-self-port-via-arc-mutex-2026-04-23.md`** — `Arc<Mutex<dyn FileSystem + Send>>` is the established pattern for shared-mutable port handles. `CritiqueSidecar` reuses it verbatim; the learning is already captured.
- **Phase 3 cache-stability discipline** — the rules block must be byte-identical across all calls in a game, or Anthropic's prefix cache misses. Phase 5's critique call uses the same `rules_text` and `card_catalog` as gameplay so the prefix remains cached; the per-turn scratch + questionnaire fit in the uncached user message. The `e2e_llm_stubbed.rs` test's cache-discipline assertion should extend to the critique call.

### External References

- **MeepleLM** (referenced in the roadmap's Phase 5 description) — pattern for delivering a post-game questionnaire to an LLM and aggregating Likert scores across plays. Not adopted wholesale; borrow the "Likert + open-ended + coded tags" pipeline shape.
- **Anthropic prompt caching docs** (already leveraged in Phase 3) — ~5-minute TTL; a critique call firing within a few seconds of the last gameplay call hits the cached prefix for free.
- **Slay the Spire CommunicationMod** — not relevant here (no subprocess critique path); noted only so reviewers don't wonder about the parallel.

## Key Technical Decisions

- **Critique lives as a post-game method on `LlmAgent`, not a separate `Critic` trait.** The LlmAgent already holds every primitive needed (live LlmClient, cached rules, scratch, sidecar). A parallel `Critic<G>` trait would duplicate registry plumbing and an `Arc<dyn LlmClient>` for marginal separation-of-concerns benefit. User confirmed during planning.
- **Coding pass runs offline via `playtest critique-code`, not inline at game end.** Parallels the existing `report`/`ingest` pattern: separate pass, re-runnable, decoupled from play-time config. Iterating the coder prompt does not require re-running games. User confirmed during planning.
- **New dedicated `<gid>.critique.jsonl` sidecar, not extension of `.llm.jsonl`.** Keeps cost-observability and product-signal streams cleanly separated. The files share owning crate (`playtest-agents::llm`) and append mechanism (`Arc<Mutex<dyn FileSystem + Send>>`); the split is semantic, not structural. User confirmed during planning.
- **Exit-criterion benchmark targets ShipWreck with Typhoon enabled/disabled.** Typhoon's forced-sacrifice semantic is plausibly frustrating without corrupting a published ruleset (Cribbage). `ShipWreckConfig::events_enabled` is the toggle; no new card is invented. User confirmed during planning.
- **Questionnaire schema is a versioned Rust static.** A `QuestionnaireSpec` struct with a `version: u16` + `items: Vec<QuestionItem>` + SHA-256 hash (stored in the sidecar header). This matches the `rules_text_sha256` discipline from Phase 3 and lets downstream consumers detect drift. No TOML/JSON loader — the schema is part of the binary, not configuration.
- **Coder output is a finite tag taxonomy, not free-form.** The coder prompt ships with a fixed list of ~20 candidate tags (`forced_sacrifice`, `turn_length`, `random_loss`, `snowball_win`, `snowball_loss`, `stalemate`, `lack_of_agency`, `satisfying_comeback`, `overwhelming_lead`, `unclear_rules`, `boring_early_game`, `tense_endgame`, …). The LLM chooses from the list; arbitrary tags are rejected. This is what makes "tags cluster coherently" an enforceable exit criterion instead of a fuzzy observation.
- **Critique is **gated off by default**, opt-in via CLI.** `playtest play --critique` flag enables questionnaire issue at game-end. The default is off because critique costs real LLM tokens even with cache; opt-in is the right trust model. The same applies to `critique-code` — always an explicit operator step.
- **`CriticqueSidecar::new` takes the questionnaire spec by value-and-hash.** The header stores `questionnaire_spec_sha256`; changing the default questionnaire between runs produces a different hash, which the markdown reporter surfaces as a cross-version warning (we will *not* mix different questionnaire versions' data in one aggregate).
- **Coder LLM budget is separate from play budget.** `LlmCliDeps` gains a `critique_budget_tokens: Option<u32>` field that the critique call and the coder pass share. This prevents a large batch run's critique pass from silently exhausting a play budget mid-game.
- **SSRF / provider policy applies uniformly.** The coder pass uses the same `ProductionLlmClient` (Anthropic or OpenAI-compat) with the same localhost-only SSRF guard. No new HTTP path to audit.

## Open Questions

### Resolved During Planning

- **Where does critique live architecturally?** → Critique-mode method on `LlmAgent`. (User selection.)
- **When does the coding pass run?** → Offline `playtest critique-code` subcommand. (User selection.)
- **Which game + card hosts the exit criterion?** → ShipWreck, existing Typhoon card, toggled via `events_enabled` config. (User selection.)
- **Storage layout?** → New `<gid>.critique.jsonl` sidecar, parallel to `.llm.jsonl`. (User selection.)
- **How do critique records reach SQLite?** → Ingest pipeline gains a second pass over `.critique.jsonl` inside the existing transaction; two new dedicated tables rather than stretching `MetricValue`. (Planning decision, see Key Technical Decisions.)
- **How is persona injection seamed in?** → `system_prompt_addendum: Option<Arc<str>>` on the critique config, defaulted to `None` in Phase 5. (See R5.8.)
- **Is critique always-on or opt-in?** → Opt-in via `--critique` flag on `playtest play`, because it costs real LLM tokens. (See Key Technical Decisions.)
- **Does the critique call use a fresh `LlmClient` or the gameplay one?** → Gameplay one, so prompt-cache stays warm. (See Key Technical Decisions.)
- **Is the coder's tag taxonomy fixed or free-form?** → Fixed list in the coder prompt; free-form tags rejected. (See Key Technical Decisions.)
- **Who owns the questionnaire schema?** → Hardcoded Rust static in `playtest-agents`, hashed into the sidecar header. (See Key Technical Decisions.)

### Deferred to Implementation

- **Exact tag taxonomy contents.** Unit 5 can iterate the list based on pilot critique outputs; the plan requires ~20 tags covering agency/pacing/variety clusters, but picking the exact strings belongs to the implementer iterating the coder prompt against real LLM output.
- **Exact default Likert items.** The roadmap lists eight; the final set (including whether to add `balance`, `novelty`, or `replayability`) is an Unit 1 decision, not a plan-level one. Lock the count between 8–12 per R5.2; lock the item semantics in code review.
- **Precise coder-prompt wording.** Start from the tag list + a few-shot examples; iterate until pilot coder outputs are deterministic and faithful. Unit 5 owns this.
- **Whether the coder pass warns or errors on tags the taxonomy rejects.** Start with warn + drop; upgrade to error only if drift becomes a problem in practice.
- **Exact statistical cut-off for R5.9.** The plan commits to "≥ 0.5 Likert delta, non-overlapping 95% CIs, ≥ 25% dominant tag frequency." Whether to tighten these once real numbers land is an Unit 8 decision.
- **Whether ShipWreck's `events_enabled` flag should default to `true` or `false` in `ShipWreckConfig::default()`.** Default `true` (preserves current behavior); exit-criterion benchmark explicitly flips it.

## High-Level Technical Design

> *This illustrates the intended data flow and is directional guidance for review, not implementation specification. The implementing agent should treat it as context, not code to reproduce.*

Critique lives on the tail end of a normal `playtest play` run, and the coder pass is an explicit separate pipeline. The shape:

```
  GAME TIME                                    POST-GAME CRITIQUE                  OFFLINE CODING PASS

  GameLoop::run ──► engine events ──► JSONL    for each LlmAgent seat:             playtest critique-code <run>
      │                                            post_game_critique(view, ..)
      │                                            ──► one LlmClient.complete()        for each <gid>.critique.jsonl:
      │                                            ──► QuestionnaireResponse              for each questionnaire_response
      ▼                                            ──► append to                            with non-empty open-ended:
  <gid>.jsonl                                         <gid>.critique.jsonl              ──► one LlmClient.complete()
  (main event log)                                                                         with coder prompt + taxonomy
                                                                                        ──► Vec<CodedTag>
                                                                                        ──► append to
                                                                                            <gid>.critique.jsonl
                                                                                            (same file, different record kind)

  PARALLEL SIDECAR                                                                    INGEST + REPORT (unchanged caller shape)

  <gid>.llm.jsonl                             <gid>.critique.jsonl                    playtest report <run>
  (llm_call records — Phase 3)                 header + questionnaire_response          ──► ingest reads .jsonl + .critique.jsonl
                                               + coded_tag (post-coder)                 ──► SQLite: games, agent_stats,
                                                                                                    game_metrics, critique_likert,
                                                                                                    critique_tags
                                                                                         ──► markdown reporter appends
                                                                                             "Subjective critique" section
```

Key invariants this diagram must preserve:

- The **main event log stream is untouched**. Questionnaire records and coded tags enter only via the new sidecar; the determinism audit tests assert this.
- The **critique LLM call reuses gameplay's cached system blocks** (same `rules_text` + `card_catalog`). Anthropic's ~5-minute cache TTL covers the sub-second gap between the last gameplay call and the questionnaire call.
- The **coder pass is re-runnable**. Appending a `coded_tag` record with `(game_id, seat)` already present replaces the previous coding; ingest picks up the latest via `INSERT OR REPLACE`.
- **Non-LLM seats are invisible to the critique pipeline**. A mixed `--agents llm,random` game produces exactly one `questionnaire_response` record.

Questionnaire-response JSON shape (coder input):

```json
{
  "kind": "questionnaire_response",
  "seat": 0,
  "spec_version": 1,
  "likert": {
    "agency": 4, "fairness": 5, "tension": 3, "pacing": 4,
    "variety": 3, "frustration": 2, "satisfaction": 4, "would_play_again": 4
  },
  "open_ended": {
    "worst_moment": "Losing my Steel Cordage upgrade to a typhoon on turn 6 felt arbitrary.",
    "what_would_you_change": "Maybe let players defend typhoons with a food sacrifice."
  }
}
```

Coded-tag JSON shape (reporter input):

```json
{ "kind": "coded_tag", "seat": 0, "tags": [
    { "tag": "forced_sacrifice", "severity": 3, "ref_card": "typhoon" },
    { "tag": "lack_of_agency",   "severity": 2, "ref_card": null }
]}
```

## Implementation Units

- [x] **Unit 1: `QuestionnaireSpec` + critique prompt builder**

**Goal:** Define the Likert-plus-open-ended schema, a hashable static default, and the prompt builder that produces a `LlmRequest` from the spec + the game's final public view + the agent's scratch buffer. No side effects yet — just the schema and the prompt.

**Requirements:** R5.1, R5.2, R5.8

**Dependencies:** None (Phase 3 shipped).

**Files:**
- Create: `crates/playtest-agents/src/llm/critique/mod.rs`
- Create: `crates/playtest-agents/src/llm/critique/spec.rs`
- Create: `crates/playtest-agents/src/llm/critique/prompt.rs`
- Test: `crates/playtest-agents/src/llm/critique/spec.rs` (unit tests in-module)
- Test: `crates/playtest-agents/src/llm/critique/prompt.rs` (unit tests in-module)

**Approach:**
- `QuestionnaireSpec { version: u16, items: Vec<QuestionItem>, open_ended: Vec<OpenEndedPrompt> }`. `QuestionItem { id: &'static str, text: &'static str, kind: LikertKind }`. `LikertKind::Scale1to5`.
- `const DEFAULT_QUESTIONNAIRE_V1: QuestionnaireSpec` with the roadmap's 8 items and 2 open-ended prompts.
- `impl QuestionnaireSpec { pub fn sha256(&self) -> String }` — serializes to canonical JSON, hashes via `sha2`. Stable across runs when the items don't change.
- `build_critique_user_message(view, result, scratch, spec, persona_addendum: Option<&str>) -> String` — embeds the final public view, the `GameResult` (winner + score), the scratch (`plan`, `notes`, `turn_log`), and the questionnaire items as a JSON-schema-style block the LLM must fill. Returns a single user-role message string; the system blocks (rules + card catalog) are reused unchanged from gameplay.
- `persona_addendum: None` for Phase 5; the parameter exists solely as the P4 seam.

**Patterns to follow:**
- `crates/playtest-agents/src/llm/prompt.rs` — `build_user_message` shape.
- `crates/playtest-agents/src/llm/sidecar.rs::sha256_hex` — the SHA-256 helper already exists; reuse.

**Test scenarios:**
- Happy path: `DEFAULT_QUESTIONNAIRE_V1.sha256()` is deterministic and non-empty (64 hex chars).
- Happy path: changing any item's text produces a different hash.
- Happy path: `build_critique_user_message` renders the expected keys (`agency`, `fairness`, …) and the two open-ended prompt labels.
- Happy path: `persona_addendum: Some("...")` is concatenated into the user message; `None` produces a message without the addendum fragment.
- Edge case: `turn_log` longer than 64 entries is truncated (re-use scratch semantics).
- Edge case: spec with fewer than 8 or more than 12 Likert items fails a `debug_assert` (invariant enforced at spec construction).

**Verification:**
- `DEFAULT_QUESTIONNAIRE_V1` lists 8–12 items, 2–3 open-ended prompts.
- `sha256()` is byte-stable across reruns of the same spec.
- Prompt rendering includes every item's `id` and every open-ended prompt's label.

---

- [x] **Unit 2: `CritiqueSidecar` — sidecar writer for `<gid>.critique.jsonl`**

**Goal:** The append-only JSONL sidecar for questionnaire and coded-tag records. Mirrors `LlmSidecar` structurally but is semantically separate: the header carries the questionnaire-spec hash, records are `questionnaire_response` and `coded_tag`, and the file path is `<run>/games/<gid>.critique.jsonl`.

**Requirements:** R5.3, R5.4

**Dependencies:** Unit 1 (for the `QuestionnaireSpec` hash that goes into the header).

**Files:**
- Create: `crates/playtest-agents/src/llm/critique/sidecar.rs`
- Modify: `crates/playtest-agents/src/llm/mod.rs` (re-export new types)
- Test: `crates/playtest-agents/src/llm/critique/sidecar.rs` (unit tests in-module)

**Approach:**
- `struct CritiqueSidecar { fs: Arc<Mutex<dyn FileSystem + Send>>, path: PathBuf }`.
- `struct CritiqueSidecarHeader { kind: "critique_sidecar_header", game: String, seed: u64, questionnaire_spec_sha256: String, rules_text_sha256: String }`.
- `struct QuestionnaireResponseRecord { kind: "questionnaire_response", seat: u8, spec_version: u16, likert: BTreeMap<String, u8>, open_ended: BTreeMap<String, String> }`.
- `struct CodedTagRecord { kind: "coded_tag", seat: u8, tags: Vec<CodedTag> }` + `struct CodedTag { tag: String, severity: u8, ref_card: Option<String> }`.
- `impl CritiqueSidecar { pub async fn new(fs, path, header) -> Result<Self, FsError>; pub async fn append_questionnaire(&self, rec: &QuestionnaireResponseRecord) -> Result<(), FsError>; pub async fn append_coded_tags(&self, rec: &CodedTagRecord) -> Result<(), FsError>; }`.
- `BTreeMap` (not `HashMap`) for Likert / open-ended fields so the JSON output is key-ordered — helps byte-level replay diffing.

**Patterns to follow:**
- `crates/playtest-agents/src/llm/sidecar.rs` — exact same mutex-serialized append pattern.
- `docs/solutions/architecture-patterns/sharing-mut-self-port-via-arc-mutex-2026-04-23.md` — the sharing discipline.

**Test scenarios:**
- Happy path: `CritiqueSidecar::new` writes the header as the first line with `kind: "critique_sidecar_header"`.
- Happy path: `append_questionnaire` produces one JSONL line with `kind: "questionnaire_response"` and a key-ordered Likert map.
- Happy path: `append_coded_tags` produces one JSONL line with `kind: "coded_tag"`.
- Edge case: concurrent `append_questionnaire` from two tasks produces two clean, non-interleaved lines (mutex guarantees line-atomicity).
- Edge case: appending `Likert` with an unknown key is accepted by the type system but the reporter will silently drop it at ingest (document in the struct's doc comment; no runtime enforcement in this unit).
- Integration (light): verify `StubFileSystem` output matches the expected byte layout for a two-seat run (header + 2 responses + 2 coded-tag records).

**Verification:**
- Files written by `CritiqueSidecar` parse back as line-delimited JSON with the documented record shapes.
- `kind` tags are distinct from the `llm_call` / `sidecar_header` kinds in the sibling `.llm.jsonl` file — no collisions if the two files were concatenated (they never are, but the kinds are orthogonal).

---

- [x] **Unit 3: `LlmAgent::post_game_critique` + replay safety**

**Goal:** The critique method on `LlmAgent`. Fires one `LlmClient.complete()` call using the agent's already-warm cached system blocks and the Unit 1 prompt builder, parses the JSON reply, appends a `questionnaire_response` record to the provided `CritiqueSidecar`. Replay must not re-call: the critique call uses the same `LlmClient` handle, so a `PlaybackLlmClient` tape replays deterministically; a production client skips the call when no critique sidecar is provided.

**Requirements:** R5.1, R5.3, R5.7, R5.8

**Dependencies:** Units 1 and 2.

**Files:**
- Create: `crates/playtest-agents/src/llm/critique/agent.rs`
- Modify: `crates/playtest-agents/src/llm/agent.rs` (add `post_game_critique` method delegating to `critique::agent`)
- Modify: `crates/playtest-agents/src/llm/mod.rs`
- Test: `crates/playtest-agents/tests/llm_agent_post_game_critique.rs`

**Approach:**
- New signature: `pub async fn post_game_critique<G: Game>(&mut self, view: &G::PublicView, result: &GameResult, spec: &QuestionnaireSpec, sidecar: &CritiqueSidecar, persona_addendum: Option<&str>) -> Result<(), AgentError>`.
- Reuses `self.cfg.llm`, `self.cfg.rules_text`, `self.cfg.card_catalog`, `self.cfg.model`. System blocks built from the same `build_system_blocks` function used in `choose`; user message built via Unit 1's `build_critique_user_message`.
- Parse reply as `QuestionnaireResponse` (Likert values must be 1–5, open-ended strings truncated to some sane cap, say 4 KB).
- On parse failure: one retry with a reminder prompt, then `AgentError::Other("critique parse failed: ...")`. Mirrors the gameplay retry discipline.
- On success: append `QuestionnaireResponseRecord` via `sidecar.append_questionnaire`. Scratch buffer is NOT mutated — critique has its own context window; bleeding it into scratch would corrupt future runs' replay if scratch ever became replay-visible.
- Budget: the critique call subtracts from the same `LlmClient` budget as gameplay. Exceeding budget surfaces as `LlmError::BudgetExceeded` → `AgentError::Other("critique budget exceeded")`. This is a log-and-skip condition at the dispatcher level, not a run-killing error.

**Patterns to follow:**
- `crates/playtest-agents/src/llm/agent.rs::choose` — same LlmClient call shape, same parse-with-one-retry, same error taxonomy.
- `crates/playtest-agents/tests/llm_agent_cribbage_stub.rs` — stub-based testing pattern.

**Test scenarios:**
- Happy path: stub LlmClient returns a well-formed questionnaire JSON; one record appears in the critique sidecar with the expected Likert values.
- Happy path: `persona_addendum: Some("You are aggressive")` is passed through to the prompt (assert via a capturing stub).
- Edge case: stub returns a Likert score outside 1–5 → parse fails → retry → second success is accepted OR final error is surfaced.
- Edge case: stub returns non-JSON → retry → second failure surfaces as `AgentError::Other` with `"critique parse failed"` substring.
- Error path: stub returns `LlmError::BudgetExceeded` → `AgentError::Other("critique budget exceeded")`; no record appears in the sidecar.
- Integration: replay with a `PlaybackLlmClient` tape that captured a prior critique call produces byte-identical records.
- Invariant: the main JSONL log contains no `questionnaire_response` record after a full game + critique run (guards R5.7).

**Verification:**
- `post_game_critique` only appends to the critique sidecar; never to the main event log, never to the `.llm.jsonl` sidecar (the gameplay `llm_call` record for that call is legitimate and expected in `.llm.jsonl`, but the questionnaire payload lives only in `.critique.jsonl`).
- A game-long LLM call sequence followed by a critique call shows one additional entry with `cache_read_input_tokens > 0` — proving the cached prefix stayed warm.

---

- [x] **Unit 4: Dispatcher wiring — `RunExtras::critique_deps`, `--critique` CLI flag**

**Goal:** Plumb the optional critique sidecar + spec through the existing `RunExtras` / `LlmCliDeps` chain, add a `--critique` flag to `playtest play`, and invoke `post_game_critique` on every LlmAgent seat after `GameLoop::run` returns.

**Requirements:** R5.1, R5.3

**Dependencies:** Unit 3.

**Files:**
- Modify: `crates/playtest-registry/src/play.rs` (extend `LlmCliDeps`, post-game iteration)
- Modify: `crates/playtest-cli/src/play.rs` (new flag, builder wiring)
- Modify: `crates/playtest-agents/src/llm/agent.rs` (expose any internal state the dispatcher needs to identify LlmAgent seats — likely via a `Box<dyn PostGameCritic>` trait object or a typed registry-side helper)
- Test: `crates/playtest-cli/tests/e2e_critique_stubbed.rs` (new)

**Approach:**
- Extend `LlmCliDeps` with `critique_spec: Option<Arc<QuestionnaireSpec>>` and `critique_sidecar: Option<Arc<CritiqueSidecar>>`. Existing callers pass `None`, keeping the full call chain backward compatible.
- Extend `RunExtras` with a convenience method `with_critique(spec, sidecar)`.
- Post-`GameLoop::run`: iterate seats; for each seat whose agent was built from the `llm` kind, downcast (via a registry-side handle — not runtime type reflection; probably a `Vec<Option<Box<dyn PostGameCritic>>>` built alongside the `Box<dyn Agent<G>>` vector during registry dispatch) and call `post_game_critique(view, result, spec, sidecar, None)`.
- The downcast hack: registry returns `(Box<dyn Agent<G>>, Option<Box<dyn PostGameCritic>>)` per seat. `PostGameCritic` is a new tiny trait with one method: `async fn post_game_critique(&mut self, view: &G::PublicView, result: &GameResult, spec: &QuestionnaireSpec, sidecar: &CritiqueSidecar) -> Result<(), AgentError>`. Only `LlmAgent<G>` implements it; every other agent returns `None`.
- CLI: `--critique` flag enables the feature. When set: build a `CritiqueSidecar` per game under `<run>/games/<gid>.critique.jsonl`, attach via `RunExtras::with_critique`. Questionnaire defaults to `DEFAULT_QUESTIONNAIRE_V1`.
- On critique failure (budget, parse error): log to stderr, continue. The main game run is already complete at this point; critique is advisory.

**Patterns to follow:**
- `crates/playtest-registry/src/play.rs::run_single_game_into_sink_with_extras` — the existing extras-threading pattern.
- `crates/playtest-cli/src/play.rs` — the `LlmCliDeps` builder pattern.
- `crates/playtest-cli/tests/e2e_llm_stubbed.rs` — end-to-end stubbed test pattern.

**Test scenarios:**
- Happy path: `playtest play --agents llm,llm --critique ...` with a stub LlmClient produces a `.critique.jsonl` with exactly 2 `questionnaire_response` records.
- Happy path: `playtest play --agents llm,random --critique ...` produces exactly 1 `questionnaire_response` record (random seat skipped).
- Happy path: running without `--critique` produces no `.critique.jsonl` file.
- Edge case: stub LlmClient returns a parse failure for one seat but success for the other — the good seat's record lands, the bad seat logs and is absent from the sidecar; the game run still exits 0.
- Edge case: budget exhaustion on seat 0's critique — seat 0 absent, seat 1 either absent (budget already gone) or present (budget replenished per-call — consistent with production semantics).
- Integration: determinism audit — `!log.contains("questionnaire_response")` on the main JSONL log (R5.7).

**Verification:**
- Default behavior (no `--critique` flag) is byte-identical to Phase 3 output.
- Mixed-agent games produce exactly one questionnaire record per LlmAgent seat.
- Critique failures never fail the `playtest play` run's exit code.

---

- [x] **Unit 5: Coder prompt + `playtest critique-code` subcommand**

**Goal:** The offline coder pass that reads each `<gid>.critique.jsonl`, sends one LlmClient call per questionnaire-response with non-empty open-ended answers, parses a `Vec<CodedTag>`, and appends a `coded_tag` record. Idempotent: re-running rewrites tags for `(game_id, seat)` pairs already coded.

**Requirements:** R5.4

**Dependencies:** Unit 2.

**Files:**
- Create: `crates/playtest-agents/src/llm/critique/coder.rs`
- Create: `crates/playtest-cli/src/critique_code.rs`
- Modify: `crates/playtest-cli/src/main.rs` (register subcommand)
- Test: `crates/playtest-agents/tests/coder_prompt.rs`
- Test: `crates/playtest-cli/tests/e2e_critique_code_stubbed.rs`

**Approach:**
- `pub const CODER_TAG_TAXONOMY: &[&str]` — ~20 tag strings covering agency, pacing, variety, frustration, balance clusters. Lock the exact list during Unit 5 implementation.
- `build_coder_prompt(response: &QuestionnaireResponseRecord, taxonomy: &[&str]) -> LlmRequest` — system block contains the tag definitions + one-or-two few-shot examples; user message contains the open-ended text.
- `parse_coder_reply(text: &str, taxonomy: &[&str]) -> Result<Vec<CodedTag>, CoderError>` — expects a strict JSON array of `{tag, severity: 1–5, ref_card?: String}`; rejects tags outside the taxonomy (warn-and-drop; log the dropped tag to stderr).
- `playtest critique-code <run-dir> --model M --provider P [--coder-budget-tokens N]` — opens each `.critique.jsonl`, reads all records, for every `questionnaire_response` with non-empty open-ended fields that lacks a matching `coded_tag` record (or, if `--overwrite` is set, for every `questionnaire_response`), builds the coder prompt, issues the call, appends a `coded_tag` record.
- Idempotency: "lacks a matching `coded_tag`" is determined by reading every `coded_tag` record in the file and checking whether seat N already has one. This is O(n) per file; n is small (≤ player count). If present and `--overwrite` is not set, skip.
- On rewrite (`--overwrite`): append a new `coded_tag` record; the most recent one wins during ingest (ingest does `INSERT OR REPLACE`).

**Patterns to follow:**
- `crates/playtest-cli/src/report.rs` — directory-scanning + per-file processing + error-tolerant operation.
- `crates/playtest-agents/src/llm/prompt.rs` — system/user prompt construction.

**Test scenarios:**
- Happy path: stub LlmClient returns well-formed tags; `coded_tag` record appears with the expected `{tag, severity, ref_card}` shape.
- Happy path: running without `--overwrite` on a file that already has a `coded_tag` for seat 0 leaves seat 0 alone and codes seats 1+.
- Happy path: running with `--overwrite` appends a fresh `coded_tag` record regardless.
- Edge case: questionnaire-response has empty open-ended fields → skipped (no coder call made); no `coded_tag` record.
- Edge case: coder returns a tag outside the taxonomy → warn, drop; `coded_tag` contains the valid subset.
- Edge case: coder returns non-JSON → one retry → still fails → log per-game, continue with next game.
- Error path: budget exhausted mid-run → log; remaining files skipped; exit 0.

**Verification:**
- `playtest critique-code` is idempotent: running twice with no `--overwrite` has no net effect on coded-tag records.
- Subcommand exits 0 even when some games fail to code (parallels the ingest tolerance).

---

- [x] **Unit 6: SQLite schema + ingest extension**

**Goal:** Two new tables (`critique_likert`, `critique_tags`). The existing ingest pipeline gains a second pass that reads `.critique.jsonl` for each game and populates both tables inside the same transaction.

**Requirements:** R5.5, R5.7

**Dependencies:** Units 2 and 5.

**Files:**
- Modify: `crates/playtest-metrics/src/schema.sql`
- Modify: `crates/playtest-metrics/src/ingest.rs`
- Modify: `crates/playtest-metrics/src/query.rs` (new query helpers for the reporter)
- Test: `crates/playtest-metrics/tests/critique_ingest.rs`

**Approach:**
- Schema:
  ```
  critique_likert (
      game_id    TEXT NOT NULL REFERENCES games(id) ON DELETE CASCADE,
      seat       INTEGER NOT NULL,
      question   TEXT NOT NULL,
      score      INTEGER NOT NULL CHECK (score BETWEEN 1 AND 5),
      spec_version INTEGER NOT NULL,
      PRIMARY KEY (game_id, seat, question)
  ) STRICT;

  critique_tags (
      game_id    TEXT NOT NULL REFERENCES games(id) ON DELETE CASCADE,
      seat       INTEGER NOT NULL,
      tag        TEXT NOT NULL,
      severity   INTEGER NOT NULL CHECK (severity BETWEEN 1 AND 5),
      ref_card   TEXT,                          -- NULL when no card is blamed
      PRIMARY KEY (game_id, seat, tag, COALESCE(ref_card, ''))
  ) STRICT;
  ```
  Indexes on `critique_likert(question)` and `critique_tags(tag)`.
- Ingest extension: after the main log is loaded for a game, look for `<gid>.critique.jsonl`. Parse line-by-line; for each `questionnaire_response`, `INSERT OR REPLACE` into `critique_likert` (one row per Likert question). For each `coded_tag` record's inner `tags`, `INSERT OR REPLACE` into `critique_tags`.
- Multiple `coded_tag` records for the same seat (from re-runs with `--overwrite`): the latest one in the file wins because both `INSERT OR REPLACE` into the same PK.
- Missing sidecar: skipped silently; critique is optional per game.
- Idempotency: ingesting the same directory twice produces byte-identical SQLite rows.

**Patterns to follow:**
- `crates/playtest-metrics/src/ingest.rs` — the transaction-bounded insert pattern.
- `crates/playtest-metrics/src/schema.sql` — the `STRICT` + `INSERT OR REPLACE` discipline.

**Test scenarios:**
- Happy path: a game with a 2-seat `.critique.jsonl` (both questionnaire-responses and both coded-tags) produces 16 `critique_likert` rows (2 seats × 8 default items) and N `critique_tags` rows.
- Happy path: re-ingesting the same directory produces the same row count (no duplicates).
- Happy path: ingesting a game without a critique sidecar completes cleanly; the tables remain empty for that game.
- Edge case: a malformed `.critique.jsonl` line is tolerated (skipped, reported in `IngestReport`).
- Edge case: `coded_tag` with `ref_card: null` round-trips into the SQLite NULL column.
- Error path: a `coded_tag` with severity outside 1–5 is rejected by the `CHECK` constraint; `IngestReport` notes the failure; ingest continues.

**Verification:**
- `critique_likert` and `critique_tags` are populated idempotently.
- `games`, `agent_stats`, `game_metrics` rows are unchanged by the critique extension.
- `!main_log.contains("questionnaire_response")` and `!main_log.contains("coded_tag")` (R5.7).

---

- [x] **Unit 7: Markdown reporter — "Subjective critique" section**

**Goal:** Extend the markdown reporter to include per-question Likert means + 95% CI across all critiqued games, and tag-frequency histograms (game-wide + per `ref_card`). The section appears only when `critique_likert` or `critique_tags` is non-empty; otherwise omitted cleanly.

**Requirements:** R5.6

**Dependencies:** Unit 6.

**Files:**
- Modify: `crates/playtest-metrics/src/markdown.rs`
- Modify: `crates/playtest-metrics/src/reporter.rs`
- Modify: `crates/playtest-metrics/src/query.rs`
- Test: `crates/playtest-metrics/tests/critique_report.rs`

**Approach:**
- Query helpers: `critique_likert_means(&Connection) -> Vec<(question, mean, ci_lower, ci_upper, n)>` and `critique_tag_counts(&Connection, scope: TagScope) -> Vec<(tag, ref_card_opt, count, share)>`.
- `TagScope` enum: `Overall` and `PerCard` (the latter groups by `ref_card` so the reporter can render per-card histograms).
- 95% CI: normal approximation (score ∈ {1..5}); `n` = rows per question. With fewer than 5 rows per question, CI is omitted (reporter shows `—` instead of a number).
- Markdown section title: `## Subjective critique`. Subsections: `### Likert means` (table: question | mean | 95% CI | n) and `### Coded tags` (overall table + one table per `ref_card` that has ≥ 3 mentions).
- When both tables are empty (no critique data in the run), the entire section is omitted — no empty heading, no "no data" placeholder.
- When the run mixes critique versions (multiple distinct `spec_version` values), the report renders a warning banner at the top of the Subjective critique section: `**Warning:** this run mixes questionnaire specs v1 and v2; Likert aggregates may not be comparable.`

**Patterns to follow:**
- `crates/playtest-metrics/src/markdown.rs` — existing table-rendering helpers.
- `crates/playtest-metrics/src/reporter.rs` — section ordering and section-omission conventions.

**Test scenarios:**
- Happy path: 100 critiqued games with a clear "agency" distribution produce a Likert table with expected means and non-zero widths on 95% CIs.
- Happy path: tag histogram renders a top-5 table ordered by frequency descending.
- Happy path: per-card table appears for every `ref_card` with ≥ 3 coded-tag mentions; no table for single-mention cards.
- Edge case: a run with no critique sidecars at all omits the entire "Subjective critique" section (not an empty heading).
- Edge case: a run with some `coded_tag` records but zero `questionnaire_response` (unusual but possible if someone edits a sidecar) renders only the tag-histogram subsection.
- Edge case: mixed `spec_version` produces the warning banner.

**Verification:**
- Markdown output is deterministic and stable byte-order across re-runs of `playtest report`.
- Existing report sections are not rearranged or altered.

---

- [x] **Unit 8: ShipWreck `events_enabled` config + exit-criterion benchmark**

**Goal:** Add a runtime toggle to ShipWreck that omits event cards from the wreckage pool; ship an `#[ignore]`'d benchmark that drives 100 games with and without Typhoon, asserts the Likert-agency delta, and documents the manual recipe in `docs/BENCHMARKS.md`.

**Requirements:** R5.9

**Dependencies:** Units 3 and 6 (critique must land; ingest must populate tables).

**Files:**
- Modify: `crates/games/shipwreck/src/config.rs`
- Modify: `crates/games/shipwreck/src/setup.rs` (skip event-card seeding when `events_enabled: false`)
- Modify: `crates/games/shipwreck/src/determinize.rs` (ensure `determinize` invariant holds under both settings)
- Create: `crates/playtest-cli/tests/r5_9_shipwreck_typhoon_benchmark.rs` (`#[ignore]` soak-style test)
- Modify: `docs/BENCHMARKS.md` (add R5.9 section with manual recipe)

**Approach:**
- `ShipWreckConfig { events_enabled: bool, /* existing fields */ }`. Default `true` (preserves current Phase 2 behavior).
- Setup: when `events_enabled: false`, the event-card indices are excluded from the shuffled wreckage pool. Everything else (player cards, base rafts, food/resource items) is unchanged.
- Determinize audit: add a property test that `determinize` under `events_enabled: false` produces public views byte-identical to the public views the engine produces naturally. Prevents a silent drift if the setup change accidentally leaks hidden state.
- `#[ignore]` benchmark test uses a stub LlmClient that produces *plausible* critique output deterministically (seed-varied Likert + open-ended text keyed to whether Typhoon was played in that game). Asserts: when `events_enabled: true`, mean `agency` ≤ X; when `false`, mean `agency` ≥ Y; |delta| ≥ 0.5. Asserts tag-cluster: `forced_sacrifice` frequency in `true` run ≥ 25%, in `false` run ≤ 5%.
- `docs/BENCHMARKS.md` R5.9: manual recipe using real Haiku with `playtest play --game shipwreck --agents llm,llm --critique --games 100 --config events_enabled=false --out ./target/playtest-runs/r5-9-baseline`, then the same with `events_enabled=true`, then `playtest critique-code` on each, then `playtest report` on each, then manual inspection of the two reports' Subjective critique sections. Expected cost: ~$4–$10 for both 100-game runs combined at Haiku pricing.

**Patterns to follow:**
- `docs/BENCHMARKS.md` R3.8/R3.9 — manual recipe template.
- Existing `#[ignore]` soak tests like `random_self_play` and `ismcts_beats_heuristic` — same release-mode discipline.

**Test scenarios:**
- Happy path (stubbed benchmark, `#[ignore]`): 100 games `events_enabled: true` vs. 100 `events_enabled: false` produce measurable Likert-agency delta with non-overlapping 95% CIs under the stub's deterministic-output policy.
- Happy path (stubbed benchmark): `forced_sacrifice` tag frequency differs ≥ 20 percentage points between the two configs.
- Determinism: `determinize` invariant (`public_view(determinize(s, p, rng), p) == public_view(s, p)`) holds for both `events_enabled` values.
- Setup invariant: the wreckage pool size and composition are correct in both configs (event-card count = 0 when disabled).

**Verification:**
- `cargo test --release --test r5_9_shipwreck_typhoon_benchmark -- --ignored` passes.
- `docs/BENCHMARKS.md` R5.9 section lists the exact CLI incantation for the manual run.
- `cargo test --release --workspace` (non-ignored) still passes — the benchmark is `#[ignore]`'d.

## System-Wide Impact

- **Interaction graph:** `playtest play` grows a post-run critique pass (new, optional); `playtest critique-code` is a new subcommand; `playtest report` transparently reads critique data when present. `playtest-server` and the HTTP API are **untouched** — critique is a CLI-only concern in Phase 5.
- **Error propagation:** Critique failures at game-end log to stderr and do not fail the game run. Coder-pass failures log per-game and do not fail the subcommand. Ingest failures on malformed `.critique.jsonl` lines are tolerated and reported in `IngestReport`, matching the existing discipline for malformed main-log lines.
- **State lifecycle risks:** The `CritiqueSidecar` uses the same `Arc<Mutex<dyn FileSystem + Send>>` pattern as `LlmSidecar`; no new concurrency invariant. Replay of a game with a captured `LlmClient` tape includes the critique call's bytes — determinism is preserved.
- **API surface parity:** None. The new surface is CLI + on-disk. `playtest-api` and the SSE stream are unchanged.
- **Integration coverage:** Stub-driven e2e in `crates/playtest-cli/tests/e2e_critique_stubbed.rs` (Unit 4) and `e2e_critique_code_stubbed.rs` (Unit 5). No real LLM calls in CI; the exit-criterion real-API run is manual per `docs/BENCHMARKS.md` R5.9.
- **Unchanged invariants:**
  - Main JSONL log stays a determinism contract — critique records never enter it (R5.7; audit test extended).
  - `Game` trait shape unchanged — critique is an agent-side concern, not a game-side one.
  - `MetricRegistry<G>` trait unchanged — critique tables are populated directly by ingest, not via registries.
  - `playtest-server` remains game-agnostic — critique code is not imported there.
  - Phase 3's R3.6 "main log is replay-from-seed deterministic, LLM not consulted during replay" invariant extends naturally: replay also does not consult the critique LLM.

## Risks & Dependencies

| Risk | Mitigation |
|---|---|
| Critique LLM call cost dominates a large batch run | Opt-in via `--critique`; per-seat budget separate from play budget; `LlmError::BudgetExceeded` logs and skips rather than failing the run. |
| Coder prompt produces inconsistent tag choices across model versions | Fixed taxonomy + warn-and-drop on out-of-taxonomy tags; `spec_version` and coder-model string recorded alongside tags (extend the `CodedTag` record if needed) so report aggregates don't silently mix coder regimes. |
| `post_game_critique` mutates state that future replays need | Scratch buffer is explicitly not touched in Unit 3; sidecar is append-only; main log is untouched. Replay determinism audit test extended. |
| Typhoon toggle leaks hidden state via `determinize` | Unit 8 adds a property test asserting `determinize` invariance under both config values. |
| Sidecar file grows unbounded for long-lived runs | One critique record per LlmAgent seat per game; for 10K games with 2 LLM seats that's 20K records — trivial. No mitigation needed. |
| Per-card coded-tag histograms attribute blame to cards that weren't the actual cause (LLM hallucination) | Accept in Phase 5; Phase 6's `playtest compare` + restricted-play analysis is where card-level attribution gets rigor. Report the histograms with a prominent "as-coded-by-LLM" caveat. |
| Anthropic cache TTL (~5 minutes) expires between last play call and critique call on very long games | Accept — the critique call still works, just without cache benefits on the very largest games. Cost impact is bounded (one extra seeding of the rules prefix per game). |

## Documentation / Operational Notes

- `docs/BENCHMARKS.md` gains an R5.9 section (manual recipe).
- The sidecar layout gets a brief mention in `CLAUDE.md`'s architecture invariants list (the "three categories of recording" discipline is now four surfaces on disk but the same three categories).
- `docs/api-contract.md` is not updated — critique is CLI-only in Phase 5.

## Sources & References

- **Origin document:** `playtest-roadmap.md` § "Phase 5 — Post-game LLM critique"
- **Shipped foundation:** `docs/plans/2026-04-22-002-feat-stdio-protocol-and-llm-agent-plan.md` (LlmAgent, LlmClient ports, sidecar plumbing)
- **Architecture discipline:** `docs/solutions/architecture-patterns/ephemeral-coordination-frame-vs-logged-event-2026-04-22.md`
- **Sharing discipline:** `docs/solutions/architecture-patterns/sharing-mut-self-port-via-arc-mutex-2026-04-23.md`
- **Project invariants:** `CLAUDE.md` (hexagonal architecture, determinism, three-categories-of-recording)
- **ShipWreck spec:** `docs/shipwreck.md` (Typhoon event-card semantics)
- **External pattern:** MeepleLM — post-game questionnaire + Likert aggregation
