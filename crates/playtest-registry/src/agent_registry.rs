//! Construct agents by name.
//!
//! Since Phase 2 introduced per-game eval functions
//! ([`cribbage_eval`](playtest_cribbage::cribbage_eval) and
//! [`shipwreck_eval`](playtest_shipwreck::shipwreck_eval)), agent
//! construction split into two per-game entry points. The original
//! generic [`build_agent`] remains for agents that don't need a
//! per-game eval (Random, HTTP-remote) — it's now a thin shim that the
//! per-game factories fall through to for those names.
//!
//! ## Per-seat build context
//!
//! Phase 2.5 added one more dimension: the `http-remote` agent needs a
//! server-provided transport port at construction time. Rather than
//! continue growing the positional signature, all three factories take
//! an [`AgentBuildCtx`] that carries the seat, seed, and any optional
//! transports the caller can supply. CLI callers pass
//! [`AgentBuildCtx::cli`] (no transport); the HTTP server builds a ctx
//! with a `Some(transport)` for any seat it wants to back with a
//! remote client.

use std::sync::Arc;

use anyhow::{Result, anyhow, bail};
use playtest_adapters::ProductionRng;
use playtest_agents::{
    DEFAULT_TEMPERATURE, GreedyAgent, HeuristicAgent, HttpRemoteAgent, ISMCTSAgent, ISMCTSConfig,
    LlmAgent, LlmAgentConfig, LlmSidecar, PostGameCritic, RandomAgent, RemoteAgentTransport,
    StdioAgent, StdioAgentConfig, build_shared_handles, parse_config_overrides,
};
use playtest_core::{Agent, Game, PlayerId};
use playtest_cribbage::{CribbageGame, cribbage_eval};
use playtest_greatgyre::{GreatGyreGame, greatgyre_eval};
use playtest_ports::LlmClient;
use playtest_shipwreck::{ShipWreckGame, shipwreck_eval};
use serde::Serialize;

/// Agent names accepted by [`build_cribbage_agent`] /
/// [`build_shipwreck_agent`] / [`build_agent`], in display order.
///
/// ISMCTS agents additionally accept a parameter suffix after `:`, e.g.
/// `"ismcts-cribbage:iter=2000,c=1.4"`. The base name (before the `:`)
/// is what [`is_known_agent`] checks against.
///
/// `"http-remote"` is a game-agnostic interactive kind — it requires the
/// caller to supply a [`RemoteAgentTransport`] via [`AgentBuildCtx`].
/// The CLI rejects it (no coordinator); the HTTP server accepts it.
pub const KNOWN_AGENTS: &[&str] = &[
    "random",
    "http-remote",
    "llm",
    "stdio",
    "greedy-cribbage",
    "heuristic-cribbage",
    "ismcts-cribbage",
    "greedy-shipwreck",
    "heuristic-shipwreck",
    "ismcts-shipwreck",
    "greedy-greatgyre",
    "heuristic-greatgyre",
];

/// Default ISMCTS budget for agents built via [`build_cribbage_agent`]
/// / [`build_shipwreck_agent`] without explicit parameter overrides.
/// Tuned to clear R2.3's 65% bar while staying reasonable on 10K-game
/// benchmarks. Larger than `ISMCTSConfig::default()` because the
/// Phase-2 exit criterion needs strong play.
const DEFAULT_ISMCTS_ITERATIONS: u32 = 1000;

fn default_ismcts_config() -> ISMCTSConfig {
    ISMCTSConfig {
        iterations: DEFAULT_ISMCTS_ITERATIONS,
        exploration_c: std::f64::consts::SQRT_2,
        rollout_depth: 50,
        seed: 0,
    }
}

/// Per-seat build context used by the agent factories.
///
/// `seed` and `player` have the same meaning as the prior positional
/// arguments. The `Option<_>` fields carry optional-per-kind
/// dependencies the caller may supply — each is consumed by exactly
/// one agent kind and must be `Some` when that kind is built:
///
/// - `remote_transport` — required by `http-remote`; server-populated.
/// - `llm_client`, `llm_model`, `llm_max_tokens`, `llm_sidecar` —
///   required by `llm`; CLI-populated (see `crates/playtest-cli/src/
///   commands/play.rs`). `Arc<_>` around the client and the sidecar so
///   all LLM seats in one run share a single provider budget and a
///   single sidecar file.
/// - `stdio_cfg` — required by `stdio`; CLI-populated from
///   `--stdio-cmd` + `--stdio-arg`.
///
/// CLI helpers default every field to `None`; see [`AgentBuildCtx::cli`].
#[derive(Clone)]
pub struct AgentBuildCtx {
    pub seed: u64,
    pub player: PlayerId,
    pub remote_transport: Option<Arc<dyn RemoteAgentTransport>>,
    pub llm_client: Option<Arc<dyn LlmClient>>,
    pub llm_sidecar: Option<Arc<LlmSidecar>>,
    pub llm_model: Option<String>,
    pub llm_max_tokens: Option<u32>,
    pub stdio_cfg: Option<StdioAgentConfig>,
}

