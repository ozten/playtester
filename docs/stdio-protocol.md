# Playtester stdio agent protocol

Authoritative wire reference for the Phase 3 stdio agent protocol: the
transport between a running `playtest` engine and a user-owned
subprocess that plays one seat of a game.

Companion document to [`docs/api-contract.md`](api-contract.md). The
HTTP+SSE contract covers browser-driven remote agents; this document
covers subprocess-driven agents reached through a pipe.

If something is documented here but does not match
`crates/playtest-agents/src/remote/stdio/protocol.rs`, the Rust source
is source of truth — this doc is drifted; please fix.

Audience: someone writing a program in any language that wants to
play a Playtester game driven by the Rust engine.

## Overview

The stdio protocol is **line-delimited JSON over a subprocess's stdin
and stdout**. Every frame is a single JSON object followed by exactly
one `\n`. There is no length prefix, no content-type header, no
envelope — just the raw frame.

Exactly two flow directions:

- **Engine → child** (agent → subprocess): exactly one `turn` frame
  per turn, written to the child's stdin.
- **Child → engine** (subprocess → agent): exactly one `action` or
  `error` frame per turn, written to the child's stdout.

The engine uses this protocol when the CLI is invoked with a `stdio`
agent kind, for example:

```sh
playtest play --game cribbage \
  --agents stdio,random \
  --stdio-cmd /usr/bin/python3 \
  --stdio-arg /path/to/agent.py \
  --seed 42 --games 1 --out ./runs/
```

The `stdio` agent kind is **CLI-only**. `POST /api/runs` rejects any
request whose `agents:` array contains `"stdio"` with error code
`AgentKindNotAllowedHere` — arbitrary command execution over HTTP is
not in the localhost trust model. See `docs/api-contract.md` for the
HTTP rejection shape.

### Trust model

Spawning a `stdio` agent is equivalent to arbitrary command execution
under the invoking user's UID. `--stdio-cmd /usr/bin/env` will run
`/usr/bin/env` exactly as typed. This is intentional under the CLI
trust model: the user controls the CLI and the command. No
allowlisting, no sandboxing, no path filtering. The Rust side does
strip `ANTHROPIC_API_KEY` and `PLAYTEST_OPENAI_COMPAT_KEY` from the
child's environment before spawn — the child has no business with the
parent's LLM credentials — but everything else is inherited.

## Protocol version

```
3.0.0
```

Lives as the string constant `STDIO_API_VERSION` in
`crates/playtest-agents/src/remote/stdio/protocol.rs` and is sent on
every `turn` frame as the `api_version` field. There is **no
separate handshake frame** — version negotiation happens implicitly
on the first turn. A child that does not recognize the version should
emit an `error` frame; the engine maps that to `AgentError::Other` and
fails the game.

Semver discipline matches the HTTP API:

- Major bump — breaking change to frame shapes (field removed, type
  changed, meaning changed).
- Minor bump — additive changes (new optional fields, new reply-frame
  kinds). Clients tolerant of unknown fields continue to work.
- Patch bump — wording-only tweaks to error strings.

`3.0.0` is the first published version. There is no compatibility
layer — the engine speaks exactly one version at a time.

## Transport

- **One line of JSON per frame.** A frame is a JSON object followed
  by a single `\n`. The child must not write a frame in chunks — the
  engine reads whole lines and parses each as a self-contained frame.
- **Non-JSON lines are tolerated up to a small cap.** The engine
  discards up to `MAX_GARBAGE_LINES = 16` non-parseable lines before
  erroring with `TooManyGarbageLines`. This makes the protocol
  human-debuggable: a stray `print()` in a debug build will usually
  survive, but a fundamentally broken child surfaces quickly.
- **stderr is inherited** by the engine's stderr. Use it freely for
  debugging — it does not interact with framing. Do **not** use
  stdout for anything except frames.
- **stdin closes** when the game ends. The child should treat EOF on
  stdin as the signal to exit cleanly. If the child does not exit,
  the engine's `kill_on_drop(true)` wrapper fires when the agent is
  dropped.
- **The child is spawned lazily.** The engine does not spawn the
  subprocess until the first `choose()` call. `--stdio-cmd` existence
  is validated at agent-construction time (fast-fail on typo), but no
  process runs until a real turn needs one.

