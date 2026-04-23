//! `playtest critique-code <run-dir>`: offline Phase-5 coder pass.
//!
//! For each `<gid>.critique.jsonl` in the run directory, for every
//! `questionnaire_response` record with non-empty open-ended fields,
//! issues one coder-LLM call and appends a `coded_tag` record to the
//! same file. Idempotent: running twice without `--overwrite` is a
//! no-op on seats that already have a `coded_tag` record.
//!
//! Failures are per-game and per-seat — a malformed record logs to
//! stderr and the pass continues.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use clap::Args as ClapArgs;
use playtest_adapters::{ProductionFileSystem, ProductionLlmClient, ProductionLlmConfig,
    ProviderKind, SecretString};
use playtest_agents::{CodedTagRecord, code_once};
use playtest_ports::{FileSystem, LlmClient};
use tokio::sync::Mutex as TokioMutex;
use url::Url;

use super::play::LlmProvider;

const DEFAULT_CODER_MAX_TOKENS: u32 = 1024;

/// Arguments for `playtest critique-code`.
#[derive(Debug, ClapArgs)]
pub struct CritiqueCodeArgs {
    /// Directory containing `game-NNNN.critique.jsonl` sidecars.
    /// Typically the `--out` directory from a prior `playtest play`
    /// run.
    #[arg(long)]
    pub run_dir: PathBuf,

    /// LLM model for the coder pass. Required.
    #[arg(long)]
    pub model: String,

    /// LLM provider: `anthropic` or `openai-compat`. Defaults to
    /// `anthropic`.
    #[arg(long, value_enum, default_value_t = LlmProvider::Anthropic)]
    pub llm_provider: LlmProvider,

    /// Base URL for `--llm-provider openai-compat`. Required for that
    /// provider.
    #[arg(long)]
    pub llm_base_url: Option<Url>,

    /// Per-request output-token cap. Default 1024.
    #[arg(long)]
    pub llm_max_tokens: Option<u32>,

    /// Per-run token budget. `0` or unset means unbounded.
    #[arg(long)]
    pub coder_budget_tokens: Option<u64>,

    /// Re-run the coder on seats that already have a `coded_tag`
    /// record. By default, seats with existing coded tags are
    /// skipped.
    #[arg(long, default_value_t = false)]
    pub overwrite: bool,
}

/// Run `playtest critique-code`.
///
/// # Errors
/// Returns configuration errors (missing env key, bad base URL) up
/// front. Per-file failures log to stderr and do not fail the run.
pub fn run(args: &CritiqueCodeArgs) -> Result<()> {
    let llm = build_coder_client(args)?;
    let max_tokens = args.llm_max_tokens.unwrap_or(DEFAULT_CODER_MAX_TOKENS);
    let fs: Arc<TokioMutex<dyn FileSystem + Send>> =
        Arc::new(TokioMutex::new(ProductionFileSystem::new()));

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("building critique-code runtime")?;

    let paths = discover_critique_files(&args.run_dir)?;
    if paths.is_empty() {
        eprintln!(
            "no .critique.jsonl files found under {} — did you pass the right --run-dir?",
            args.run_dir.display()
        );
        return Ok(());
    }

    let mut total_codings = 0_u64;
    let mut total_skipped = 0_u64;
    let mut total_failed = 0_u64;
    for path in &paths {
        match rt.block_on(code_one_file(
            llm.as_ref(),
            fs.clone(),
            path,
            &args.model,
            max_tokens,
            args.overwrite,
        )) {
            Ok(outcome) => {
                total_codings += outcome.coded;
                total_skipped += outcome.skipped;
                total_failed += outcome.failed;
            }
            Err(e) => {
                eprintln!("critique-code: {}: {e}", path.display());
                total_failed += 1;
            }
        }
    }
    eprintln!(
        "critique-code: coded={total_codings} skipped={total_skipped} failed={total_failed} \
         files={}",
        paths.len()
    );
    Ok(())
}