impl AgentBuildCtx {
    /// Ctx for a CLI caller — no transports, no LLM, no stdio.
    ///
    /// Preserved as the zero-configured constructor so every existing
    /// call site that only cares about `seed` + `player` still compiles
    /// unchanged.
    #[must_use]
    pub fn cli(seed: u64, player: PlayerId) -> Self {
        Self {
            seed,
            player,
            remote_transport: None,
            llm_client: None,
            llm_sidecar: None,
            llm_model: None,
            llm_max_tokens: None,
            stdio_cfg: None,
        }
    }
}

impl core::fmt::Debug for AgentBuildCtx {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("AgentBuildCtx")
            .field("seed", &self.seed)
            .field("player", &self.player)
            .field("remote_transport", &self.remote_transport.is_some())
            .field("llm_client", &self.llm_client.is_some())
            .field("llm_sidecar", &self.llm_sidecar.is_some())
            .field("llm_model", &self.llm_model)
            .field("llm_max_tokens", &self.llm_max_tokens)
            .field("stdio_cfg", &self.stdio_cfg)
            .finish()
    }
}

/// Composite result of agent construction: the `Box<dyn Agent<G>>` the
/// game loop drives, plus an optional `Box<dyn PostGameCritic<G>>` for
/// the post-game critique pass. `critic` is `Some` exactly for `llm`
/// seats; every other kind returns `None`.
///
/// Both handles point at the same underlying `LlmAgent<G>` instance
/// via an `Arc<Mutex<_>>` wrapper — scratch mutations from gameplay
/// are visible to the critic. See
/// `docs/solutions/architecture-patterns/sharing-mut-self-port-via-arc-mutex-2026-04-23.md`.
pub struct BuiltAgent<G: Game + ?Sized> {
    pub agent: Box<dyn Agent<G>>,
    pub critic: Option<Box<dyn PostGameCritic<G>>>,
}

impl<G: Game + ?Sized> BuiltAgent<G> {
    #[must_use]
    pub fn without_critic(agent: Box<dyn Agent<G>>) -> Self {
        Self {
            agent,
            critic: None,
        }
    }

    #[must_use]
    pub fn with_critic(
        agent: Box<dyn Agent<G>>,
        critic: Box<dyn PostGameCritic<G>>,
    ) -> Self {
        Self {
            agent,
            critic: Some(critic),
        }
    }

    /// Discard the critic and unwrap the underlying agent. Used by
    /// call sites that never ran a critique pass (pre-Phase-5 code).
    #[must_use]
    pub fn into_agent(self) -> Box<dyn Agent<G>> {
        self.agent
    }
}

/// Split an agent spec like `"ismcts-cribbage:iter=2000,c=1.4"` into
/// `("ismcts-cribbage", Some("iter=2000,c=1.4"))`. Names without a `:`
/// return `(name, None)`.
#[must_use]
pub fn split_agent_spec(spec: &str) -> (&str, Option<&str>) {
    match spec.split_once(':') {
        Some((name, params)) => (name, Some(params)),
        None => (spec, None),
    }
}

/// Scan an `llm:...` parameterization for a `provider=<name>` override
/// and return the value (trimmed). Returns `None` when the spec isn't
/// an `llm:` kind at all, or when no `provider=` override is present.
///
/// Used by [`validate_llm_provider_consistency`] and by the CLI to
/// pick the one provider the whole run must speak.
#[must_use]
pub fn llm_provider_from_spec(spec: &str) -> Option<&str> {
    let (base, params) = split_agent_spec(spec);
    if base != "llm" {
        return None;
    }
    let params = params?;
    for pair in params.split(',') {
        let pair = pair.trim();
        if let Some((k, v)) = pair.split_once('=')
            && k.trim() == "provider"
        {
            return Some(v.trim());
        }
    }
    None
}

