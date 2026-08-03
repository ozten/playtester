//! `playtest play` — run N games of a registered game with two
//! registered agents, writing one JSONL event log per game.
//!
//! ## Phase 3: `llm` and `stdio` agent kinds
//!
//! - `--agents random,llm` requires `--model <name>` (defaults to a
//!   Haiku Anthropic model if `--model` is omitted) and reads
//!   `ANTHROPIC_API_KEY` from the environment. For OpenAI-compatible
//!   backends, pass `--llm-provider openai_compat --llm-base-url http://
//!   localhost:<port>/v1/`. The base URL is SSRF-guarded at adapter
//!   construction — only localhost hosts are allowed.
//! - `--agents random,stdio` requires `--stdio-cmd <path>` and one
//!   `--stdio-arg <arg>` per positional argument passed to the child.
//! - When any seat is `llm`, a sidecar `<out>/game-XXXX.llm.jsonl` is
//!   written alongside the main event log, with a `sidecar_header`
//!   first line carrying the SHA-256 of the in-repo `rules_for_llm.md`
//!   for that game (cache-stability auditing).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use clap::Args as ClapArgs;
use playtest_adapters::{
    ProductionFileSystem, ProductionGameEventSink, ProductionLlmClient, ProductionLlmConfig,
    ProviderKind, SecretString,
};
use playtest_agents::{
    CritiqueSidecar, CritiqueSidecarHeader, LlmSidecar, QuestionnaireSpec, SidecarHeader,
    StdioAgentConfig, default_questionnaire_v1, sha256_hex,
};
use playtest_ports::{FileSystem, LlmClient};
use playtest_registry::agent_registry::split_agent_spec;
use playtest_registry::game_registry::{
    RegisteredGame, lookup as lookup_game, player_count_range,
};
use playtest_registry::play::{LlmCliDeps, RunExtras, run_single_game_into_sink_with_extras};
use tokio::sync::Mutex as TokioMutex;
use url::Url;

/// Default Claude model when a seat is `llm` but `--model` isn't
/// supplied. Matches the plan's "exit criteria Haiku model".
const DEFAULT_LLM_MODEL: &str = "claude-haiku-4-5-20251001";

/// Provider selection for the `--llm-provider` flag.
#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum LlmProvider {
    Anthropic,
    OpenaiCompat,
}

/// CLI flags for the `play` subcommand.
#[derive(Debug, ClapArgs)]
pub struct PlayArgs {
    /// Game name (e.g. `cribbage`).
    #[arg(long)]
    pub game: String,

    /// Comma-separated agent names, one per player (e.g. `random,random`).
    #[arg(long)]
    pub agents: String,

    /// Number of games to play.
    #[arg(long)]
    pub games: u32,

    /// Master seed. Per-game seeds derive as `seed + game_index`.
    #[arg(long)]
    pub seed: u64,

    /// Output directory for JSONL event logs. Created if missing.
    #[arg(long)]
    pub out: PathBuf,

    /// Fan games out across rayon worker threads.
    #[arg(long)]
    pub parallel: bool,

    /// Pin the log header's `started_at` to a fixed Unix-ms value.
    /// When unset, the production clock is used — so two runs of the
    /// same command produce bit-identical event bodies but differing
    /// header timestamps. Tests that want bit-for-bit determinism
    /// across runs should pass `--fixed-time 0`.
    #[arg(long)]
    pub fixed_time: Option<u64>,

    // --- LLM-agent flags (active only when an `llm` seat is present) ---
    /// Model name (e.g. `claude-haiku-4-5-20251001`). Required when any
    /// seat is `llm`; defaults to a Haiku model if omitted.
    #[arg(long)]
    pub model: Option<String>,

    /// LLM provider: `anthropic` or `openai-compat`. Defaults to
    /// `anthropic`.
    #[arg(long, value_enum, default_value_t = LlmProvider::Anthropic)]
    pub llm_provider: LlmProvider,

    /// Base URL for `--llm-provider openai-compat`. Required for that
    /// provider; SSRF-guarded to localhost at adapter construction.
    #[arg(long)]
    pub llm_base_url: Option<Url>,

    /// Per-run token budget across all LLM seats. `0` or unset means
    /// unbounded.
    #[arg(long)]
    pub llm_budget_tokens: Option<u64>,

