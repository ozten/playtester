//! `LlmAgent<G>`: game-generic agent driven by an `Arc<dyn LlmClient>`.
//!
//! Owns its own [`ScratchBuffer`]; builds cacheable system blocks + an
//! uncached per-turn user message; parses the model's JSON reply into an
//! action index; appends a cost-observability record to a
//! [`LlmSidecar`] when one is configured.
//!
//! The agent is a pure port consumer — the deterministic record/playback
//! seam lives at the byte-level `LlmClient` tape, not here.

use core::marker::PhantomData;
use std::sync::Arc;

use async_trait::async_trait;
use playtest_core::{Agent, AgentError, Game, PlayerId};
use playtest_ports::{ChatMessage, ChatRole, LlmClient, LlmError, LlmRequest};
use serde::{Deserialize, Serialize};
use tokio::time::Instant;

use super::prompt::{build_system_blocks, build_user_message};
use super::scratch::ScratchBuffer;
use super::sidecar::{LlmCallRecord, LlmSidecar};

/// Shape of the model's expected reply.
#[derive(Debug, Deserialize, Serialize)]
struct LlmReply {
    action_index: usize,
    #[serde(default)]
    plan: String,
    #[serde(default)]
    notes: String,
}

/// Configuration for [`LlmAgent`]. Construction-time — each per-turn
/// invocation reads from these shared handles.
#[derive(Clone)]
pub struct LlmAgentConfig {
    pub llm: Arc<dyn LlmClient>,
    pub model: String,
    pub rules_text: Arc<str>,
    pub card_catalog: Arc<str>,
    pub sidecar: Option<Arc<LlmSidecar>>,
    pub max_tokens: u32,
    pub temperature: Option<f32>,
}

impl core::fmt::Debug for LlmAgentConfig {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("LlmAgentConfig")
            .field("model", &self.model)
            .field("max_tokens", &self.max_tokens)
            .field("temperature", &self.temperature)
            .field("sidecar", &self.sidecar.is_some())
            .finish_non_exhaustive()
    }
}

/// LLM-driven agent, generic over the `Game` it plays.
///
/// Agent is single-threaded: each `choose` call mutates `scratch` and
/// bumps `tick`. Concurrent invocations would be a misuse.
pub struct LlmAgent<G>
where
    G: Game + ?Sized,
{
    seat: PlayerId,
    cfg: LlmAgentConfig,
    scratch: ScratchBuffer,
    tick: u64,
    _game: PhantomData<fn() -> G>,
}

impl<G> LlmAgent<G>
where
    G: Game + ?Sized,
{
    #[must_use]
    pub fn new(seat: PlayerId, cfg: LlmAgentConfig) -> Self {
        Self {
            seat,
            cfg,
            scratch: ScratchBuffer::new(),
            tick: 0,
            _game: PhantomData,
        }
    }

    /// Read-only view of the current scratch buffer. Tests use this to
    /// assert that `plan` / `notes` / `turn_log` evolve correctly.
    #[must_use]
    pub fn scratch(&self) -> &ScratchBuffer {
        &self.scratch
    }
}

impl<G> core::fmt::Debug for LlmAgent<G>
where
    G: Game + ?Sized,
{
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("LlmAgent")
            .field("seat", &self.seat)
            .field("tick", &self.tick)
            .finish_non_exhaustive()
    }
}

async fn write_sidecar(sidecar: Option<&Arc<LlmSidecar>>, record: LlmCallRecord) {
    if let Some(s) = sidecar
        && let Err(e) = s.append_call(&record).await
    {
        // Sidecar write failures never kill the agent. Surface to
        // stderr so CI noise is obvious without dragging in the
        // `tracing` crate for one line.
        eprintln!("LlmAgent sidecar write failed: {e}");
    }
}

