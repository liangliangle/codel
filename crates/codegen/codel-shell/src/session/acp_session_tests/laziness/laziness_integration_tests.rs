//! End-to-end tests for `maybe_fire_laziness_check`.
//! Each test drives the actor against a non-listening `http://localhost` base URL.
//! The unified path's `prepare_chat_completion().conversation_collect()` call surfaces the connection failure as the `ClassifierError` abort.
//! The tests observe state mutations and the per-test `events.jsonl`.
//!
//! Tests that depend on a *successful* classifier response are out of scope here.
//! They would need a real `SamplerActor` responding with a stubbed verdict, which is heavyweight.
//! The happy path from classifier verdict to nudge dispatch is covered by the unit tests on `evaluate_laziness` and `build_laziness_nudge`.
//! The integration tests here pin the actor-level behaviour.
//! They cover enabled/disabled gating, both generation-counter abort arms, idle re-check, the sampler-error abort, and reset on model switch.
use super::support::*;
use super::*;
use crate::agent::config::{LazinessDetectorPerModelConfig, ModelInfo};

fn events_log(tmp: &tempfile::TempDir) -> String {
    std::fs::read_to_string(tmp.path().join("events.jsonl")).unwrap_or_default()
}

/// Attach a `laziness_debug_log` to an existing actor, bypassing `SessionActor::new` (production threads a `PathBuf` through it).
/// Any invariant `SessionActor::new` adds around `laziness_debug_log` MUST be mirrored here or these tests silently diverge from prod.
fn arm_debug_log(actor: &mut SessionActor, path: std::path::PathBuf) {
    actor.laziness_debug_log = Some(std::sync::Arc::from(path.as_path()));
}