/// Ensure every `llm:...` seat in `agent_names` names a compatible
/// `provider=` override. `model=` may differ; the Anthropic API accepts
/// per-request model selection. A single `ProductionLlmClient` is
/// constructed per run, so two seats demanding different providers
/// cannot both be served — reject the run before it starts.
///
/// # Errors
/// Returns an error naming the conflicting seats + providers when two
/// `llm:*` seats specify different `provider=` values.
pub fn validate_llm_provider_consistency(agent_names: &[String]) -> Result<()> {
    let mut first: Option<(usize, &str)> = None;
    for (i, spec) in agent_names.iter().enumerate() {
        let Some(p) = llm_provider_from_spec(spec) else {
            continue;
        };
        if let Some((j, prior)) = first {
            if prior != p {
                bail!(
                    "llm seats must agree on `provider`: seat {j} says `{prior}`, seat {i} \
                     says `{p}` — one ProductionLlmClient is built per run"
                );
            }
        } else {
            first = Some((i, p));
        }
    }
    Ok(())
}

/// Is `spec` either a base name in [`KNOWN_AGENTS`] or a parameterized
/// form `"<base>:params"` where `<base>` is in [`KNOWN_AGENTS`]?
#[must_use]
pub fn is_known_agent(spec: &str) -> bool {
    let (name, _) = split_agent_spec(spec);
    KNOWN_AGENTS.contains(&name)
}

fn ismcts_config_from_params(params: Option<&str>, seed: u64) -> Result<ISMCTSConfig> {
    let mut cfg = default_ismcts_config();
    cfg.seed = seed;
    if let Some(p) = params {
        let overrides = parse_config_overrides(p).map_err(|e| anyhow::anyhow!(e))?;
        // Only override non-default fields. `parse_config_overrides`
        // starts from `ISMCTSConfig::default()`, so we merge manually
        // against *that* baseline: if the parsed value differs from
        // the default, treat it as explicit.
        let d = ISMCTSConfig::default();
        if overrides.iterations != d.iterations {
            cfg.iterations = overrides.iterations;
        }
        if (overrides.exploration_c - d.exploration_c).abs() > f64::EPSILON {
            cfg.exploration_c = overrides.exploration_c;
        }
        if overrides.rollout_depth != d.rollout_depth {
            cfg.rollout_depth = overrides.rollout_depth;
        }
        if overrides.seed != d.seed {
            cfg.seed = overrides.seed;
        }
    }
    Ok(cfg)
}

fn build_http_remote<G>(ctx: &AgentBuildCtx) -> Result<Box<dyn Agent<G>>>
where
    G: Game + ?Sized + 'static,
    G::State: Send + Sync + 'static,
    G::PublicView: Send + Sync + 'static,
    G::Action: Send + Sync + serde::Serialize + 'static,
{
    let transport = ctx.remote_transport.as_ref().ok_or_else(|| {
        anyhow::anyhow!(
            "http-remote agent requires a server-provided transport; use POST /api/runs with \
             agents=[\"http-remote\", ...] instead of the CLI"
        )
    })?;
    Ok(Box::new(HttpRemoteAgent::<G>::new(
        ctx.player,
        transport.clone(),
    )))
}

/// Rules text bundled into both Cribbage-side and ShipWreck-side
/// system blocks. `include_str!` pulls the file in at compile time so
/// the binary needs no runtime asset plumbing.
///
/// Phase 3 simplification: the `rules_for_llm.md` files combine the
/// rules body with any card/state catalog the model needs. We therefore
/// set `card_catalog = Arc::from("")` below and let the one file be the
/// single source of truth. If future work splits the catalog out, add a
/// second `include_str!` and wire it through [`LlmAgentConfig::card_catalog`].
const CRIBBAGE_RULES_FOR_LLM: &str =
    include_str!("../../games/cribbage/rules_for_llm.md");
const SHIPWRECK_RULES_FOR_LLM: &str =
    include_str!("../../games/shipwreck/rules_for_llm.md");

/// Default output-token cap when the CLI doesn't supply one. Matches
/// the plan's "--llm-max-tokens defaults to 1024" note.
const DEFAULT_LLM_MAX_TOKENS: u32 = 1024;