#[async_trait]
impl<G> Agent<G> for LlmAgent<G>
where
    G: Game + ?Sized + Send + Sync,
    G::State: Send + Sync,
    G::PublicView: Send + Sync + Serialize,
    G::Action: Send + Sync + Serialize,
{
    #[allow(clippy::too_many_lines)]
    async fn choose(
        &mut self,
        view: &G::PublicView,
        legal: &[G::Action],
        _state: &G::State,
    ) -> Result<usize, AgentError> {
        if legal.is_empty() {
            return Err(AgentError::Other(
                "LlmAgent::choose called with empty legal slice (engine bug)".into(),
            ));
        }

        // Short-circuit: a single legal action doesn't need the LLM.
        // Common in the pegging phase where only one card of the
        // player's hand keeps the running total ≤ 31. Saves a turn's
        // worth of tokens and sidesteps retry logic.
        if legal.len() == 1 {
            self.scratch
                .push_turn_log(format!(
                    "tick={} seat={} forced index=0 (1 legal action)",
                    self.tick, self.seat
                ));
            self.tick += 1;
            return Ok(0);
        }

        let system_blocks = build_system_blocks(&self.cfg.rules_text, &self.cfg.card_catalog);
        let user_body = build_user_message(view, legal, &self.scratch)
            .map_err(|e| AgentError::Other(format!("serialize user message: {e}")))?;

        let mut messages: Vec<ChatMessage> = vec![ChatMessage {
            role: ChatRole::User,
            content: user_body.clone(),
        }];

        let start = Instant::now();
        let first = self
            .cfg
            .llm
            .complete(LlmRequest {
                system_blocks: system_blocks.clone(),
                messages: messages.clone(),
                model: self.cfg.model.clone(),
                max_tokens: self.cfg.max_tokens,
                temperature: self.cfg.temperature,
            })
            .await;

        let resp = match first {
            Ok(r) => r,
            Err(e) => {
                let latency_ms = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
                let budget_exceeded = matches!(e, LlmError::BudgetExceeded { .. });
                write_sidecar(
                    self.cfg.sidecar.as_ref(),
                    LlmCallRecord {
                        tick: self.tick,
                        seat: self.seat,
                        model: self.cfg.model.clone(),
                        input_tokens: 0,
                        output_tokens: 0,
                        cache_read_input_tokens: 0,
                        cache_creation_input_tokens: 0,
                        latency_ms,
                        chosen_index: None,
                        budget_exceeded,
                    },
                )
                .await;
                self.tick += 1;
                let msg = if budget_exceeded {
                    format!("llm budget exceeded: {e}")
                } else {
                    format!("llm call failed: {e}")
                };
                return Err(AgentError::Other(msg));
            }
        };

        // Parse. On failure, retry exactly once with an augmented
        // conversation (append the invalid reply as an Assistant turn,
        // then a User turn explaining the format requirement).
        let reply: LlmReply = match serde_json::from_str(&resp.text) {
            Ok(r) => r,
            Err(first_err) => {
                messages.push(ChatMessage {
                    role: ChatRole::Assistant,
                    content: resp.text.clone(),
                });
                messages.push(ChatMessage {
                    role: ChatRole::User,
                    content: "Your previous reply was not valid JSON. Please respond with \
only the JSON object as instructed."
                        .to_owned(),
                });

                let second = self
                    .cfg
                    .llm
                    .complete(LlmRequest {
                        system_blocks,
                        messages,
                        model: self.cfg.model.clone(),
                        max_tokens: self.cfg.max_tokens,
                        temperature: self.cfg.temperature,
                    })
                    .await;

                let resp2 = match second {
                    Ok(r) => r,
                    Err(e) => {
                        let latency_ms =
                            u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
                        let budget_exceeded = matches!(e, LlmError::BudgetExceeded { .. });
                        write_sidecar(
                            self.cfg.sidecar.as_ref(),
                            LlmCallRecord {
                                tick: self.tick,
                                seat: self.seat,
                                model: self.cfg.model.clone(),
                                input_tokens: resp.input_tokens,
                                output_tokens: resp.output_tokens,
                                cache_read_input_tokens: resp.cache_read_input_tokens,
                                cache_creation_input_tokens: resp.cache_creation_input_tokens,
                                latency_ms,
                                chosen_index: None,
                                budget_exceeded,
                            },
                        )
                        .await;
                        self.tick += 1;
                        let msg = if budget_exceeded {
                            format!("llm budget exceeded during retry: {e}")
                        } else {
                            format!("llm retry call failed: {e}")
                        };
                        return Err(AgentError::Other(msg));
                    }
                };

                match serde_json::from_str::<LlmReply>(&resp2.text) {
                    Ok(r) => r,
                    Err(second_err) => {
                        let latency_ms =
                            u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
                        write_sidecar(
                            self.cfg.sidecar.as_ref(),
                            LlmCallRecord {
                                tick: self.tick,
                                seat: self.seat,
                                model: self.cfg.model.clone(),
                                input_tokens: resp.input_tokens + resp2.input_tokens,
                                output_tokens: resp.output_tokens + resp2.output_tokens,
                                cache_read_input_tokens: resp.cache_read_input_tokens
                                    + resp2.cache_read_input_tokens,
                                cache_creation_input_tokens: resp.cache_creation_input_tokens
                                    + resp2.cache_creation_input_tokens,
                                latency_ms,
                                chosen_index: None,
                                budget_exceeded: false,
                            },
                        )
                        .await;
                        self.tick += 1;
                        return Err(AgentError::Other(format!(
                            "failed to parse LLM reply as JSON after one retry: {first_err}; retry error: {second_err}"
                        )));
                    }
                }
            }
        };

        // Validate the index is inside the legal slice the model saw.
        if reply.action_index >= legal.len() {
            let latency_ms = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
            write_sidecar(
                self.cfg.sidecar.as_ref(),
                LlmCallRecord {
                    tick: self.tick,
                    seat: self.seat,
                    model: self.cfg.model.clone(),
                    input_tokens: resp.input_tokens,
                    output_tokens: resp.output_tokens,
                    cache_read_input_tokens: resp.cache_read_input_tokens,
                    cache_creation_input_tokens: resp.cache_creation_input_tokens,
                    latency_ms,
                    chosen_index: None,
                    budget_exceeded: false,
                },
            )
            .await;
            self.tick += 1;
            return Err(AgentError::Other(format!(
                "LLM returned action_index {} out of {} legal range",
                reply.action_index,
                legal.len()
            )));
        }

        // Happy path: update scratch + log, write sidecar, return index.
        let chosen = reply.action_index;
        self.scratch.plan = reply.plan;
        self.scratch.notes = reply.notes;
        self.scratch
            .push_turn_log(format!(
                "tick={} seat={} chose index={}",
                self.tick, self.seat, chosen
            ));

        let latency_ms = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
        write_sidecar(
            self.cfg.sidecar.as_ref(),
            LlmCallRecord {
                tick: self.tick,
                seat: self.seat,
                model: self.cfg.model.clone(),
                input_tokens: resp.input_tokens,
                output_tokens: resp.output_tokens,
                cache_read_input_tokens: resp.cache_read_input_tokens,
                cache_creation_input_tokens: resp.cache_creation_input_tokens,
                latency_ms,
                chosen_index: Some(chosen),
                budget_exceeded: false,
            },
        )
        .await;

        self.tick += 1;
        Ok(chosen)
    }
}
