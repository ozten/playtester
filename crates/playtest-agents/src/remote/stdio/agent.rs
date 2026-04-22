//! `StdioAgent<G>`: game-generic agent that defers `choose` to a
//! configured subprocess over newline-delimited JSON.
//!
//! Sibling to [`HttpRemoteAgent`](super::super::http_remote::HttpRemoteAgent);
//! the difference is topology — HTTP remote agents share a server-side
//! coordinator (one browser tab -> one seat across the whole run),
//! while stdio agents own their subprocess 1:1 for the life of one
//! game. Consequently there is no transport trait: this agent spawns,
//! polls, and reaps the `tokio::process::Child` directly.
//!
//! ## Subprocess lifecycle (load-bearing)
//!
//! - **Spawn is lazy.** [`StdioAgent::new`] validates that the
//!   configured command exists on disk but does *not* spawn. The
//!   subprocess is spawned on the first [`Agent::choose`] call so
//!   `tokio::process::Command::spawn` sees the current-thread runtime
//!   the game loop runs on. This resolves the runtime-context
//!   requirement `tokio::process` imposes (see the Phase 3 plan's
//!   "Tokio + subprocess gotcha" in Key Technical Decisions).
//! - **Reap-on-drop.** `ChildHandle` field declaration order is
//!   `stdin`, `stdout`, `child`. Rust drops in declaration order, so
//!   closing `stdin` first sends EOF (letting a well-behaved child
//!   exit cleanly); the child's `kill_on_drop(true)` catches any
//!   hanger.
//! - **Credential scrubbing.** `ANTHROPIC_API_KEY` and
//!   `PLAYTEST_OPENAI_COMPAT_KEY` are `env_remove`d before spawn. The
//!   CLI trust model extends to the subprocess, but LLM credentials
//!   belong to the parent.

use core::marker::PhantomData;
use std::path::PathBuf;
use std::process::Stdio;

use async_trait::async_trait;
use playtest_core::{Agent, AgentError, Game, PlayerId};
use serde::Serialize;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

use crate::llm::ScratchBuffer;

use super::protocol::{ReplyFrame, STDIO_API_VERSION, TurnFrame};

/// Upper bound on how many non-JSON lines the agent will discard from
/// the child's stdout before emitting `TooManyGarbageLines`.
///
/// Enough to swallow a couple of debug prints but small enough that a
/// silently-broken child surfaces fast.
const MAX_GARBAGE_LINES: usize = 16;

/// Environment variables scrubbed from the child's environment on
/// spawn. The user's subprocess has no business with our LLM
/// credentials.
const SCRUBBED_ENV_VARS: &[&str] = &["ANTHROPIC_API_KEY", "PLAYTEST_OPENAI_COMPAT_KEY"];

/// Configuration for a [`StdioAgent`]. Required: the executable path.
/// Args are forwarded as-is.
#[derive(Debug, Clone)]
pub struct StdioAgentConfig {
    /// Path to the executable to spawn.
    pub command: PathBuf,
    /// Command-line arguments passed to the executable.
    pub args: Vec<String>,
}

impl StdioAgentConfig {
    /// Quick existence check run at [`StdioAgent::new`] time — no
    /// spawn, no runtime requirement. Surfaces typo / packaging
    /// errors before any game begins.
    ///
    /// # Errors
    /// Returns [`StdioProtocolError::CommandNotFound`] if the path
    /// doesn't resolve to a filesystem entry. Permission and
    /// executability checks are skipped intentionally — those surface
    /// as spawn errors, which carry better platform-specific messages
    /// than a Rust-side pre-check would.
    pub fn validate(&self) -> Result<(), StdioProtocolError> {
        if std::fs::metadata(&self.command).is_err() {
            return Err(StdioProtocolError::CommandNotFound(self.command.clone()));
        }
        Ok(())
    }
}