/// Construct an [`LlmAgent<G>`] from `ctx`, validating the required
/// dependencies up front and producing a helpful CLI-pointing error if
/// any are absent.
///
/// Returns both roles: a `Box<dyn Agent<G>>` for the game loop and a
/// `Box<dyn PostGameCritic<G>>` for the Phase 5 post-game pass. Both
/// handles share one `LlmAgent<G>` instance so the critic reads the
/// scratch the gameplay path mutated.
fn build_llm<G>(
    ctx: &AgentBuildCtx,
    rules_text: &'static str,
) -> Result<BuiltAgent<G>>
where
    G: Game + ?Sized + Send + Sync + 'static,
    G::State: Send + Sync + 'static,
    G::PublicView: Send + Sync + Serialize + 'static,
    G::Action: Send + Sync + Serialize + 'static,
{
    let llm = ctx.llm_client.as_ref().ok_or_else(|| {
        anyhow!(
            "llm agent requires an LlmClient + --model; pass `--model <name>` (and, for \
             openai_compat, `--llm-base-url`) to `playtest play`"
        )
    })?;
    let model = ctx.llm_model.as_ref().ok_or_else(|| {
        anyhow!(
            "llm agent requires a model name; pass `--model <name>` to `playtest play`"
        )
    })?;
    let max_tokens = ctx.llm_max_tokens.unwrap_or(DEFAULT_LLM_MAX_TOKENS);

    let cfg = LlmAgentConfig {
        llm: llm.clone(),
        model: model.clone(),
        rules_text: Arc::from(rules_text),
        // See CRIBBAGE_RULES_FOR_LLM doc comment: the single rules file
        // carries the card catalog too, so we pass an empty string here.
        card_catalog: Arc::from(""),
        sidecar: ctx.llm_sidecar.clone(),
        max_tokens,
        temperature: None,
    };
    let agent = LlmAgent::<G>::new(ctx.player, cfg);
    let (agent_handle, critic_handle) = build_shared_handles(agent);
    Ok(BuiltAgent::with_critic(agent_handle, critic_handle))
}

/// Construct a [`StdioAgent<G>`] from `ctx`. The subprocess is *not*
/// spawned here — `StdioAgent::new` validates the command path and
/// defers spawn to the first `choose` call.
fn build_stdio<G>(
    ctx: &AgentBuildCtx,
    game_name: &'static str,
) -> Result<Box<dyn Agent<G>>>
where
    G: Game + ?Sized + Send + Sync + 'static,
    G::State: Send + Sync + 'static,
    G::PublicView: Send + Sync + Serialize + Clone + 'static,
    G::Action: Send + Sync + Serialize + Clone + 'static,
{
    let cfg = ctx.stdio_cfg.as_ref().ok_or_else(|| {
        anyhow!(
            "stdio agent requires a StdioAgentConfig; pass `--stdio-cmd <path>` (and any \
             `--stdio-arg <arg>` values) to `playtest play`"
        )
    })?;
    let agent = StdioAgent::<G>::new(ctx.player, game_name, cfg.clone())
        .map_err(|e| anyhow!("stdio agent build failed: {e}"))?;
    Ok(Box::new(agent))
}

/// Dispatch `llm:...` / `stdio:...` / per-game eval-free kinds that can
/// be built from `ctx` alone. Used by both per-game factories as a
/// shared fallback — centralises the game-agnostic arms (`random`,
/// `http-remote`, `llm`, `stdio`) in one place.
///
/// Per-game search agents (`greedy-cribbage`, `heuristic-cribbage`,
/// `greedy-shipwreck`, `heuristic-shipwreck`) must be built via
/// [`build_cribbage_agent`] / [`build_shipwreck_agent`] respectively,
/// because they need the concrete game's eval function at construction.
///
/// `llm` and `stdio` need a rules-text and a game-name handle from the
/// caller, so the per-game factories pre-resolve those and pass them in;
/// here we default to empty/"unknown" because the generic path isn't
/// the intended entry point for those kinds. Prefer calling
/// [`build_cribbage_agent`] / [`build_shipwreck_agent`] — they round-trip
/// through this function with the right game-specific context.
///
/// # Errors
/// Returns an error listing [`KNOWN_AGENTS`] if `name` is not
/// registered or is a per-game kind in disguise (e.g. trying to build
/// `greedy-cribbage` via the generic path). Returns an error for
/// `http-remote` when `ctx.remote_transport` is `None`, for `llm` when
/// the LLM client/model fields are `None`, and for `stdio` when
/// `ctx.stdio_cfg` is `None`.
pub fn build_agent<G>(name: &str, ctx: &AgentBuildCtx) -> Result<Box<dyn Agent<G>>>
where
    G: Game + ?Sized + Send + Sync + 'static,
    G::State: Send + Sync + 'static,
    G::PublicView: Send + Sync + Serialize + Clone + 'static,
    G::Action: Send + Sync + Serialize + Clone + 'static,
{
    build_agent_with_game_metadata::<G>(name, ctx, "", "unknown").map(BuiltAgent::into_agent)
}

