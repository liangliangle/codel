//! Compaction product telemetry events.

use serde::Serialize;

#[derive(Serialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub enum CompactionTrigger {
    Manual,
    Auto,
}

/// Mixpanel mode label. Detail is omitted so `segments` never includes it.
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CompactionModeLabel {
    Summary,
    Transcript,
    Segments,
}

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TwoPassOutcome {
    /// Policy or product-exception off (cursor, subagents).
    Disabled,
    /// Enabled, fell back to single-pass.
    SinglePass,
    /// Pass-2 summary applied.
    TwoPass,
}

#[derive(Serialize)]
pub struct AutoCompactFired {
    pub tokens_before: u64,
    pub percentage: u8,
}

#[derive(Serialize)]
pub struct CompactionTriggered {
    pub trigger: CompactionTrigger,
    pub tokens_used: u64,
    pub context_window: u64,
    pub percentage: u8,
    pub model_id: String,
    pub compaction_id: String,
    pub compaction_mode: CompactionModeLabel,
    pub two_pass_enabled: bool,
    pub is_subagent: bool,
}

#[derive(Serialize)]
pub struct CompactionCompleted {
    pub duration_ms: u64,
    pub tokens_before: u64,
    pub tokens_after: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
    pub compaction_id: String,
    pub compaction_mode: CompactionModeLabel,
    pub two_pass: TwoPassOutcome,
    pub segments_queued: u32,
    pub degenerate_retries: u32,
    pub input_overflow_retries: u32,
    pub is_subagent: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_wait_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pre_compaction_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub post_compaction_ms: Option<u64>,
}

pub struct CompactionBeginParams {
    pub trigger: CompactionTrigger,
    pub tokens_used: u64,
    pub context_window: u64,
    pub model_id: String,
    pub compaction_mode: CompactionModeLabel,
    pub two_pass_enabled: bool,
    pub is_subagent: bool,
}

pub struct CompactionCompleteStats {
    pub tokens_after: u64,
    pub two_pass_used: bool,
    pub segments_queued: u32,
    pub degenerate_retries: u32,
    pub input_overflow_retries: u32,
}

#[derive(Clone, Copy)]
pub struct CompactionTiming {
    pub model_wait_ms: Option<u64>,
    pub pre_compaction_ms: Option<u64>,
    pub post_compaction_ms: Option<u64>,
}

/// Tracks one compaction for as long as it is in flight: the scope owns the
/// `compaction_active` gauge and the id the full-replace observer correlates on.
/// A scope dropped without `complete` (error or cancel) releases the gauge.
pub struct CompactionScope {
    pub compaction_id: String,
    pub tokens_before: u64,
    pub model_id: String,
    _active: crate::activity::ActivityGaugeGuard,
}

impl CompactionScope {
    pub fn begin(params: CompactionBeginParams) -> Self {
        let CompactionBeginParams {
            tokens_used,
            model_id,
            ..
        } = params;
        let compaction_id = uuid::Uuid::new_v4().to_string();
        let active = crate::activity::COMPACTIONS_ACTIVE.enter();
        debug_assert!(
            crate::activity::COMPACTIONS_ACTIVE.get() >= 1,
            "a compaction scope must stamp a self-inclusive active count"
        );
        Self {
            compaction_id,
            tokens_before: tokens_used,
            model_id,
            _active: active,
        }
    }

    /// Ends the compaction scope. Callers pass the compaction's outcome for the
    /// caller's own use; the scope itself only owns the gauge.
    pub fn complete(self, _stats: CompactionCompleteStats, _timing: CompactionTiming) {}
}

/// Auto-compaction suppressed after a deterministic failure so the turn loop stops re-firing a doomed compaction.
/// Fires once per transition into the suppressed state; `reason` is a fixed classification: `credit_block | size | auth | schema | other`.
#[derive(Serialize)]
pub struct AutoCompactSuppressed {
    pub reason: &'static str,
    pub estimated_tokens: u64,
    pub context_window: u64,
}

#[derive(Serialize)]
pub struct CompactionRetryDegraded {
    pub trigger: CompactionTrigger,
    pub reason: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from_stage: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to_stage: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary_chars: Option<u64>,
    pub attempt: u32,
    pub context_window: u64,
    pub compaction_id: String,
}
