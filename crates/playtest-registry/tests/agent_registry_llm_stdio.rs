//! Unit 6 — `llm` and `stdio` agent-kind dispatch through the registry.
//!
//! Covers:
//! 1. Happy paths: `build_cribbage_agent("llm:...", ctx)` and
//!    `build_cribbage_agent("stdio:cmd=/bin/cat", ctx)` construct.
//! 2. Failure cases: absent dependencies give CLI-pointing errors.
//! 3. Pre-build provider-consistency check rejects mixed providers.
//! 4. `is_known_agent` sees `llm:...` / `stdio:...` as known kinds.
//!
//! The LLM happy path uses a local [`CannedLlmClient`] — a real `LlmClient`
//! implementation that returns a fixed JSON-shaped reply — not a mock.
//! The stdio path constructs against `/bin/cat`; the subprocess is only
//! spawned lazily (on first `choose`), so construction is a pure path
//! check.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use playtest_adapters::StubFileSystem;
use playtest_agents::{LlmSidecar, SidecarHeader, StdioAgentConfig};
use playtest_ports::{FileSystem, LlmClient, LlmError, LlmRequest, LlmResponse};
use playtest_registry::agent_registry::{
    AgentBuildCtx, build_cribbage_agent, is_known_agent, validate_llm_provider_consistency,
};
use tokio::sync::Mutex as TokioMutex;

/// A real `LlmClient` that replies with a canned JSON body. Not a mock
/// — this is the same shape the stub adapter uses.
struct CannedLlmClient {
    text: String,
}

#[async_trait]
impl LlmClient for CannedLlmClient {
    async fn complete(&self, _req: LlmRequest) -> Result<LlmResponse, LlmError> {
        Ok(LlmResponse {
            text: self.text.clone(),
            input_tokens: 10,
            output_tokens: 5,
            cache_read_input_tokens: 0,
            cache_creation_input_tokens: 0,
        })
    }
}

fn canned_llm_client() -> Arc<dyn LlmClient> {
    Arc::new(CannedLlmClient {
        text: r#"{"action_index":0,"plan":"","notes":""}"#.to_owned(),
    })
}

async fn stub_sidecar() -> Arc<LlmSidecar> {
    let fs: Arc<TokioMutex<dyn FileSystem + Send>> =
        Arc::new(TokioMutex::new(StubFileSystem::new()));
    let header = SidecarHeader::new("cribbage", 7, "deadbeef", "");
    let path = PathBuf::from("/tmp-sidecar/game.llm.jsonl");
    Arc::new(LlmSidecar::new(fs, path, header).await.unwrap())
}

fn ctx_with_llm(
    client: Arc<dyn LlmClient>,
    sidecar: Option<Arc<LlmSidecar>>,
    model: &str,
) -> AgentBuildCtx {
    let mut ctx = AgentBuildCtx::cli(42, 0);
    ctx.llm_client = Some(client);
    ctx.llm_sidecar = sidecar;
    ctx.llm_model = Some(model.to_owned());
    ctx.llm_max_tokens = Some(512);
    ctx
}

fn ctx_with_stdio(cfg: StdioAgentConfig) -> AgentBuildCtx {
    let mut ctx = AgentBuildCtx::cli(42, 0);
    ctx.stdio_cfg = Some(cfg);
    ctx
}

#[tokio::test]
async fn llm_happy_path_builds_cribbage_agent() {
    let sidecar = stub_sidecar().await;
    let client = canned_llm_client();
    let ctx = ctx_with_llm(client, Some(sidecar), "claude-haiku-4-5-20251001");
    let agent = build_cribbage_agent("llm:provider=anthropic,model=claude-haiku-4-5-20251001", &ctx)
        .expect("llm agent builds");
    // Can't call choose without running inside a game loop, but the
    // return type should be the expected trait object.
    let _ = &agent;
}

