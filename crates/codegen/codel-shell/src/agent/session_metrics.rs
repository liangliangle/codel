//! Session lifecycle event structs.
//!
//! The structs moved to `codel-logging` in the telemetry crate split.
//! This module re-exports them so the existing import path in shell keeps working.

pub(crate) use codel_logging::session_metrics::{
    DoomLoopDetected, DoomLoopRecovery, LongReasoningReminderTurn, SessionContextSnapshot,
    SessionStartKind, SessionStarted, TraceUploadAttempted, TraceUploadFailed, TraceUploadSkipped,
    TraceUploadSucceeded, Turn, TurnCompletedLifecycle,
};