/// Lower-level entry used by the per-game factories: accepts the
/// rules-text and game-name handles the `llm` / `stdio` builders need.
/// The generic [`build_agent`] is a thin shim that passes empty strings.
fn build_agent_with_game_metadata<G>(
    name: &str,
    ctx: &AgentBuildCtx,
    llm_rules_text: &'static str,
    stdio_game_name: &'static str,
) -> Result<BuiltAgent<G>>
where
    G: Game + ?Sized + Send + Sync + 'static,
    G::State: Send + Sync + 'static,
    G::PublicView: Send + Sync + Serialize + Clone + 'static,
    G::Action: Send + Sync + Serialize + Clone + 'static,
{
    match name {
        "random" => {
            let rng = ProductionRng::from_seed(ctx.seed);
            Ok(BuiltAgent::without_critic(Box::new(RandomAgent::<G, _>::new(rng))))
        }
        "http-remote" => Ok(BuiltAgent::without_critic(build_http_remote::<G>(ctx)?)),
        "llm" => build_llm::<G>(ctx, llm_rules_text),
        "stdio" => Ok(BuiltAgent::without_critic(build_stdio::<G>(ctx, stdio_game_name)?)),
        other => bail!(
            "unknown or non-generic agent: {other}; known: {}",
            KNOWN_AGENTS.join(", ")
        ),
    }
}

/// Build an agent typed for `CribbageGame`.
///
/// Accepts parameterized forms for ISMCTS, e.g.
/// `"ismcts-cribbage:iter=2000,c=1.4,depth=30"`. Unknown override keys
/// return an error.
///
/// # Errors
/// Returns an error if `spec` doesn't match a known agent or is a
/// ShipWreck-specific kind. Returns an error for `http-remote` when
/// `ctx.remote_transport` is `None`.
pub fn build_cribbage_agent(
    spec: &str,
    ctx: &AgentBuildCtx,
) -> Result<Box<dyn Agent<CribbageGame>>> {
    build_cribbage_agent_with_critic(spec, ctx).map(BuiltAgent::into_agent)
}

/// Phase-5 entry point: build a Cribbage agent and return both the
/// gameplay handle and the optional post-game-critique handle. For
/// `llm` seats, `critic` is `Some`; every other kind returns `None`.
///
/// # Errors
/// Same as [`build_cribbage_agent`].
pub fn build_cribbage_agent_with_critic(
    spec: &str,
    ctx: &AgentBuildCtx,
) -> Result<BuiltAgent<CribbageGame>> {
    let (name, params) = split_agent_spec(spec);
    match name {
        "random" | "http-remote" | "llm" | "stdio" => {
            build_agent_with_game_metadata::<CribbageGame>(
                name,
                ctx,
                CRIBBAGE_RULES_FOR_LLM,
                CribbageGame::NAME,
            )
        }
        "greedy-cribbage" => Ok(BuiltAgent::without_critic(Box::new(
            GreedyAgent::<CribbageGame>::new(ctx.player, cribbage_eval),
        ))),
        "heuristic-cribbage" => {
            let rng = ProductionRng::from_seed(ctx.seed);
            Ok(BuiltAgent::without_critic(Box::new(
                HeuristicAgent::<CribbageGame, _>::with_temperature(
                    ctx.player,
                    cribbage_eval,
                    rng,
                    DEFAULT_TEMPERATURE,
                ),
            )))
        }
        "ismcts-cribbage" => {
            let cfg = ismcts_config_from_params(params, ctx.seed)?;
            Ok(BuiltAgent::without_critic(Box::new(
                ISMCTSAgent::<CribbageGame>::with_eval(cfg, ctx.player, cribbage_eval),
            )))
        }
        other => bail!(
            "unknown agent: {other} (for cribbage); known: {}",
            KNOWN_AGENTS.join(", ")
        ),
    }
}

