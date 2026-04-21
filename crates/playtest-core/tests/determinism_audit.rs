//! Grep-based guardrail: fails if the engine or any game crate imports
//! a non-deterministic system call outside the sanctioned `Clock` /
//! `Rng` adapter layer.
//!
//! This is not a proof — clippy or manual review could always slip
//! something through — but it catches 95% of determinism regressions
//! at test time, on every PR, without needing a full soak run.
//!
//! Scope: `src/` trees of `playtest-core` and every crate under
//! `crates/games/`. `tests/` directories are excluded because tests
//! are allowed to call `Instant::now` for timing assertions.
//! Adapter crates are excluded because `SystemTime::now` is the
//! *point* of the `ProductionClock` adapter.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// Forbidden substrings. Any literal match in engine/game `src/`
/// code is a determinism regression.
const FORBIDDEN: &[&str] = &[
    "SystemTime::now",
    "thread_rng",
    "Instant::now",
    "rand::random",
];

#[test]
fn engine_and_games_do_not_call_system_nondeterminism() {
    // Starting at `tests/` in playtest-core, the workspace root is
    // two levels up.
    let workspace_root = env!("CARGO_MANIFEST_DIR");
    let workspace_root = Path::new(workspace_root)
        .parent()
        .and_then(Path::parent)
        .expect("workspace root two levels above playtest-core");

    let mut hits: Vec<(PathBuf, usize, String, &'static str)> = Vec::new();

    let mut roots = vec![workspace_root.join("crates/playtest-core/src")];
    for dir in std::fs::read_dir(workspace_root.join("crates/games")).expect("crates/games/ exists")
    {
        let dir = dir.unwrap();
        let src = dir.path().join("src");
        if src.is_dir() {
            roots.push(src);
        }
    }

    for root in &roots {
        scan_dir(root, &mut hits);
    }

    if !hits.is_empty() {
        let mut msg = String::from(
            "determinism audit: forbidden non-deterministic calls found in engine/game source:\n",
        );
        for (path, line, content, pattern) in &hits {
            writeln!(
                msg,
                "  {}:{line}: `{pattern}` in `{}`",
                path.display(),
                content.trim()
            )
            .expect("writing to String never fails");
        }
        panic!("{msg}");
    }
}

fn scan_dir(dir: &Path, hits: &mut Vec<(PathBuf, usize, String, &'static str)>) {
    for entry in std::fs::read_dir(dir).unwrap_or_else(|e| panic!("reading {}: {e}", dir.display()))
    {
        let entry = entry.unwrap();
        let path = entry.path();
        if path.is_dir() {
            scan_dir(&path, hits);
        } else if path.extension().and_then(|s| s.to_str()) == Some("rs") {
            scan_file(&path, hits);
        }
    }
}

fn scan_file(path: &Path, hits: &mut Vec<(PathBuf, usize, String, &'static str)>) {
    let content =
        std::fs::read_to_string(path).unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
    for (idx, line) in content.lines().enumerate() {
        // Skip comments and doc lines — they legitimately *mention*
        // the forbidden patterns (and the whole point of the project
        // is to document why you shouldn't call them).
        let trimmed = line.trim_start();
        if trimmed.starts_with("//") || trimmed.starts_with("///") || trimmed.starts_with("//!") {
            continue;
        }
        for pattern in FORBIDDEN {
            if line.contains(pattern) {
                hits.push((path.to_path_buf(), idx + 1, line.to_owned(), pattern));
            }
        }
    }
}
