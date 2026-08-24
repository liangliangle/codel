use serde::{Deserialize, Serialize};

/// Outcome of a single tool call. More granular than a boolean -- distinguishes
/// between tools that executed vs tools that were never run.
///
/// Functional (not product telemetry): mapped to the observability/UI
/// `SessionEvent::ToolCallCompleted` outcome and used by the workspace
/// `ActivityTracker` for tool-server status reporting.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, strum::IntoStaticStr)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum ToolOutcome {
    /// Tool executed and returned a result.
    Success,
    /// Tool executed but returned an error.
    Error,
    /// User rejected the permission prompt.
    PermissionRejected,
    /// User cancelled the permission prompt (Cmd+C).
    PermissionCancelled,
    /// User provided a followup message instead of approving.
    Followup,
    /// A user-configured hook blocked execution.
    HookDenied,
    /// Tool not found or arguments couldn't be parsed.
    InvalidTool,
    /// Tool was running when the turn was cancelled (Cmd+C).
    Cancelled,
}

// `Deserialize`/`PartialEq`/`Eq`/`Hash` let the workspace decode
// `cancellation_category` strings back into this enum. `snake_case` keeps the
// wire form identical, so adding `Deserialize` doesn't change serialization.
//
// Functional (not product telemetry): drives the `PriorTurnInterrupt` marker
// stamped onto the next real user turn so the model is told how the previous
// turn was interrupted.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum CancellationCategory {
    HookDenied,
    PermissionRejected,
    PermissionCancelled,
    MidTurnAbort,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every variant must survive a `to_value` -> `from_value` round-trip.
    #[test]
    fn cancellation_category_round_trips_every_variant() {
        for variant in [
            CancellationCategory::HookDenied,
            CancellationCategory::PermissionRejected,
            CancellationCategory::PermissionCancelled,
            CancellationCategory::MidTurnAbort,
        ] {
            let value = serde_json::to_value(variant).unwrap();
            let decoded: CancellationCategory = serde_json::from_value(value).unwrap();
            assert_eq!(decoded, variant, "{variant:?} must round-trip");
        }
    }

    /// Serialization is unchanged by the added derives (bare snake_case strings).
    #[test]
    fn cancellation_category_serializes_snake_case() {
        for (variant, expected) in [
            (CancellationCategory::HookDenied, "\"hook_denied\""),
            (
                CancellationCategory::PermissionRejected,
                "\"permission_rejected\"",
            ),
            (
                CancellationCategory::PermissionCancelled,
                "\"permission_cancelled\"",
            ),
            (CancellationCategory::MidTurnAbort, "\"mid_turn_abort\""),
        ] {
            let json = serde_json::to_string(&variant).unwrap();
            assert_eq!(json, expected, "{variant:?} must serialize to {expected}");
        }
    }
}