fn build_coder_client(args: &CritiqueCodeArgs) -> Result<Arc<dyn LlmClient>> {
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
            "environment variable `{key_var}` is not set; coder pass requires a provider \
             credential"
        );
    }

    let mut cfg = ProductionLlmConfig::new(provider, api_key);
    if let Some(budget) = args.coder_budget_tokens
        && budget > 0
    {
        cfg = cfg.with_budget_tokens(budget);
    }
    let client = ProductionLlmClient::new(cfg)
        .context("building coder production LLM client")?;
    Ok(Arc::new(client))
}

fn discover_critique_files(run_dir: &Path) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    let walker = std::fs::read_dir(run_dir)
        .with_context(|| format!("reading {}", run_dir.display()))?;
    for entry in walker {
        let entry = entry?;
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "jsonl")
            && path.file_name().is_some_and(|n| {
                n.to_string_lossy().ends_with(".critique.jsonl")
            })
        {
            out.push(path);
        }
    }
    out.sort();
    Ok(out)
}

#[derive(Debug, Default)]
struct FileOutcome {
    coded: u64,
    skipped: u64,
    failed: u64,
}

async fn code_one_file(
    llm: &dyn LlmClient,
    fs: Arc<TokioMutex<dyn FileSystem + Send>>,
    path: &Path,
    model: &str,
    max_tokens: u32,
    overwrite: bool,
) -> Result<FileOutcome> {
    // Read the sidecar. We go direct through std::fs here because the
    // read happens once up front; the appends later go through the
    // FileSystem port to preserve the adapter discipline.
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("reading {}", path.display()))?;

    // Collect (seat, open_ended) for every questionnaire_response
    // record, plus the seats that already have a coded_tag record.
    let mut pending: Vec<(u8, BTreeMap<String, String>)> = Vec::new();
    let mut already_coded: std::collections::BTreeSet<u8> =
        std::collections::BTreeSet::new();
    for (line_no, line) in text.lines().enumerate() {
        if line.is_empty() {
            continue;
        }
        let value: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(e) => {
                eprintln!(
                    "critique-code: {}: line {line_no}: ignoring malformed JSON: {e}",
                    path.display()
                );
                continue;
            }
        };
        match value.get("kind").and_then(|v| v.as_str()) {
            Some("questionnaire_response") => {
                let Some(seat_u64) = value.get("seat").and_then(serde_json::Value::as_u64)
                else {
                    continue;
                };
                let Ok(seat) = u8::try_from(seat_u64) else {
                    continue;
                };
                let open_ended_val = value.get("open_ended");
                let open_ended: BTreeMap<String, String> = open_ended_val
                    .and_then(|v| serde_json::from_value(v.clone()).ok())
                    .unwrap_or_default();
                if open_ended.values().all(|s| s.trim().is_empty()) {
                    continue;
                }
                pending.push((seat, open_ended));
            }
            Some("coded_tag") => {
                if let Some(seat_u64) = value.get("seat").and_then(serde_json::Value::as_u64)
                    && let Ok(seat) = u8::try_from(seat_u64)
                {
                    already_coded.insert(seat);
                }
            }
            _ => {}
        }
    }

    let mut outcome = FileOutcome::default();
    for (seat, open_ended) in pending {
        if !overwrite && already_coded.contains(&seat) {
            outcome.skipped += 1;
            continue;
        }
        match code_once(llm, &open_ended, model, max_tokens).await {
            Ok(result) => {
                if !result.dropped_tags.is_empty() {
                    eprintln!(
                        "critique-code: {}: seat {seat}: dropped out-of-taxonomy tags: {:?}",
                        path.display(),
                        result.dropped_tags
                    );
                }
                let record = CodedTagRecord::new(seat, result.accepted);
                let line = serde_json::to_string(&record)
                    .expect("CodedTagRecord Serialize is infallible");
                fs.lock()
                    .await
                    .append_line(path, &line)
                    .with_context(|| format!("appending coded_tag to {}", path.display()))?;
                outcome.coded += 1;
            }
            Err(e) => {
                eprintln!("critique-code: {}: seat {seat}: coder failed: {e}", path.display());
                outcome.failed += 1;
            }
        }
    }
    Ok(outcome)
}

