//! Compile-time assertions that the ports intended to be object-safe are.
//!
//! If one of these functions stops compiling, a port trait has gained a
//! generic method, an `impl Trait` return, or a `where Self: Sized`-less
//! generic — any of which breaks `&mut dyn Port` callers. The `Rng`
//! shuffle method uses `where Self: Sized` precisely so this test still
//! passes.
//!
//! `LlmClient` is intentionally omitted — the lib docs state it is not
//! guaranteed object-safe, and `async_trait` may change that contract in
//! the future.

use playtest_ports::{Clock, EventSink, FileSystem, Rng};

fn _assert_clock_object_safe(_: &mut dyn Clock) {}
fn _assert_rng_object_safe(_: &mut dyn Rng) {}
fn _assert_filesystem_object_safe(_: &dyn FileSystem) {}
fn _assert_filesystem_mut_object_safe(_: &mut dyn FileSystem) {}
fn _assert_event_sink_object_safe(_: &mut dyn EventSink) {}

#[test]
fn ports_are_object_safe() {
    // The real signal is that this file compiles at all. The runtime
    // assertion below only exists so `cargo test` surfaces a visible test.
    // See the `_assert_*` fns above for the compile-time checks.
    assert!(std::mem::size_of::<&dyn Clock>() > 0);
}