## Frame shapes

All field names are `snake_case`. The schemas are written to match
the Rust structs in `crates/playtest-agents/src/remote/stdio/protocol.rs`
exactly. Where the Rust type carries a game-generic payload (e.g.
`V = PublicView`, `A = Action`), the schema below notes
"game-specific JSON".

### `turn` frame (engine → child)

```json
{
  "kind": "turn",
  "api_version": "3.0.0",
  "game": "cribbage",
  "seat": 0,
  "prompt_id": 17,
  "view": { /* game-specific PublicView JSON */ },
  "legal_actions": [ /* array of game-specific Action JSON */ ],
  "scratch": {
    "plan": "keep runs and pairs",
    "notes": "dealer next hand",
    "turn_log": [
      "tick=0 seat=0 stdio_chose index=1",
      "tick=3 seat=0 stdio_chose index=0"
    ]
  }
}
```

Fields, in the order the Rust struct declares them:

| Field | Type | Notes |
|-------|------|-------|
| `kind` | string literal `"turn"` | Discriminator. |
| `api_version` | string | Always the current `STDIO_API_VERSION` (`"3.0.0"`). |
| `game` | string | Game name, e.g. `"cribbage"`, `"shipwreck"`. Lets the child dispatch rules/view schemas. |
| `seat` | integer (u8) | 0-based seat index this child is playing. |
| `prompt_id` | integer (u64) | Monotonically increasing per-agent. The child **must** echo this in its reply. |
| `view` | object | The game's `PublicView` JSON. Redacted to what this seat can see. |
| `legal_actions` | array | The full enumerated legal-action slice for this turn. The child's `action_index` is an index into this array. |
| `scratch` | object | The agent's persistent memory — see `ScratchBuffer` below. |

#### `ScratchBuffer` inside `turn`

| Field | Type | Notes |
|-------|------|-------|
| `plan` | string | Free-form strategic plan. Round-trips: the child's reply `scratch.plan` becomes the next turn's `scratch.plan`. |
| `notes` | string | Free-form tactical notes. Round-trips the same way. |
| `turn_log` | array of string | Bounded rolling window of past turn summaries (cap `MAX_TURN_LOG = 64`). Maintained by the Rust agent — each turn the agent acts, it appends one line. The child cannot write `turn_log`; the reply frame does not carry it. |

### `action` frame (child → engine)

```json
{
  "kind": "action",
  "prompt_id": 17,
  "action_index": 2,
  "scratch": {
    "plan": "keep runs and pairs",
    "notes": "dealer next hand"
  }
}
```

| Field | Type | Notes |
|-------|------|-------|
| `kind` | string literal `"action"` | Discriminator (tag on the `ReplyFrame` enum). |
| `prompt_id` | integer (u64) | **Must** equal the `prompt_id` from the `turn` frame being replied to. Mismatches fail the turn with `PromptIdMismatch`. |
| `action_index` | integer (usize) | Index into `turn.legal_actions`. Must be in `0..len(legal_actions)`. |
| `scratch` | object (optional) | Defaults to `{ "plan": "", "notes": "" }` if omitted (via `#[serde(default)]`). Only `plan` and `notes` are accepted — the agent manages `turn_log` itself. |

### `error` frame (child → engine)

```json
{
  "kind": "error",
  "prompt_id": 17,
  "message": "unable to decide: model returned an unparseable reply"
}
```

| Field | Type | Notes |
|-------|------|-------|
| `kind` | string literal `"error"` | Discriminator. |
| `prompt_id` | integer (u64) | Echoed for correlation. Defaults to `0` if the child has no prompt id to echo (e.g. a frame before any turn was received). |
| `message` | string | Human-readable explanation. Mapped to `AgentError::Other("child replied with error frame: <message>")` on the Rust side. |

Use `error` frames for child-side failures that should fail the game
cleanly rather than hang. Examples: invalid JSON in the `turn` frame,
an LLM the child was delegating to timing out, an internal assertion.

## Handshake

There is none. Every `turn` frame carries `api_version` and `game`;
the first turn is the handshake. A child that needs to validate the
version or the game can inspect the first frame it reads and emit an
`error` frame if it does not support it — no `hello` / `ready`
round-trip.