    /// Per-request output-token cap. Defaults to 1024 inside the
    /// registry when unset.
    #[arg(long)]
    pub llm_max_tokens: Option<u32>,

    /// Toggle Anthropic prompt caching for the rules-text system block.
    /// Defaults to `true`. No-op for `--llm-provider openai-compat`.
    #[arg(long, default_value_t = true)]
    pub llm_cache: bool,

    // --- Stdio-agent flags (active only when a `stdio` seat is present) ---
    /// Path to the stdio subprocess binary. Required when any seat is
    /// `stdio`.
    #[arg(long)]
    pub stdio_cmd: Option<PathBuf>,

    /// Argument passed to the stdio subprocess. Repeatable — arguments
    /// are forwarded in the order given.
    #[arg(long = "stdio-arg")]
    pub stdio_args: Vec<String>,

    // --- Phase 5 post-game critique flag ---
    /// When set alongside one or more `llm` seats, every LLM seat
    /// answers a standardized questionnaire after the game ends. The
    /// results go to `<out>/game-<idx>.critique.jsonl`. Non-LLM seats
    /// are untouched.
    ///
    /// Disabled by default because critique costs extra tokens.
    #[arg(long, default_value_t = false)]
    pub critique: bool,

    // --- Phase 6 restricted-play flag (ShipWreck only) ---
    /// Disable a specific ShipWreck event card for this run.
    /// Repeatable: `--shipwreck-disable-event shark
    /// --shipwreck-disable-event typhoon` disables both. Values
    /// must be `shark`, `typhoon`, or `flying_fish`. No-op when
    /// `--game` is not `shipwreck`.
    #[arg(long = "shipwreck-disable-event")]
    pub shipwreck_disabled_events: Vec<ShipwreckEventArg>,
}

/// Clap-facing value type for `--shipwreck-disable-event`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum ShipwreckEventArg {
    Shark,
    Typhoon,
    FlyingFish,
}

impl From<ShipwreckEventArg> for playtest_shipwreck::EventCard {
    fn from(arg: ShipwreckEventArg) -> Self {
        match arg {
            ShipwreckEventArg::Shark => Self::Shark,
            ShipwreckEventArg::Typhoon => Self::Typhoon,
            ShipwreckEventArg::FlyingFish => Self::FlyingFish,
        }
    }
}

/// Run the `play` command.
///
/// # Errors
/// Propagates any game-run error, I/O error, or registry-lookup error.
pub fn run(args: &PlayArgs) -> Result<()> {
    let game = lookup_game(&args.game)?;
    let agent_names: Vec<String> = args.agents.split(',').map(str::to_owned).collect();

    // Each game declares its player-count range; the CLI validates
    // against that rather than hardcoding a fixed count. Cribbage is
    // still 2..=2; ShipWreck is 2..=4.
    let (min, max) = player_count_range(&game);
    if agent_names.len() < min || agent_names.len() > max {
        bail!(
            "game `{}` expects {min}..={max} agents, got {}: `{}`",
            args.game,
            agent_names.len(),
            args.agents
        );
    }

    if args.games == 0 {
        return Ok(());
    }

    // Decide whether LLM / stdio infra needs to be provisioned.
    let has_llm_seat = agent_names.iter().any(|n| is_kind(n, "llm"));
    let has_stdio_seat = agent_names.iter().any(|n| is_kind(n, "stdio"));

    let llm_client: Option<Arc<dyn LlmClient>> = if has_llm_seat {
        Some(build_llm_client(args)?)
    } else {
        None
    };
    let llm_model = args
        .model
        .clone()
        .unwrap_or_else(|| DEFAULT_LLM_MODEL.to_owned());

    let stdio_cfg: Option<StdioAgentConfig> = if has_stdio_seat {
        let cmd = args.stdio_cmd.clone().ok_or_else(|| {
            anyhow::anyhow!(
                "a `stdio` seat requires --stdio-cmd <path> on the command line"
            )
        })?;
        Some(StdioAgentConfig {
            command: cmd,
            args: args.stdio_args.clone(),
        })
    } else {
        None
    };

    std::fs::create_dir_all(&args.out)?;

    // Phase 5: critique opts in via `--critique` AND requires at least
    // one `llm` seat (non-LLM seats have nothing to say). The shared
    // spec is built once per run so every game's sidecar header carries
    // a byte-identical `questionnaire_spec_sha256`.
    let critique_spec = if args.critique && has_llm_seat {
        Some(Arc::new(default_questionnaire_v1()))
    } else {
        None
    };
    if args.critique && !has_llm_seat {
        bail!(
            "--critique has no effect when no `llm` seat is present; add at least one llm \
             seat or remove --critique"
        );
    }

    let indices: Vec<u32> = (0..args.games).collect();
    let run_one = |idx: u32| -> Result<()> {
        let per_game_seed = args.seed.wrapping_add(u64::from(idx));
        let out_path = args.out.join(format!("game-{idx:04}.jsonl"));
        let sidecar_path = args.out.join(format!("game-{idx:04}.llm.jsonl"));
        let critique_path = args.out.join(format!("game-{idx:04}.critique.jsonl"));
        run_single_game(
            &game,
            &agent_names,
            per_game_seed,
            args.fixed_time,
            &out_path,
            llm_client.as_ref(),
            &llm_model,
            args.llm_max_tokens,
            stdio_cfg.as_ref(),
            if has_llm_seat { Some(&sidecar_path) } else { None },
            critique_spec.as_ref(),
            &args.shipwreck_disabled_events,
            if critique_spec.is_some() {
                Some(&critique_path)
            } else {
                None
            },
        )
    };

    if args.parallel {
        use rayon::prelude::*;
        indices.into_par_iter().try_for_each(run_one)?;
    } else {
        for idx in indices {
            run_one(idx)?;
        }
    }

    Ok(())
}