/// Build an agent typed for `ShipWreckGame`.
///
/// Accepts parameterized forms for ISMCTS, e.g.
/// `"ismcts-shipwreck:iter=1000,c=1.4"`.
///
/// # Errors
/// Returns an error if `spec` doesn't match a known agent or is a
/// Cribbage-specific kind. Returns an error for `http-remote` when
/// `ctx.remote_transport` is `None`.
pub fn build_shipwreck_agent(
    spec: &str,
    ctx: &AgentBuildCtx,
) -> Result<Box<dyn Agent<ShipWreckGame>>> {
    build_shipwreck_agent_with_critic(spec, ctx).map(BuiltAgent::into_agent)
}

/// Phase-5 entry point: build a ShipWreck agent and return both the
/// gameplay handle and the optional post-game-critique handle. For
/// `llm` seats, `critic` is `Some`; every other kind returns `None`.
///
/// # Errors
/// Same as [`build_shipwreck_agent`].
pub fn build_shipwreck_agent_with_critic(
    spec: &str,
    ctx: &AgentBuildCtx,
) -> Result<BuiltAgent<ShipWreckGame>> {
    let (name, params) = split_agent_spec(spec);
    match name {
        "random" | "http-remote" | "llm" | "stdio" => {
            build_agent_with_game_metadata::<ShipWreckGame>(
                name,
                ctx,
                SHIPWRECK_RULES_FOR_LLM,
                ShipWreckGame::NAME,
            )
        }
        "greedy-shipwreck" => Ok(BuiltAgent::without_critic(Box::new(
            GreedyAgent::<ShipWreckGame>::new(ctx.player, shipwreck_eval),
        ))),
        "heuristic-shipwreck" => {
            let rng = ProductionRng::from_seed(ctx.seed);
            Ok(BuiltAgent::without_critic(Box::new(
                HeuristicAgent::<ShipWreckGame, _>::with_temperature(
                    ctx.player,
                    shipwreck_eval,
                    rng,
                    DEFAULT_TEMPERATURE,
                ),
            )))
        }
        "ismcts-shipwreck" => {
            let cfg = ismcts_config_from_params(params, ctx.seed)?;
            Ok(BuiltAgent::without_critic(Box::new(
                ISMCTSAgent::<ShipWreckGame>::with_eval(cfg, ctx.player, shipwreck_eval),
            )))
        }
        other => bail!(
            "unknown agent: {other} (for shipwreck); known: {}",
            KNOWN_AGENTS.join(", ")
        ),
    }
}

/// Build an agent typed for `GreatGyreGame`.
///
/// # Errors
/// Returns an error if `spec` doesn't match a known agent or is a
/// Cribbage-/ShipWreck-specific kind. Returns an error for
/// `http-remote` when `ctx.remote_transport` is `None`.
pub fn build_greatgyre_agent(
    spec: &str,
    ctx: &AgentBuildCtx,
) -> Result<Box<dyn Agent<GreatGyreGame>>> {
    build_greatgyre_agent_with_critic(spec, ctx).map(BuiltAgent::into_agent)
}