This keeps the protocol a single frame-pair instead of two, and makes
the version-mismatch failure mode the same as any other protocol
error: a failed turn, surfaced as `AgentError::Other`.

## Lifecycle

### Spawn (lazy)

The engine does not spawn the subprocess during agent construction.
`StdioAgentConfig::validate()` checks that `--stdio-cmd` resolves to
a filesystem entry, but that is it. The real spawn happens in the
first `choose()` call, inside the tokio current-thread runtime the
game loop runs on. This resolves the
`tokio::process::Command`-needs-a-runtime constraint without forcing
the CLI path to build a runtime early.

At spawn:

- `stdin` is `Stdio::piped()`.
- `stdout` is `Stdio::piped()`.
- `stderr` is `Stdio::inherit()` — debug output from the child surfaces
  in the terminal.
- `kill_on_drop(true)` is set — the child is reaped when the agent is
  dropped.
- Environment is inherited with `ANTHROPIC_API_KEY` and
  `PLAYTEST_OPENAI_COMPAT_KEY` removed via `env_remove`.

### Per-turn round-trip

For each turn the seat needs to act on:

1. Engine builds a `TurnFrame`, serializes to JSON, writes one line
   to the child's stdin, and flushes.
2. Engine calls `read_line` on the child's stdout, discarding up to
   `MAX_GARBAGE_LINES = 16` non-JSON lines. The first parseable
   `ReplyFrame` is taken.
3. Engine validates `prompt_id` and (on `action`) `action_index`.
4. Engine updates its `ScratchBuffer` from the reply's `plan` and
   `notes`, appends one line to `turn_log`, and returns the action
   index to the game loop.

If `len(legal_actions) == 1`, the engine short-circuits and does not
talk to the child at all — the forced index is `0`. The child sees no
turn frame for that turn. Do not assume a 1:1 correspondence between
turns in your log and turn frames received.

### Shutdown

At game end, the engine drops the `StdioAgent`. Rust's drop order is
declaration order, and the `ChildHandle` struct is deliberately
ordered `stdin, stdout, child` so that:

1. `stdin` closes first. The child sees EOF on its read side and can
   exit cleanly.
2. `stdout` closes.
3. `child` drops. `kill_on_drop(true)` fires if the child is still
   alive, reaping any hanger.

There is no explicit graceful-shutdown timeout. A well-behaved child
sees EOF and exits; a hung child gets killed.

### Environment scrubbing

Before spawn, the engine removes `ANTHROPIC_API_KEY` and
`PLAYTEST_OPENAI_COMPAT_KEY` from the child's environment via
`Command::env_remove`. These are the two variables the in-Rust LLM
provider adapters read; the engine does not want user-authored
subprocesses to have opportunistic access to them. Everything else
about the parent environment is inherited.

## Error handling

Every error on the Rust side is a `StdioProtocolError` variant,
mapped to `AgentError::Other(<variant Display string>)` at the
boundary. In full (from
`crates/playtest-agents/src/remote/stdio/agent.rs`):

| Variant | Triggered when | What a child should do to avoid it |
|---------|----------------|------------------------------------|
| `SpawnFailed(String)` | `Command::spawn` itself failed — permission denied, bad exec format. | Make `--stdio-cmd` executable (`chmod +x`). Pass an interpreter as `--stdio-cmd` and the script as `--stdio-arg` if the script lacks a valid shebang. |
| `CommandNotFound(PathBuf)` | The path given to `--stdio-cmd` does not exist at agent-construction time. | Pass an **absolute path**. `command -v python3` is the idiomatic resolver. |
| `ProtocolVersionMismatch` | Reserved for future use. Phase 3 surfaces version problems via child-emitted `error` frames. | Inspect `api_version` on the first `turn` frame and emit an `error` frame if you don't support it. |
| `ParseError(String)` | The child's stdout produced bytes that looked like JSON but did not match `ReplyFrame`. | Only write two frame kinds: `action` or `error`. Include all required fields. |
| `PromptIdMismatch { expected, got }` | The reply's `prompt_id` does not equal the turn's. | Always echo the exact `prompt_id` the engine sent. Do not cache and reorder turns. |
| `IndexOutOfRange { got, legal_len }` | `action_index` is outside `0..legal_actions.len()`. | Validate before emitting. `0 <= action_index < len(legal_actions)`. |
| `ChildExited` | The child's stdout closed mid-turn (read returned 0 bytes), or stdin writes received `BrokenPipe`. | Do not exit until stdin closes (EOF). Handle SIGPIPE. |
| `Io(String)` | Any other I/O failure talking to the child. | Usually indicates the child was killed out-of-band. |
| `TooManyGarbageLines(usize)` | More than 16 non-JSON lines read before a valid frame. | Do not write anything except frames to stdout. Use stderr for debug. |
| `ChildError(String)` | The child emitted a structurally valid `error` frame. | Use `error` frames intentionally when your own logic fails — they surface as clean game failures rather than protocol mysteries. |

