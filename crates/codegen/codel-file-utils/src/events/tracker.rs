use std::cell::{Cell, RefCell};
use std::path::Path;
use std::time::Instant;

use super::types::CancellationCategory;

/// Per-session functional turn state. `!Send` — lives on the session actor.
///
/// Product-telemetry emission (the `events.jsonl` writer and the `Event`
/// catalog) has been removed. What remains is the cross-turn
/// state that drives functional behavior — tagging the next user turn for the
/// model and arming an interrupt `<system-reminder>` — plus the per-turn tool
/// counting that feeds the UI / observability status stream.
pub struct EventTracker {
    active_tool: RefCell<Option<(String, Instant)>>,
    turn_tool_count: Cell<u32>,
    /// Cross-turn one-shot: the *fatal* user-interrupt cause that cancelled the
    /// most recent turn (set by the cancel paths), consumed by the *next* real
    /// user prompt to tag `UserItem::prior_turn_interrupt`. Deliberately NOT
    /// reset by `begin_turn` — it must survive into the next turn; the consumer
    /// clears it via `take_prior_interrupt_category`. Interjections are NOT
    /// recorded here (they don't cancel the turn).
    prior_interrupt_category: Cell<Option<CancellationCategory>>,
    /// Cross-turn one-shot: armed by the cancel path only when a turn was aborted
    /// mid-stream with NO tool in flight, so neither the dangling-tool-call
    /// repair nor a permission tool-result will tell the model it was
    /// interrupted. Consumed by the next *real* user prompt to inject an
    /// interrupt `<system-reminder>`. Like the marker above it deliberately
    /// survives `begin_turn` so it reaches the next real turn.
    pending_interrupt_reminder: Cell<bool>,
}

impl std::fmt::Debug for EventTracker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let active_tool = self.active_tool.borrow();
        f.debug_struct("EventTracker")
            .field("turn_tool_count", &self.turn_tool_count.get())
            .field("active_tool", &active_tool.as_ref().map(|(name, _)| name))
            .field(
                "prior_interrupt_category",
                &self.prior_interrupt_category.get(),
            )
            .field(
                "pending_interrupt_reminder",
                &self.pending_interrupt_reminder.get(),
            )
            .finish()
    }
}

impl EventTracker {
    /// `_session_dir` is retained for call-site stability; the per-session
    /// `events.jsonl` writer it once opened has been removed.
    pub fn new(_session_dir: &Path) -> Self {
        Self {
            active_tool: RefCell::new(None),
            turn_tool_count: Cell::new(0),
            prior_interrupt_category: Cell::new(None),
            pending_interrupt_reminder: Cell::new(false),
        }
    }

    /// Reset per-turn state. Called at the start of each turn.
    pub fn begin_turn(&self) {
        self.turn_tool_count.set(0);
    }

    /// Record the start of a tool call for cancellation tracking and per-turn
    /// counting; returns the start instant so callers can compute a duration.
    pub fn tool_started(&self, tool_name: String) -> Instant {
        let now = Instant::now();
        *self.active_tool.borrow_mut() = Some((tool_name, now));
        self.turn_tool_count.set(self.turn_tool_count.get() + 1);
        now
    }

    pub fn tool_count_this_turn(&self) -> u32 {
        self.turn_tool_count.get()
    }

    pub fn has_active_tool(&self) -> bool {
        self.active_tool.borrow().is_some()
    }

    pub fn tool_finished(&self) {
        *self.active_tool.borrow_mut() = None;
    }

    /// Clear the in-flight tool on cancel. Called from `cancel_running_task()`
    /// so `has_active_tool()` reflects that no tool is running anymore.
    pub fn cancel_active_tool(&self) {
        self.active_tool.borrow_mut().take();
    }

    /// Record the *fatal* user-interrupt cause that cancelled this turn so the
    /// *next* real user prompt can be tagged. Overwrites any prior value (latest
    /// cause wins).
    pub fn set_prior_interrupt_category(&self, category: CancellationCategory) {
        self.prior_interrupt_category.set(Some(category));
    }

    /// Take (and clear) the recorded prior-turn interrupt cause.
    pub fn take_prior_interrupt_category(&self) -> Option<CancellationCategory> {
        self.prior_interrupt_category.take()
    }

    /// Arm the one-shot interrupt reminder for the next real user prompt. Set
    /// only on the cancel path when no tool was in flight (the case where the
    /// model would otherwise get no signal that it was interrupted).
    pub fn set_pending_interrupt_reminder(&self) {
        self.pending_interrupt_reminder.set(true);
    }

    /// Take (and clear) the pending interrupt-reminder flag.
    pub fn take_pending_interrupt_reminder(&self) -> bool {
        self.pending_interrupt_reminder.replace(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prior_interrupt_markers_are_one_shot_and_survive_begin_turn() {
        let dir = tempfile::tempdir().expect("tempdir");
        let t = EventTracker::new(dir.path());

        // Defaults: nothing recorded.
        assert_eq!(t.take_prior_interrupt_category(), None);
        assert!(!t.take_pending_interrupt_reminder());

        // Cancel cause is consumed exactly once.
        t.set_prior_interrupt_category(CancellationCategory::MidTurnAbort);
        assert_eq!(
            t.take_prior_interrupt_category(),
            Some(CancellationCategory::MidTurnAbort)
        );
        assert_eq!(t.take_prior_interrupt_category(), None);

        // Interrupt-reminder flag is consumed exactly once.
        t.set_pending_interrupt_reminder();
        assert!(t.take_pending_interrupt_reminder());
        assert!(!t.take_pending_interrupt_reminder());

        // `begin_turn` runs at the START of a turn — BEFORE the next real user
        // prompt consumes the markers — so it must NOT clear these cross-turn
        // markers (it only resets per-turn counters). A regression here would
        // silently drop the `prior_turn_interrupt` tag.
        t.set_prior_interrupt_category(CancellationCategory::PermissionRejected);
        t.set_pending_interrupt_reminder();
        t.begin_turn();
        assert_eq!(
            t.take_prior_interrupt_category(),
            Some(CancellationCategory::PermissionRejected),
            "begin_turn must preserve the cross-turn interrupt cause"
        );
        assert!(
            t.take_pending_interrupt_reminder(),
            "begin_turn must preserve the pending interrupt reminder"
        );
    }
}