/// Failure modes for the stdio agent. Boundary-mapped to
/// [`AgentError::Other`] via `Display`.
#[derive(Debug, thiserror::Error)]
pub enum StdioProtocolError {
    /// `Command::spawn` itself failed (permission denied, exec format,
    /// etc.). The `String` is the platform-specific kernel message.
    #[error("child process failed to spawn: {0}")]
    SpawnFailed(String),
    /// The configured command path does not exist on disk (caught at
    /// build time via [`StdioAgentConfig::validate`]).
    #[error("child binary not found: {0}")]
    CommandNotFound(PathBuf),
    /// Child signalled an `api_version` other than the one the agent
    /// sent. Reserved for future use — Phase 3 detects mismatches
    /// implicitly via `error` frames.
    #[error("protocol version mismatch: child said {got}, expected {expected}")]
    ProtocolVersionMismatch { expected: String, got: String },
    /// The child's reply could not be parsed as a [`ReplyFrame`] after
    /// [`MAX_GARBAGE_LINES`] attempts.
    #[error("failed to parse child reply as JSON: {0}")]
    ParseError(String),
    /// Child replied with a different `prompt_id` than the agent sent.
    /// Most often a symptom of a buggy child that re-orders turns.
    #[error("child replied with prompt_id {got}, expected {expected}")]
    PromptIdMismatch { expected: u64, got: u64 },
    /// Child's `action_index` is not a valid index into the
    /// `legal_actions` slice.
    #[error("child replied with action_index {got}, but only {legal_len} legal actions were sent")]
    IndexOutOfRange { got: usize, legal_len: usize },
    /// Child exited before replying, or `read_line` returned 0 bytes
    /// mid-turn.
    #[error("child process exited before replying")]
    ChildExited,
    /// Any other I/O failure talking to the child.
    #[error("stdio io error: {0}")]
    Io(String),
    /// Child emitted more than [`MAX_GARBAGE_LINES`] non-JSON lines
    /// before (or instead of) a valid frame.
    #[error("child sent {0} non-JSON lines before a valid frame")]
    TooManyGarbageLines(usize),
    /// Child emitted a structurally valid `error` frame.
    #[error("child replied with error frame: {0}")]
    ChildError(String),
}

/// Handle to the child process and its plumbed stdio.
///
/// Field declaration order is load-bearing: Rust drops in declaration
/// order, so `stdin` closes first (EOF -> graceful child exit), then
/// `stdout`, then `child` (whose `kill_on_drop(true)` fires if the
/// child is still alive).
struct ChildHandle {
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    /// Held onto so `Drop` invokes the configured `kill_on_drop(true)`;
    /// also lets us own PID / exit-status if needed in the future.
    #[allow(dead_code)]
    child: Child,
}

/// Stdio-subprocess agent, generic over the game it plays.
///
/// Not cloneable: the subprocess is 1:1 with the agent. Single-threaded
/// at the per-seat level — concurrent `choose` calls would multiplex
/// prompt ids out of order.
pub struct StdioAgent<G>
where
    G: Game + ?Sized,
{
    seat: PlayerId,
    game_name: String,
    cfg: StdioAgentConfig,
    /// `None` until the first `choose` call spawns the subprocess
    /// inside the game-loop runtime.
    child: Option<ChildHandle>,
    next_prompt_id: u64,
    scratch: ScratchBuffer,
    _game: PhantomData<fn() -> G>,
}

