//! Per-turn prompt latency measurement.
//!
//! Implementation lives in `codel-logging::prompt_timing`.
//! This shim keeps `crate::session::prompt_timing::PromptTiming` resolving at the original path so callers don't need to change imports.

pub(crate) use codel_logging::prompt_timing::PromptTiming;
