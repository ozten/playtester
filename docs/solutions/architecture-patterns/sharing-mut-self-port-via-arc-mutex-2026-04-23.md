---
title: "Sharing a mutable-self port across consumers via `Arc<Mutex<dyn Trait>>`"
date: 2026-04-23
category: architecture-patterns
module: playtest-ports
problem_type: architecture_pattern
component: service_object
severity: medium
applies_when:
  - "A port trait defines methods taking `&mut self` (writers, appenders, anything stateful)"
  - "The port handle must be shared across multiple consumers within one logical run"
  - "Consumers hold the handle behind `Arc` — typically async tasks that may run concurrently"
  - "Operation-atomic semantics on the shared resource are desirable (e.g. line-atomic appends)"
  - "The port trait is already defined and changing its signature to `&self` is not wanted"
tags:
  - rust
  - hexagonal
  - ports-and-adapters
  - arc-mutex
  - interior-mutability
  - async
  - shared-state
  - filesystem-port
related_components:
  - playtest-ports
  - playtest-agents
  - playtest-adapters
---

# Sharing a mutable-self port across consumers via `Arc<Mutex<dyn Trait>>`

## Context

Hexagonal architecture puts every external effect behind a port trait, and write-shaped ports (filesystem, event sinks, sidecar loggers) almost always expose `&mut self` methods because the underlying resource — a file handle, a buffered writer, a connection — is stateful. The tempting planning-time shape for sharing one such port across several in-run consumers is `Arc<dyn Port>`: it reads cleanly, compiles in isolation, and matches the mental model of "one writer, many holders". It also breaks on first contact. `Arc<dyn T>` hands out shared references; a method signed `fn append_line(&mut self, ...)` cannot be called through one.

The concrete case that surfaced it: a Phase-3 `LlmSidecar` that two `LlmAgent` instances (one per player) must share so both append to a single `<run>/games/<gid>.llm.jsonl` file with line-atomic JSON lines. Planning pseudocode reached for `Arc<dyn FileSystem>` and put a `Mutex<Vec<u8>>` beside it as a buffer. The buffer is the wrong place for the lock.

This wasn't a novel problem in the repo — the `Record*` adapters had already solved the single-ownership version of it in Phase 0–1. `RecordFileSystem` (`crates/playtest-adapters/src/filesystem/record.rs`) uses `RefCell<TapeWriter>` for single-threaded interior mutability of an owned resource. `RecordLlmClient` uses `std::sync::Mutex<TapeWriter>` specifically because `LlmClient::complete(&self)` takes a shared reference but the tape needs `&mut`. That adapter's doc comment — *"async clients are typically shared, but the tape needs `&mut`"* — is the spiritual ancestor of this pattern. (session history)

## Guidance

When a port's methods take `&mut self` and multiple consumers within one run must share a single handle, wrap the port itself in a mutex:

```rust
Arc<tokio::sync::Mutex<dyn FileSystem + Send>>
```

Three rules govern the shape:

1. **Wrap the port, not a byte buffer.** The interior mutability must sit at the boundary where `&mut self` is actually needed. A side buffer doesn't let you call the trait method, and it hides the real concurrency boundary.
2. **Pick the mutex to match the caller.** Use `tokio::sync::Mutex` when the guard is held across `.await`; use `std::sync::Mutex` or `parking_lot::Mutex` on purely synchronous paths. Holding a `std::sync::Mutex` guard across an await point is a soundness bug that clippy flags.
3. **Keep the `+ Send` bound.** `Arc<Mutex<dyn Trait>>` is only `Send + Sync` when the erased trait object is itself `Send`. Drop the bound and the error surfaces far away — at the first `tokio::spawn` that moves the `Arc`, with a multi-line "future is not `Send`" complaint pointing at the await site rather than the trait object.

## Why This Matters

Catching this at planning time saves an implementation iteration. Pseudocode that types `Arc<dyn Port>` against a mutating port will fail to compile the moment a real call site is written, and the fix ripples through every downstream signature that accepted the wrong type. The Phase 3 plan itself illustrates the blind spot — two different sections pinned two mutually-inconsistent mutex placements (one wrapping a `FileHandle`, one leaving the mutex beside a bare `Arc<dyn FileSystem>`), neither of which actually compiles. Either shape would have failed at first call. (session history)

Line-atomic write semantics fall out for free once the mutex is on the port. A turn-based game loop may not concurrently invoke two sidecars today, but the mutex guarantees no torn JSONL lines if a future observation-streaming or parallel-evaluation mode lands. Defense-in-depth at zero cost.

The `+ Send` bound looks cosmetic until it isn't. Its absence doesn't fail where it's declared; it fails where the `Arc` crosses a spawn boundary. Declaring it at the port-object boundary keeps the compile errors co-located with the type decision.

## When to Apply

