"""Playtester stdio-protocol reference client.

A stdlib-only Python module that speaks the Phase 3 stdio agent
protocol. See ``docs/stdio-protocol.md`` for the authoritative wire
reference; the frame shapes here are a 1:1 mirror of
``crates/playtest-agents/src/remote/stdio/protocol.rs``.

Typical use::

    from playtest_stdio import StdioAgent

    class MyAgent(StdioAgent):
        def choose(self, view, legal_actions, scratch, seat, game):
            return 0, "plan text", "notes text"

    if __name__ == "__main__":
        MyAgent().run()

Design notes:

- One line of JSON per frame on stdin/stdout. Anything printed to
  ``sys.stdout`` that is not a valid frame will break the parent.
  Use ``sys.stderr`` for any debug output — the Rust agent inherits
  the child's stderr, so it will surface naturally in the terminal.
- The loop reads until stdin closes (EOF) and then returns. The Rust
  side closes stdin when the game ends, which is the child's cue to
  exit cleanly.
- Any exception raised inside ``choose`` is caught and surfaced as an
  ``error`` frame, which the Rust agent maps to ``AgentError::Other``.
"""

from __future__ import annotations

import json
import sys
from dataclasses import dataclass, field
from typing import Any, Dict, List, Optional, Tuple

API_VERSION = "3.0.0"


@dataclass
class Scratch:
    """Per-turn scratch buffer echoed from the Rust agent.

    ``plan`` and ``notes`` are round-trippable — whatever the child
    returns in the reply frame becomes the plan/notes on the next
    turn. ``turn_log`` is populated by the Rust agent (one entry per
    turn it acted on) and is effectively read-only from the child's
    perspective: the agent re-populates it on every turn.
    """

    plan: str = ""
    notes: str = ""
    turn_log: List[str] = field(default_factory=list)


class StdioAgent:
    """Base class for Python stdio agents. Subclass and override
    :meth:`choose`.

    The ``run()`` event loop reads one ``turn`` frame per line from
    stdin, dispatches to :meth:`choose`, and writes one ``action``
    frame (or ``error`` frame) per turn to stdout. It never prints to
    stdout except as a valid frame.
    """

    # Expose the protocol version as a class attribute for
    # introspection / version-pinning subclasses.
    api_version: str = API_VERSION

    def run(self) -> None:
        """Event loop. Returns when stdin closes (EOF)."""
        for line in sys.stdin:
            line = line.strip()
            if not line:
                continue
            try:
                frame = json.loads(line)
            except json.JSONDecodeError as exc:
                self._emit_error(None, f"invalid JSON: {exc}")
                continue
            if not isinstance(frame, dict):
                self._emit_error(None, "frame is not a JSON object")
                continue
            if frame.get("kind") != "turn":
                self._emit_error(
                    frame.get("prompt_id"),
                    f"expected kind='turn', got {frame.get('kind')!r}",
                )
                continue
            self._handle_turn(frame)

    def _handle_turn(self, frame: Dict[str, Any]) -> None:
        prompt_id = frame.get("prompt_id")
        scratch_raw = frame.get("scratch") or {}
        scratch = Scratch(
            plan=scratch_raw.get("plan", ""),
            notes=scratch_raw.get("notes", ""),
            turn_log=list(scratch_raw.get("turn_log", [])),
        )
        try:
            result = self.choose(
                view=frame.get("view"),
                legal_actions=frame.get("legal_actions", []),
                scratch=scratch,
                seat=frame.get("seat", 0),
                game=frame.get("game", ""),
            )
        except Exception as exc:  # noqa: BLE001 — catch-all is intentional
            self._emit_error(prompt_id, f"agent exception: {exc}")
            return

        try:
            action_index, plan, notes = result
        except (TypeError, ValueError):
            self._emit_error(
                prompt_id,
                "choose() must return (action_index, plan, notes)",
            )
            return

        legal = frame.get("legal_actions", [])
        if not isinstance(action_index, int) or not 0 <= action_index < len(legal):
            self._emit_error(
                prompt_id,
                f"action_index {action_index!r} out of range for "
                f"legal_actions of length {len(legal)}",
            )
            return

        self._emit_action(prompt_id, action_index, plan, notes)

    def _emit_action(
        self,
        prompt_id: Any,
        action_index: int,
        plan: str,
        notes: str,
    ) -> None:
        sys.stdout.write(
            json.dumps(
                {
                    "kind": "action",
                    "prompt_id": prompt_id,
                    "action_index": action_index,
                    "scratch": {"plan": plan, "notes": notes},
                }
            )
            + "\n"
        )
        sys.stdout.flush()

    def _emit_error(self, prompt_id: Optional[Any], message: str) -> None:
        sys.stdout.write(
            json.dumps(
                {
                    "kind": "error",
                    # Rust side tolerates a missing/zero prompt_id via
                    # ``#[serde(default)]`` — send 0 when we don't have
                    # one (e.g. the frame was unparseable).
                    "prompt_id": prompt_id if prompt_id is not None else 0,
                    "message": message,
                }
            )
            + "\n"
        )
        sys.stdout.flush()

    def choose(
        self,
        view: Dict[str, Any],
        legal_actions: List[Any],
        scratch: Scratch,
        seat: int,
        game: str,
    ) -> Tuple[int, str, str]:
        """Pick a legal action index and update scratch memory.

        Subclasses override this. Must return a 3-tuple
        ``(action_index, plan, notes)`` where ``action_index`` is a
        valid index into ``legal_actions``. Any exception is caught by
        :meth:`run` and surfaced as an ``error`` frame.
        """
        raise NotImplementedError(
            "StdioAgent.choose() must be overridden by a subclass"
        )
