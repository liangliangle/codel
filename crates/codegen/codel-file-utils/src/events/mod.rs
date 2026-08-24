//! Per-session functional turn state.
//!
//! Product-telemetry emission (the `events.jsonl` writer and the `Event`
//! catalog) has been removed. What remains are the functional types shared
//! across crates: [`CancellationCategory`] (drives the next-turn interrupt
//! marker for the model) and [`ToolOutcome`] (drives the observability/UI
//! tool-call outcome), plus the [`EventTracker`] cross-turn state.

pub mod tracker;
pub mod types;

pub use tracker::EventTracker;
pub use types::{CancellationCategory, ToolOutcome};