- Any hexagonal port whose trait takes `&mut self` on the methods you'll actually call.
- Multiple consumers within one logical scope (game, request, session) need the same handle — typically output ports: writers, event sinks, metric recorders.
- Async call sites — reach for `tokio::sync::Mutex` when the guard lives across `.await`.
- **Do not apply** when the trait is already `&self`-only. `LlmClient::complete(&self, req)` is fine as `Arc<dyn LlmClient>`; wrapping a `&self`-only trait in a mutex just serializes independent requests for no payoff.

The `FileSystem` port in this repo is borderline: `read`/`exists` take `&self` but `write`/`append_line` take `&mut self`. That asymmetry is what forces the `Arc<Mutex<_>>` shape for the write-shaped use case specifically. Read-only consumers can still use `Arc<dyn FileSystem>` directly if they only call the `&self` methods — though in practice the sharing discipline is easier to reason about when one wrapping choice applies per handle.

## Examples

**Anti-pattern — `Arc<dyn T>` against a `&mut self` method:**

```rust
// Broken pseudocode (from the original plan)
pub struct LlmSidecar {
    fs: Arc<dyn FileSystem>,
    path: PathBuf,
    inner: tokio::sync::Mutex<Vec<u8>>, // wrong place for the lock
}

impl LlmSidecar {
    pub async fn append(&self, line: &str) -> std::io::Result<()> {
        self.fs.append_line(&self.path, line) // E0596: cannot borrow
                                              // data in an `Arc` as mutable
    }
}
```

The compiler says roughly `cannot borrow data in an 'Arc' as mutable` / `trait 'DerefMut' is not implemented for 'Arc<dyn FileSystem>'`. No amount of `Arc::get_mut` rescues this once the refcount exceeds one — which is the whole point of sharing.

**Correct pattern — mutex on the port itself** (`crates/playtest-agents/src/llm/sidecar.rs`):

```rust
pub struct LlmSidecar {
    fs: Arc<tokio::sync::Mutex<dyn FileSystem + Send>>,
    path: PathBuf,
}

impl LlmSidecar {
    pub async fn append_call(&self, record: &LlmCallRecord)
        -> Result<(), SidecarError>
    {
        let line = serde_json::to_string(record)?;
        let mut fs = self.fs.lock().await; // acquires &mut dyn FileSystem
        fs.append_line(&self.path, &line)?;
        Ok(())
    }
}
```

Each player's `LlmAgent` holds a clone of the `Arc`; `lock().await` serializes writers so JSONL lines stay whole, and the `+ Send` bound keeps the composite type `Send + Sync` so it can cross spawn boundaries cleanly.

**Anti-pattern of the opposite kind — over-wrapping:** if your port trait is `&self` throughout (query-shaped, read-only, or internally synchronized), `Arc<Mutex<dyn T>>` adds contention for no payoff. `Arc<dyn LlmClient>` is the right shape there; the trait already promises shared-reference concurrency. The Phase 3 plan explicitly does this — `ctx.llm_client: Option<Arc<dyn LlmClient>>` is bare `Arc` because `complete(&self, req)` doesn't mutate.

**Prior-art precedents in the adapter layer** for owned (non-shared) cases:

```rust
// crates/playtest-adapters/src/filesystem/record.rs
struct RecordFileSystem<F: FileSystem> {
    inner: F,
    tape: RefCell<TapeWriter>, // owned resource, single-threaded interior mutability
}

// crates/playtest-adapters/src/llm_client/record.rs
struct RecordLlmClient {
    inner: Arc<dyn LlmClient>,
    tape: std::sync::Mutex<TapeWriter>, // &self trait method + &mut tape
}
```

`RefCell` works for owned-not-shared; `std::sync::Mutex` works when the lock is brief and sync; `tokio::sync::Mutex` works when the lock crosses `.await`. Pick per consumer, not per port.

## Related

- [`blocking-loop-to-main-runtime-via-transport-trait-2026-04-22.md`](blocking-loop-to-main-runtime-via-transport-trait-2026-04-22.md) — sibling learning from Phase 2.5 that passes `Arc<dyn RemoteAgentTransport>` (a `&self` trait, no mutex needed). That doc's phrasing *"Non-deterministic transport lives in `playtest-agents`, not `playtest-ports`"* explains one branch of the decision tree; this doc documents the other branch — when you *do* keep something in `playtest-ports` and it takes `&mut self`, here is the shared-handle shape.
- Prior-art precedents: `crates/playtest-adapters/src/filesystem/record.rs` (`RefCell` for owned interior mutability) and `crates/playtest-adapters/src/llm_client/record.rs` (`std::sync::Mutex` for `&self` trait method with `&mut` internal state).
- Object-safety guard: `crates/playtest-ports/tests/traits_object_safe.rs` already asserts `&mut dyn FileSystem` is valid, which is why this was a clean one-line type change and not a cascading trait-surgery problem.