/// Phase-5-shaped entry point: build a Great Gyre agent and return
/// both the gameplay handle and the optional post-game-critique
/// handle. `llm`/`stdio` route through the shared generic path with no
/// game-specific rules text (Great Gyre has no `rules_for_llm.md` —
/// out of scope for Unit 5, which only requires `random`/`http-remote`/
/// `greedy-greatgyre`/`heuristic-greatgyre`); `critic` is `None` for
/// every kind here as a result.
///
/// # Errors
/// Same as [`build_greatgyre_agent`].
pub fn build_greatgyre_agent_with_critic(
    spec: &str,
    ctx: &AgentBuildCtx,
) -> Result<BuiltAgent<GreatGyreGame>> {
    let (name, _params) = split_agent_spec(spec);
    match name {
        "random" | "http-remote" | "llm" | "stdio" => {
            build_agent_with_game_metadata::<GreatGyreGame>(name, ctx, "", GreatGyreGame::NAME)
        }
        "greedy-greatgyre" => Ok(BuiltAgent::without_critic(Box::new(
            GreedyAgent::<GreatGyreGame>::new(ctx.player, greatgyre_eval),
        ))),
        "heuristic-greatgyre" => {
            let rng = ProductionRng::from_seed(ctx.seed);
            Ok(BuiltAgent::without_critic(Box::new(
                HeuristicAgent::<GreatGyreGame, _>::with_temperature(
                    ctx.player,
                    greatgyre_eval,
                    rng,
                    DEFAULT_TEMPERATURE,
                ),
            )))
        }
        other => bail!(
            "unknown agent: {other} (for greatgyre); known: {}",
            KNOWN_AGENTS.join(", ")
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use playtest_agents::RemoteTransportError;
    use serde_json::Value as JsonValue;

    /// Minimal stub for tests that just need the `http-remote` code path
    /// to succeed in constructing an agent — no round-trip logic.
    struct NeverCalledTransport;

    #[async_trait]
    impl RemoteAgentTransport for NeverCalledTransport {
        async fn issue_prompt(
            &self,
            _seat: u8,
            _legal_json: Vec<JsonValue>,
        ) -> Result<u64, RemoteTransportError> {
            unreachable!("test does not exercise issue_prompt")
        }
        async fn await_action(
            &self,
            _seat: u8,
            _prompt_id: u64,
        ) -> Result<usize, RemoteTransportError> {
            unreachable!("test does not exercise await_action")
        }
    }

    #[test]
    fn http_remote_is_in_known_agents() {
        assert!(KNOWN_AGENTS.contains(&"http-remote"));
        assert!(is_known_agent("http-remote"));
    }

    #[test]
    fn build_http_remote_without_transport_fails_with_helpful_message() {
        let ctx = AgentBuildCtx::cli(42, 0);
        let err = build_cribbage_agent("http-remote", &ctx)
            .err()
            .expect("must fail without transport");
        let msg = err.to_string();
        assert!(
            msg.contains("requires a server-provided transport"),
            "message was: {msg}"
        );
    }

    #[test]
    fn build_http_remote_with_transport_succeeds_for_all_games() {
        let ctx = AgentBuildCtx {
            seed: 42,
            player: 0,
            remote_transport: Some(Arc::new(NeverCalledTransport)),
            llm_client: None,
            llm_sidecar: None,
            llm_model: None,
            llm_max_tokens: None,
            stdio_cfg: None,
        };
        build_cribbage_agent("http-remote", &ctx).expect("cribbage http-remote");
        build_shipwreck_agent("http-remote", &ctx).expect("shipwreck http-remote");
        build_greatgyre_agent("http-remote", &ctx).expect("greatgyre http-remote");
    }

    #[test]
    fn build_greatgyre_random_and_search_agents_succeed() {
        let ctx = AgentBuildCtx::cli(7, 0);
        build_greatgyre_agent("random", &ctx).expect("random");
        build_greatgyre_agent("greedy-greatgyre", &ctx).expect("greedy-greatgyre");
        build_greatgyre_agent("heuristic-greatgyre", &ctx).expect("heuristic-greatgyre");
    }

    #[test]
    fn build_greatgyre_rejects_other_games_per_game_kinds() {
        let ctx = AgentBuildCtx::cli(7, 0);
        let err = build_greatgyre_agent("greedy-shipwreck", &ctx)
            .err()
            .expect("shipwreck-specific kind rejected on the greatgyre path");
        assert!(err.to_string().contains("unknown agent"));
    }

    #[test]
    fn greatgyre_agents_are_in_known_agents() {
        assert!(KNOWN_AGENTS.contains(&"greedy-greatgyre"));
        assert!(KNOWN_AGENTS.contains(&"heuristic-greatgyre"));
        assert!(is_known_agent("greedy-greatgyre"));
        assert!(is_known_agent("heuristic-greatgyre"));
    }

    #[test]
    fn non_remote_agents_ignore_transport() {
        let ctx = AgentBuildCtx {
            seed: 42,
            player: 0,
            remote_transport: Some(Arc::new(NeverCalledTransport)),
            llm_client: None,
            llm_sidecar: None,
            llm_model: None,
            llm_max_tokens: None,
            stdio_cfg: None,
        };
        // Having a transport present shouldn't break non-remote kinds.
        build_cribbage_agent("random", &ctx).expect("random with transport present");
        build_cribbage_agent("greedy-cribbage", &ctx).expect("greedy with transport present");
    }

    #[test]
    fn generic_build_agent_rejects_per_game_kinds() {
        let ctx = AgentBuildCtx::cli(1, 0);
        let err = build_agent::<CribbageGame>("greedy-cribbage", &ctx)
            .err()
            .expect("per-game kinds not in generic path");
        let msg = err.to_string();
        assert!(msg.contains("unknown or non-generic"), "message was: {msg}");
    }
}