## Worked example — Cribbage discard phase

A real turn exchange from a 2-player Cribbage game, annotated. This
is the flow the `lowest_index_agent.py` example takes.

### Engine → child (turn frame, one line)

```json
{"kind":"turn","api_version":"3.0.0","game":"cribbage","seat":0,"prompt_id":0,"view":{"player":0,"own_hand":{"cards":[{"rank":"Two","suit":"Hearts"},{"rank":"Nine","suit":"Hearts"},{"rank":"Queen","suit":"Diamonds"},{"rank":"Five","suit":"Spades"},{"rank":"Five","suit":"Diamonds"},{"rank":"Two","suit":"Diamonds"}]},"crib_size":0,"starter":null,"pegging_stack":[],"running_total":0,"board":{"scores":[0,0],"dealer":0},"phase":"Discard","to_act":0},"legal_actions":[{"DiscardToCrib":[{"rank":"Two","suit":"Hearts"},{"rank":"Nine","suit":"Hearts"}]},{"DiscardToCrib":[{"rank":"Two","suit":"Hearts"},{"rank":"Queen","suit":"Diamonds"}]}],"scratch":{"plan":"","notes":"","turn_log":[]}}
```

(Line wrapped here for readability — on the wire it is one line ending in `\n`.)

Interpretation:

- `api_version` and `game` tell the child what rules/schema to use.
- `seat: 0` — this child is player 0.
- `prompt_id: 0` — first turn of the game for this agent.
- `view.phase: "Discard"` — we are in the 2-card discard-to-crib
  phase.
- `legal_actions` holds two `DiscardToCrib` options. The child picks
  one by index.
- `scratch` is empty because the agent has not acted yet.

### Child → engine (action frame, one line)

```json
{"kind":"action","prompt_id":0,"action_index":0,"scratch":{"plan":"pick-first","notes":"seat=0 game=cribbage turn=0"}}
```

Interpretation:

- `prompt_id` echoes the turn's.
- `action_index: 0` — picks the first legal discard.
- `scratch` updates `plan` and `notes`. `turn_log` is omitted (the
  agent manages it; the reply cannot write it).

### Engine-side effect

The engine:

1. Validates `prompt_id == 0` and `action_index < 2`.
2. Looks up `legal_actions[0]` (the `DiscardToCrib([Two♥, Nine♥])`).
3. Hands that `Action` to the game loop, which produces one or more
   `Event`s via `apply_action`.
4. Updates the agent's `ScratchBuffer`: `plan = "pick-first"`,
   `notes = "seat=0 game=cribbage turn=0"`, and appends
   `"tick=0 seat=0 stdio_chose index=0"` to `turn_log`.
5. Writes the engine's event to the main JSONL log. The stdio frames
   themselves are **never** logged — they are coordination, not
   history. See
   `docs/solutions/architecture-patterns/ephemeral-coordination-frame-vs-logged-event-2026-04-22.md`.

Replay from seed + event log reconstructs the game byte-for-byte
without talking to the subprocess again.

## Python reference client

A stdlib-only reference implementation lives at
[`tools/python-stdio-client/`](../tools/python-stdio-client/). It
ships:

- `playtest_stdio.py` — ~170-line `StdioAgent` base class. Subclass
  and override `choose()`.
- `examples/lowest_index_agent.py` — trivial baseline agent.
- `examples/cribbage_demo.sh` — runs one Cribbage game end-to-end
  under seed 42.

Implementations in other languages should follow the same shape: one
stdin reader loop, one `choose` method per turn, one stdout writer
per reply. The protocol has no streaming or multiplexing — a single
synchronous `read` / `write` per turn is enough.
