//! Local logging and diagnostics for Codel sessions.
//!
//! Covers the structured unified log, the debug/hooks/memory/sampling file logs,
//! the typed records those logs carry, local tracing spans, and process identity.
//! The fork ships no analytics transport: nothing here opens a network sender or
//! batches a record for export, so no diagnostic leaves the process.

#![deny(clippy::indexing_slicing)]

pub mod config;
pub mod enums;
pub mod events;
pub mod http;
pub mod id;

// Leaf modules re-exported at crate root below, so the public API stays unchanged.
mod logs;
mod process;
mod session;
mod spans;

pub(crate) use codel_trace_context::redact_common;
pub use codel_trace_context::redact_common::redact_error_detail;

pub(crate) use logs::appender;
pub use logs::{debug_log, hooks_log, memory_log, sampling_log, unified_log};
pub use process::{memory_telemetry, process_info, process_metrics};
pub use session::{activity, session_ctx, session_end, session_metrics, subagent_spawn};
pub use spans::{instrumentation, prompt_timing, region, span_profile, startup, turn_phases};

pub use session::session_ctx::{TelemetryCtx, spawn_local_in_session_ctx, with_session_ctx};
