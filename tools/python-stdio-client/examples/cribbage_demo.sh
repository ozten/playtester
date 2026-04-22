#!/usr/bin/env bash
# Demo: play one full 2-player Cribbage game with a Python subprocess
# driving seat 0, and a random agent driving seat 1. Deterministic
# under seed 42.
#
# Expected outcome: the command exits 0 and writes
# ${REPO_ROOT}/target/playtest-runs/stdio-demo/game-0000.jsonl. The log
# replays byte-for-byte under `playtest replay`.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
AGENT="${SCRIPT_DIR}/lowest_index_agent.py"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../../.." && pwd)"
OUT_DIR="${REPO_ROOT}/target/playtest-runs/stdio-demo"

mkdir -p "${OUT_DIR}"

cd "${REPO_ROOT}"

# Resolve python3 to an absolute path: the Rust StdioAgentConfig
# validates that --stdio-cmd exists on disk at agent-construction time
# (no PATH lookup). `env python3` gives us PATH resolution via a
# POSIX-standard absolute path.
PY_ABS="$(command -v python3)"
if [[ -z "${PY_ABS}" ]]; then
  echo "python3 not found on PATH" >&2
  exit 1
fi

# --release for everything — the project's CLAUDE.md forbids
# target/debug/ artifacts on this machine.
cargo run --release --quiet --bin playtest -- play \
  --game cribbage \
  --agents "stdio,random" \
  --stdio-cmd "${PY_ABS}" \
  --stdio-arg "${AGENT}" \
  --seed 42 \
  --games 1 \
  --out "${OUT_DIR}"

echo "Demo complete — log at ${OUT_DIR}/game-0000.jsonl"
echo "Replay with:  cargo run --release --quiet --bin playtest -- replay ${OUT_DIR}/game-0000.jsonl"