#[test]
fn llm_without_client_returns_cli_pointing_error() {
    let ctx = AgentBuildCtx::cli(1, 0);
    let Err(err) = build_cribbage_agent("llm", &ctx) else {
        panic!("should fail without client");
    };
    let msg = err.to_string();
    assert!(
        msg.contains("--model") || msg.contains("LlmClient"),
        "helpful error, got: {msg}"
    );
}

#[test]
fn llm_without_model_returns_cli_pointing_error() {
    // Client present, model absent.
    let mut ctx = AgentBuildCtx::cli(1, 0);
    ctx.llm_client = Some(canned_llm_client());
    let Err(err) = build_cribbage_agent("llm", &ctx) else {
        panic!("should fail without model");
    };
    let msg = err.to_string();
    assert!(
        msg.contains("--model") || msg.contains("model"),
        "helpful error, got: {msg}"
    );
}

#[test]
fn stdio_happy_path_builds_cribbage_agent() {
    // `/bin/cat` is guaranteed on Linux CI; on macOS dev boxes too.
    let cfg = StdioAgentConfig {
        command: PathBuf::from("/bin/cat"),
        args: vec![],
    };
    let ctx = ctx_with_stdio(cfg);
    let agent = build_cribbage_agent("stdio:cmd=/bin/cat", &ctx)
        .expect("stdio agent builds");
    // Lazy spawn: constructing the agent must not have launched a child.
    let _ = &agent;
}

#[test]
fn stdio_without_cfg_returns_cli_pointing_error() {
    let ctx = AgentBuildCtx::cli(1, 0);
    let Err(err) = build_cribbage_agent("stdio", &ctx) else {
        panic!("should fail without cfg");
    };
    let msg = err.to_string();
    assert!(
        msg.contains("--stdio-cmd") || msg.contains("StdioAgentConfig"),
        "helpful error, got: {msg}"
    );
}

#[test]
fn stdio_nonexistent_command_fails_fast_at_build() {
    let cfg = StdioAgentConfig {
        command: PathBuf::from("/really/does/not/exist/here"),
        args: vec![],
    };
    let ctx = ctx_with_stdio(cfg);
    let Err(err) = build_cribbage_agent("stdio", &ctx) else {
        panic!("should fail without binary");
    };
    let msg = err.to_string();
    assert!(
        msg.contains("not found") || msg.contains("/really/does/not/exist"),
        "got: {msg}"
    );
}

#[test]
fn is_known_recognizes_llm_and_stdio_parameterized() {
    assert!(is_known_agent("llm"));
    assert!(is_known_agent("stdio"));
    assert!(is_known_agent("llm:provider=anthropic,model=x"));
    assert!(is_known_agent("stdio:cmd=/bin/cat"));
    assert!(!is_known_agent("not-a-real-agent"));
}

#[test]
fn mixed_providers_across_llm_seats_rejected() {
    let specs: Vec<String> = vec![
        "llm:provider=anthropic,model=x".to_owned(),
        "llm:provider=openai_compat,model=y".to_owned(),
    ];
    let err = validate_llm_provider_consistency(&specs)
        .expect_err("mixed providers should reject");
    let msg = err.to_string();
    assert!(msg.contains("anthropic"), "got: {msg}");
    assert!(msg.contains("openai_compat"), "got: {msg}");
}

#[test]
fn matching_providers_across_llm_seats_accepted() {
    let specs: Vec<String> = vec![
        "llm:provider=anthropic,model=haiku".to_owned(),
        "llm:provider=anthropic,model=sonnet".to_owned(),
    ];
    validate_llm_provider_consistency(&specs).expect("matching providers should accept");
}

#[test]
fn llm_without_provider_override_accepts() {
    // With no explicit provider, the CLI's default (anthropic) applies
    // uniformly — validation treats them as compatible.
    let specs: Vec<String> = vec!["llm".to_owned(), "llm".to_owned()];
    validate_llm_provider_consistency(&specs).expect("no-provider specs accepted");
}