fn is_kind(spec: &str, base: &str) -> bool {
    split_agent_spec(spec).0 == base
}

fn build_llm_client(args: &PlayArgs) -> Result<Arc<dyn LlmClient>> {
    // Choose the provider. Anthropic pulls its key from
    // ANTHROPIC_API_KEY; openai-compat pulls from
    // PLAYTEST_OPENAI_COMPAT_KEY (matching the scrubbed-vars list the
    // stdio agent strips before spawning a child).
    let (provider, key_var): (ProviderKind, &str) = match args.llm_provider {
        LlmProvider::Anthropic => (ProviderKind::Anthropic, "ANTHROPIC_API_KEY"),
        LlmProvider::OpenaiCompat => {
            let base_url = args.llm_base_url.clone().ok_or_else(|| {
                anyhow::anyhow!(
                    "--llm-provider openai-compat requires --llm-base-url <url>"
                )
            })?;
            (
                ProviderKind::OpenAICompat { base_url },
                "PLAYTEST_OPENAI_COMPAT_KEY",
            )
        }
    };

    let api_key = SecretString::from_env(key_var);
    if api_key.is_empty() {
        bail!(
            "environment variable `{key_var}` is not set; llm agents require a provider \
             credential"
        );
    }

    let mut cfg = ProductionLlmConfig::new(provider, api_key);
    if let Some(budget) = args.llm_budget_tokens
        && budget > 0
    {
        cfg = cfg.with_budget_tokens(budget);
    }

    // `llm_cache` is threaded through per-call at `LlmAgent` construction
    // via the `SystemBlock.cache` flag. The production adapter honours
    // that flag for Anthropic and ignores it for openai-compat. The
    // flag on the CLI stays here for future wiring; today the `LlmAgent`
    // builds system blocks that unconditionally mark the rules block
    // as cacheable (see `playtest-agents/src/llm/prompt.rs`).
    let _ = args.llm_cache;

    let client = ProductionLlmClient::new(cfg)
        .context("building production LLM client; check API key + base URL validity")?;
    Ok(Arc::new(client))
}

