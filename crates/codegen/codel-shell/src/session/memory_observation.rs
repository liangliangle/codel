use codel_logging::memory_telemetry::{
    MemoryInjection, MemoryInjectionOutcome, MemorySearch,
    MemorySearchErrorClass as TelemetryErrorClass, MemorySearchMode as TelemetryMode,
    MemorySearchOutcome as TelemetryOutcome, MemorySearchSource as TelemetrySource,
    MemoryWatcherSync,
};
use codel_memory::{
    MemoryObservationSink, MemoryRetrievalMode, MemorySearchErrorClass, MemorySearchObservation,
    MemorySearchSource, MemoryWatcherSyncObservation,
};

pub(crate) struct TelemetryMemoryObservationSink {
    pub(crate) session_id: String,
}

#[derive(Default)]
pub(crate) struct MemoryInjectionMetrics {
    pub(crate) is_greeting_fallback: bool,
    pub(crate) result_count: usize,
    pub(crate) total_snippet_chars: usize,
    pub(crate) top_score: f64,
    pub(crate) configured_min_score: f64,
    pub(crate) duration_ms: u64,
    pub(crate) injected_bytes: u64,
    pub(crate) estimated_tokens: u64,
    pub(crate) global_entry_count: usize,
    pub(crate) workspace_entry_count: usize,
    pub(crate) was_reused: bool,
    pub(crate) compact_index: bool,
}

pub(crate) fn log_memory_injection(
    session_id: String,
    outcome: MemoryInjectionOutcome,
    metrics: MemoryInjectionMetrics,
) {
}

pub(crate) fn memory_v2_model_usage(
    model: &str,
    response: &codel_sampling_types::ConversationResponse,
) -> codel_logging::memory_telemetry::MemoryV2ModelUsage {
    codel_logging::memory_telemetry::MemoryV2ModelUsage {
        model_id: Some(model.to_owned()),
        prompt_tokens: response.usage.as_ref().map(|usage| usage.prompt_tokens),
        completion_tokens: response.usage.as_ref().map(|usage| usage.completion_tokens),
        reasoning_tokens: response.usage.as_ref().map(|usage| usage.reasoning_tokens),
        cached_prompt_tokens: response
            .usage
            .as_ref()
            .map(|usage| usage.cached_prompt_tokens),
        cache_creation_tokens: response
            .usage
            .as_ref()
            .map(|usage| usage.cache_creation_prompt_tokens),
        cost_usd_ticks: response.cost_usd_ticks,
    }
}

impl MemoryObservationSink for TelemetryMemoryObservationSink {
    fn observe_search(&self, observation: MemorySearchObservation) {}

    fn observe_watcher_sync(&self, observation: MemoryWatcherSyncObservation) {}
}