impl<G> StdioAgent<G>
where
    G: Game + ?Sized,
{
    /// Construct an agent that will spawn `cfg.command` on its first
    /// `choose` call. The command path is validated here (fast-fail on
    /// typo); the process is not spawned until a tokio runtime is
    /// entered.
    ///
    /// # Errors
    /// Returns [`StdioProtocolError::CommandNotFound`] if the command
    /// path doesn't exist. Spawn-time failures (permission denied, exec
    /// format) surface later from `choose`.
    pub fn new(
        seat: PlayerId,
        game_name: impl Into<String>,
        cfg: StdioAgentConfig,
    ) -> Result<Self, StdioProtocolError> {
        cfg.validate()?;
        Ok(Self {
            seat,
            game_name: game_name.into(),
            cfg,
            child: None,
            next_prompt_id: 0,
            scratch: ScratchBuffer::new(),
            _game: PhantomData,
        })
    }

    /// Read-only view of this agent's scratch buffer. Test-only.
    #[must_use]
    pub fn scratch(&self) -> &ScratchBuffer {
        &self.scratch
    }

    /// Whether the subprocess has been spawned yet. Test-only.
    #[must_use]
    pub fn is_spawned(&self) -> bool {
        self.child.is_some()
    }

    /// Lazy spawn. Called from `choose` the first time. Must run inside
    /// a tokio runtime with the I/O driver enabled (i.e. built with
    /// `enable_all()` or `enable_io()`). Not `async` today because
    /// `tokio::process::Command::spawn` is itself synchronous — what
    /// it requires is only that the *caller* run inside a tokio
    /// runtime context so the child registers with the signal reaper.
    fn spawn_lazy(&mut self) -> Result<(), StdioProtocolError> {
        let mut cmd = Command::new(&self.cfg.command);
        cmd.args(&self.cfg.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .kill_on_drop(true);
        for v in SCRUBBED_ENV_VARS {
            cmd.env_remove(v);
        }

        let mut child = cmd
            .spawn()
            .map_err(|e| StdioProtocolError::SpawnFailed(e.to_string()))?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| StdioProtocolError::Io("child stdin pipe missing".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| StdioProtocolError::Io("child stdout pipe missing".into()))?;
        let stdout = BufReader::new(stdout);

        self.child = Some(ChildHandle {
            stdin,
            stdout,
            child,
        });
        Ok(())
    }
}

impl<G> core::fmt::Debug for StdioAgent<G>
where
    G: Game + ?Sized,
{
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("StdioAgent")
            .field("seat", &self.seat)
            .field("game", &self.game_name)
            .field("command", &self.cfg.command)
            .field("spawned", &self.child.is_some())
            .field("next_prompt_id", &self.next_prompt_id)
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl<G> Agent<G> for StdioAgent<G>
where
    G: Game + ?Sized + Send + Sync,
    G::State: Send + Sync,
    G::PublicView: Send + Sync + Serialize + Clone,
    G::Action: Send + Sync + Serialize + Clone,
{
    async fn choose(
        &mut self,
        view: &G::PublicView,
        legal: &[G::Action],
        _state: &G::State,
    ) -> Result<usize, AgentError> {
        if legal.is_empty() {
            return Err(AgentError::Other(
                "StdioAgent::choose called with empty legal slice (engine bug)".into(),
            ));
        }

        // Short-circuit: a single legal action doesn't need the child.
        // Same rationale as `LlmAgent` — skip the round-trip when the
        // forced index is 0.
        if legal.len() == 1 {
            self.scratch.push_turn_log(format!(
                "tick={} seat={} stdio forced index=0 (1 legal action)",
                self.next_prompt_id, self.seat
            ));
            self.next_prompt_id += 1;
            return Ok(0);
        }

        // Lazy spawn on first real turn so `tokio::process::Command`
        // runs inside the game-loop runtime (see module docs).
        if self.child.is_none() {
            self.spawn_lazy()
                .map_err(|e| AgentError::Other(e.to_string()))?;
        }

        let prompt_id = self.next_prompt_id;
        self.next_prompt_id += 1;

        let frame = TurnFrame {
            kind: "turn",
            api_version: STDIO_API_VERSION,
            game: self.game_name.clone(),
            seat: self.seat,
            prompt_id,
            view: view.clone(),
            legal_actions: legal.to_vec(),
            scratch: self.scratch.clone(),
        };
        let frame_json = serde_json::to_string(&frame)
            .map_err(|e| AgentError::Other(format!("serialize TurnFrame: {e}")))?;

        // Write one line: JSON + newline + flush.
        let handle = self.child.as_mut().expect("spawned above");
        write_turn(handle, &frame_json)
            .await
            .map_err(|e| AgentError::Other(e.to_string()))?;

        // Read reply, discarding up to MAX_GARBAGE_LINES non-JSON lines.
        let reply = read_reply(handle)
            .await
            .map_err(|e| AgentError::Other(e.to_string()))?;

        match reply {
            ReplyFrame::Error {
                prompt_id: got,
                message,
            } => {
                // Validate prompt_id on error frames too — if the child
                // errors on a stale prompt, surface that as a mismatch
                // rather than a generic child-error.
                if got != prompt_id && got != 0 {
                    return Err(AgentError::Other(
                        StdioProtocolError::PromptIdMismatch {
                            expected: prompt_id,
                            got,
                        }
                        .to_string(),
                    ));
                }
                Err(AgentError::Other(
                    StdioProtocolError::ChildError(message).to_string(),
                ))
            }
            ReplyFrame::Action {
                prompt_id: got,
                action_index,
                scratch,
            } => {
                if got != prompt_id {
                    return Err(AgentError::Other(
                        StdioProtocolError::PromptIdMismatch {
                            expected: prompt_id,
                            got,
                        }
                        .to_string(),
                    ));
                }
                if action_index >= legal.len() {
                    return Err(AgentError::Other(
                        StdioProtocolError::IndexOutOfRange {
                            got: action_index,
                            legal_len: legal.len(),
                        }
                        .to_string(),
                    ));
                }
                self.scratch.plan = scratch.plan;
                self.scratch.notes = scratch.notes;
                self.scratch.push_turn_log(format!(
                    "tick={} seat={} stdio_chose index={}",
                    prompt_id, self.seat, action_index
                ));
                Ok(action_index)
            }
        }
    }
}

async fn write_turn(handle: &mut ChildHandle, frame_json: &str) -> Result<(), StdioProtocolError> {
    let stdin = &mut handle.stdin;
    stdin
        .write_all(frame_json.as_bytes())
        .await
        .map_err(|e| map_write_err(&e))?;
    stdin.write_all(b"\n").await.map_err(|e| map_write_err(&e))?;
    stdin.flush().await.map_err(|e| map_write_err(&e))?;
    Ok(())
}

fn map_write_err(e: &std::io::Error) -> StdioProtocolError {
    // A closed pipe means the child exited; map explicitly so callers
    // can surface a targeted error.
    if matches!(
        e.kind(),
        std::io::ErrorKind::BrokenPipe | std::io::ErrorKind::UnexpectedEof
    ) {
        StdioProtocolError::ChildExited
    } else {
        StdioProtocolError::Io(e.to_string())
    }
}

async fn read_reply(handle: &mut ChildHandle) -> Result<ReplyFrame, StdioProtocolError> {
    let mut garbage = 0usize;

    loop {
        let mut buf = String::new();
        let n = handle
            .stdout
            .read_line(&mut buf)
            .await
            .map_err(|e| StdioProtocolError::Io(e.to_string()))?;
        if n == 0 {
            return Err(StdioProtocolError::ChildExited);
        }

        let trimmed = buf.trim();
        if trimmed.is_empty() {
            garbage += 1;
            if garbage >= MAX_GARBAGE_LINES {
                return Err(StdioProtocolError::TooManyGarbageLines(garbage));
            }
            continue;
        }

        if let Ok(frame) = serde_json::from_str::<ReplyFrame>(trimmed) {
            return Ok(frame);
        }
        garbage += 1;
        if garbage >= MAX_GARBAGE_LINES {
            return Err(StdioProtocolError::TooManyGarbageLines(garbage));
        }
    }
}