/// Run one game. Dispatches on the registered game's variant.
#[allow(clippy::too_many_arguments)]
fn run_single_game(
    game: &RegisteredGame,
    agent_names: &[String],
    seed: u64,
    fixed_time: Option<u64>,
    out_path: &Path,
    llm_client: Option<&Arc<dyn LlmClient>>,
    llm_model: &str,
    llm_max_tokens: Option<u32>,
    stdio_cfg: Option<&StdioAgentConfig>,
    sidecar_path: Option<&Path>,
    critique_spec: Option<&Arc<QuestionnaireSpec>>,
    shipwreck_disabled_events: &[ShipwreckEventArg],
    critique_path: Option<&Path>,
) -> Result<()> {
    // If any seat is `llm`, open the sidecar and seal a header line
    // carrying the rules hash so cache-stability tooling can cross-ref
    // the bytes actually sent to the model.
    let sidecar = if let (Some(client), Some(path)) = (llm_client, sidecar_path) {
        let _ = client; // client is only used by the agent; kept here to gate on Some.
        let rules_text = rules_text_for_game(game);
        let header = SidecarHeader::new(
            game_name_for(game),
            seed,
            sha256_hex(rules_text.as_bytes()),
            sha256_hex(b""),
        );
        let fs: Arc<TokioMutex<dyn FileSystem + Send>> =
            Arc::new(TokioMutex::new(ProductionFileSystem::new()));
        // `LlmSidecar::new` is async because it writes the header via
        // the FileSystem port. Block on a small current-thread runtime
        // so this function stays synchronous for parallel-rayon reuse.
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .context("building sidecar runtime")?;
        let sidecar = rt
            .block_on(LlmSidecar::new(fs, path.to_path_buf(), header))
            .context("writing sidecar header")?;
        Some(Arc::new(sidecar))
    } else {
        None
    };

    // Phase 5: if --critique is enabled, open one critique sidecar per
    // game. Header carries the spec hash so the reporter can detect
    // cross-version drift.
    let critique_sidecar = if let (Some(spec), Some(path)) = (critique_spec, critique_path) {
        let rules_text = rules_text_for_game(game);
        let header = CritiqueSidecarHeader::new(
            game_name_for(game),
            seed,
            spec.sha256(),
            sha256_hex(rules_text.as_bytes()),
        );
        let fs: Arc<TokioMutex<dyn FileSystem + Send>> =
            Arc::new(TokioMutex::new(ProductionFileSystem::new()));
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .context("building critique sidecar runtime")?;
        let cs = rt
            .block_on(CritiqueSidecar::new(fs, path.to_path_buf(), header))
            .context("writing critique sidecar header")?;
        Some(Arc::new(cs))
    } else {
        None
    };

    let llm_deps = llm_client.map(|client| LlmCliDeps {
        client: client.clone(),
        sidecar: sidecar.clone(),
        model: llm_model.to_owned(),
        max_tokens: llm_max_tokens,
        critique_sidecar: critique_sidecar.clone(),
        critique_spec: critique_spec.cloned(),
    });

    let mut extras = RunExtras::new();
    if let Some(d) = llm_deps.as_ref() {
        extras = extras.with_llm_deps(d);
    }
    if let Some(c) = stdio_cfg {
        extras = extras.with_stdio_cfg(c);
    }
    if !shipwreck_disabled_events.is_empty() {
        let events: Vec<playtest_shipwreck::EventCard> = shipwreck_disabled_events
            .iter()
            .copied()
            .map(Into::into)
            .collect();
        extras = extras.with_disabled_shipwreck_events(events);
    }

    let fs = ProductionFileSystem::new();
    let mut sink = ProductionGameEventSink::new(fs, out_path);
    run_single_game_into_sink_with_extras(game, agent_names, seed, fixed_time, &extras, &mut sink)
}

fn rules_text_for_game(game: &RegisteredGame) -> &'static str {
    match game {
        RegisteredGame::Cribbage(_) => {
            include_str!("../../../games/cribbage/rules_for_llm.md")
        }
        RegisteredGame::ShipWreck(_) => {
            include_str!("../../../games/shipwreck/rules_for_llm.md")
        }
        // Great Gyre has no `rules_for_llm.md` yet — Unit 5 only wires
        // up `random`/`http-remote`/`greedy-greatgyre`/
        // `heuristic-greatgyre`; `llm`/`stdio` seats route through the
        // shared generic path with empty rules text (see
        // `agent_registry::build_greatgyre_agent_with_critic`'s doc
        // comment) rather than failing to compile here.
        RegisteredGame::GreatGyre(_) => "",
    }
}

fn game_name_for(game: &RegisteredGame) -> &'static str {
    match game {
        RegisteredGame::Cribbage(_) => "cribbage",
        RegisteredGame::ShipWreck(_) => "shipwreck",
        RegisteredGame::GreatGyre(_) => "greatgyre",
    }
}
