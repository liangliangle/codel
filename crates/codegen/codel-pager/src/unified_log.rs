//! Unified log forwarding for the pager.
//!
//! Previously buffered log entries in memory and flushed them to the shell
//! via `codel/log` ACP notifications. The telemetry types that backed this
//! pipeline have been removed; the public API is kept as no-ops so callers
//! compile unchanged.

use codel_acp_lib::AcpAgentTx;

/// Initialize the unified log forwarder with the ACP sender.
///
/// No-op: the telemetry pipeline that consumed these entries was removed.
pub fn init(_tx: AcpAgentTx) {}

/// Flush any buffered entries to the shell (fire-and-forget).
///
/// No-op: nothing is buffered anymore.
pub fn flush() {}

/// Flush buffered entries and await delivery.
///
/// No-op: nothing is buffered anymore.
pub async fn flush_blocking() {}

/// Log an info-level entry (no-op).
pub fn info(_msg: &str, _sid: Option<&str>, _ctx: Option<serde_json::Value>) {}

/// Log a warn-level entry (no-op).
pub fn warn(_msg: &str, _sid: Option<&str>, _ctx: Option<serde_json::Value>) {}

/// Log an error-level entry (no-op).
pub fn error(_msg: &str, _sid: Option<&str>, _ctx: Option<serde_json::Value>) {}

/// Log a debug-level entry (no-op).
pub fn debug(_msg: &str, _sid: Option<&str>, _ctx: Option<serde_json::Value>) {}
