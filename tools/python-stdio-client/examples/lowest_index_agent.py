#!/usr/bin/env python3
"""Simplest possible stdio agent: always picks legal_actions[0].

Invoke via the playtest CLI:

    playtest play --game cribbage \\
        --agents stdio,random \\
        --stdio-cmd python3 \\
        --stdio-arg /path/to/lowest_index_agent.py \\
        --seed 42 --games 1 --out ./runs/

See ``tools/python-stdio-client/examples/cribbage_demo.sh`` for a
ready-to-run wrapper.
"""
import sys
from pathlib import Path

# Allow running from the examples/ directory without installing the
# package.
sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from playtest_stdio import StdioAgent  # noqa: E402


class LowestIndexAgent(StdioAgent):
    """Trivial baseline: always plays the first legal action.

    Demonstrates the three scratch fields: ``plan`` is a static
    strategy hint, ``notes`` shows how to reference turn-state, and
    the base class does all the framing work.
    """

    def choose(self, view, legal_actions, scratch, seat, game):
        return (
            0,
            "pick-first",
            f"seat={seat} game={game} turn={len(scratch.turn_log)}",
        )


if __name__ == "__main__":
    LowestIndexAgent().run()
