//! Port traits: every external-system interaction crosses one of these.
//!
//! # Three kinds of "recording" in this project
//!
//! These are often conflated. They are not the same thing.
//!
//! | Kind | Who writes | Who reads | Purpose |
//! |------|------------|-----------|---------|
//! | **Port I/O tapes** | `record` adapter (wraps `production`) | `playback` adapter, tests | Reproduce non-determinism at test time |
//! | **Operator logs** | Anywhere in code via `tracing!` | Humans eyeballing stdout | Developer/operator observability |
//! | **Game event log** | [`GameEventSink`] from the game loop | Replay, Phase 1 metrics, human debug | The game's authoritative history |
//!
//! **Port I/O tapes** are a test-time artifact. `Record<Rng>` wraps a
//! production RNG and appends every `(call, output)` pair to a sidecar
//! file. `Playback<Rng>` reads that file back and returns the stored
//! outputs, panicking on divergence. Same pattern for Clock, FileSystem,
//! LlmClient. These tapes are disposable — regenerate them when code
//! shape changes.
//!
//! **Operator logs** are not a port and never will be. Use the `tracing`
//! crate directly. Warnings like "pegging stack reset at 31" are not
//! domain-bearing, do not need to be reproducible, and would not benefit
//! from the stub/prod/record/playback ceremony.
//!
//! **The game event log** is a permanent production artifact. Every run
//! writes one. `GameState@tick_N = replay(seed, events[0..N])`, so the
//! event log *is* the game's memory. All downstream Phases (metrics,
//! compare, UI) read from it.
//!
//! # Object-safety
//!
//! All ports here are object-safe (usable via `&mut dyn Port`) *except*
//! [`LlmClient`], which accepts the coupling of `async_trait` so
//! asynchronous LLM calls can live behind the same record/playback
//! discipline as every other external system.
//!
//! # Input vs. output ports
//!
//! Four of the ports ([`Clock`], [`Rng`], [`FileSystem`], [`LlmClient`])
//! are **input** ports: the engine consumes values from them. All four
//! adapter variants are meaningful; playback is what gives us
//! deterministic end-to-end tests.
//!
//! [`GameEventSink`] is the one **output** port: the engine produces to
//! it. `stub` and `production` are the meaningful variants; `record` and
//! `playback` collapse (the game event log *is* the tape for the game's
//! own history).

pub mod clock;
pub mod filesystem;
pub mod game_event_sink;
pub mod llm_client;
pub mod rng;

pub use clock::{Clock, UnixMillis};
pub use filesystem::{FileSystem, FsError};
pub use game_event_sink::{GameEventSink, GameEventSinkError};
pub use llm_client::{LlmClient, LlmError, LlmRequest, LlmResponse};
pub use rng::{Rng, RngError};
