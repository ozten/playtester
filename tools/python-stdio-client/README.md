# Playtester Python stdio client

A stdlib-only Python reference implementation of the Playtester
Phase 3 stdio agent protocol. Lets an external Python process play a
game driven by the Rust `playtest` engine without touching HTTP.

Authoritative wire reference: [`docs/stdio-protocol.md`](../../docs/stdio-protocol.md).

## What this is

The `playtest` CLI can delegate any seat of a game to a subprocess
that speaks a tiny newline-delimited JSON protocol on its stdin and
stdout. This directory ships:

- `playtest_stdio.py` — the library. One file, pure stdlib, ~170
  lines including docstrings. Subclass `StdioAgent`, override
  `choose()`, call `.run()`. The base class handles framing,
  scratch-buffer unwrap/repack, prompt-id echo, and error frames.
- `examples/lowest_index_agent.py` — the simplest possible agent:
  always picks `legal_actions[0]`.
- `examples/cribbage_demo.sh` — plays one full 2-player Cribbage
  game with the Python subprocess on seat 0 and a random agent on
  seat 1, deterministic under seed 42.

## Quick start

### Run the bundled demo

```sh
bash tools/python-stdio-client/examples/cribbage_demo.sh
```

The script builds the `playtest` binary with `--release` (required by
this project), resolves `python3` to an absolute path (the Rust side
validates the command exists on disk — there is no PATH lookup),
plays a 121-point Cribbage game, and writes a JSONL event log to
`target/playtest-runs/stdio-demo/game-0000.jsonl`.

Expected output:

```
Demo complete — log at .../target/playtest-runs/stdio-demo/game-0000.jsonl
Replay with:  cargo run --release --quiet --bin playtest -- replay ...
```

### Replay the recorded game

```sh
cargo run --release --quiet --bin playtest -- \
  replay target/playtest-runs/stdio-demo/game-0000.jsonl
```

Replay re-runs the game from seed + events without contacting the
Python subprocess again — the action indices are already in the
event log.

## Writing your own agent

Subclass `StdioAgent` and override `choose()`:

```python
from playtest_stdio import StdioAgent, Scratch


class MyAgent(StdioAgent):
    def choose(self, view, legal_actions, scratch, seat, game):
        # view         — game-specific PublicView JSON (dict)
        # legal_actions — list of game-specific Action JSON values
        # scratch      — Scratch(plan, notes, turn_log)
        # seat         — 0-based integer seat
        # game         — game name, e.g. "cribbage"
        action_index = 0  # pick whatever makes sense
        plan = "keep aces; pass if pegging total > 20"
        notes = f"turn {len(scratch.turn_log)}: first legal action"
        return action_index, plan, notes


if __name__ == "__main__":
    MyAgent().run()
```

Invoke it via the CLI:

```sh
PY=$(command -v python3)
playtest play --game cribbage \
  --agents stdio,random \
  --stdio-cmd "${PY}" \
  --stdio-arg /absolute/path/to/my_agent.py \
  --seed 42 --games 1 \
  --out ./runs/
```

Notes on the CLI flags:

- `--stdio-cmd` must be an absolute path to an existing file on
  disk. The Rust side runs `fs::metadata` on it at agent-construction
  time to fail fast on typos.
- `--stdio-arg` is repeatable — each occurrence becomes one
  positional argument to the child. There is no argv-splitting.
- `--out` is required and must exist or be creatable.
- `--fixed-time 0` pins the log header's `started_at` timestamp so
  two runs with the same seed produce bit-identical JSONL files.

## Protocol reference

See [`docs/stdio-protocol.md`](../../docs/stdio-protocol.md) for the
authoritative wire-level reference: frame shapes, versioning,
lifecycle, and the error taxonomy.

Short form:

- Rust agent writes one `turn` frame per turn to the child's stdin,
  terminated by `\n`.
- Child writes one `action` frame (or `error` frame) per turn to its
  stdout, terminated by `\n`.
- Frames are JSON objects tagged on `kind`.
- `api_version` and `game` are on every `turn` frame — there is no
  separate handshake.
- The child's stderr is inherited by the parent terminal. Use it for
  debug output.

## Limitations (Phase 3)

- **One subprocess per game.** The agent spawns the child on its
  first `choose` call and keeps it alive for the life of one game.
  Multi-game runs spawn a fresh child per game.
- **No streaming, no partial frames.** One line of JSON per frame.
- **No timeout.** If the child hangs, the game hangs. Same policy as
  the Phase 2.5 HTTP remote agent. `StdioAgent::Drop` kills the
  child on shutdown via `kill_on_drop(true)`.
- **CLI only.** `POST /api/runs` rejects any `llm` or `stdio` agent
  kind with `AgentKindNotAllowedHere` — arbitrary command execution
  over HTTP is not in the localhost trust model.
- **No reconnect.** A child that exits mid-game fails the game.
- **LLM credentials are scrubbed.** The child's environment has
  `ANTHROPIC_API_KEY` and `PLAYTEST_OPENAI_COMPAT_KEY` removed before
  spawn. The parent's LLM creds are not shared with user subprocesses.

## Troubleshooting

- **"child binary not found" at run start.** The `--stdio-cmd` path
  does not resolve to an existing file. Use `command -v python3` to
  resolve against PATH before passing to the CLI — the demo script
  does this.
- **Child prints to stdout and the parent hangs on a parse error.**
  Every debug line from the child goes to stdout would be interpreted
  as a frame. Use `sys.stderr` (or `logging` configured to stderr)
  for debugging. The Rust agent discards up to 16 non-JSON lines
  before erroring, so a rare debug line is survivable, but routine
  prints are not.
- **Game log looks corrupted after re-running the demo.** The
  production event-sink appends rather than truncating. Delete the
  `target/playtest-runs/stdio-demo/` directory before re-running, or
  point `--out` at a fresh directory per run.
- **`python3` not found at all.** Install Python 3.10 or newer; the
  demo tested on 3.12.3. The library itself has no runtime Python
  version requirements beyond `dataclasses` (3.7+).
- **Parent exited, but the Python process is still running.** The
  Rust agent sets `kill_on_drop(true)` on its `tokio::process::Child`
  handle. If you see a zombie, your child probably ignored SIGTERM
  after SIGPIPE — handle stdin EOF as "exit cleanly".

## No install step required

This directory is not a pip package. `examples/lowest_index_agent.py`
inserts the parent directory onto `sys.path` so it can `import
playtest_stdio` without any installation. Drop the file into your
own project and import it the same way, or (if you prefer) run
`pip install -e tools/python-stdio-client/` — there is no
`pyproject.toml`, so this is a no-op today. The intent is "one file,
pure stdlib, copy it wherever."
