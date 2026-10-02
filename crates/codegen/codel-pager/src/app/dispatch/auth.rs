//! Auth-related dispatchers: MCP server auth setup, and the scrollback helpers that
//! classify auth/transport failures.
//!
//! Upstream also handled login, logout, account switching and auth-code submission here;
//! the fork authenticates with a configured API key, so none of those exist.

use super::ctx::{refuse_withheld, restore_auth_return_view, show_welcome};
use super::queue::{maybe_drain_queue, note_peek_page_flip};
use super::router::dispatch;
use super::session::lifecycle::{clear_startup_actions, drain_startup_actions};
use crate::app::actions::{Action, Effect};
use crate::app::agent::AgentId;
use crate::app::agent_view::AgentView;
use crate::app::app_view::{ActiveView, AppView, AuthState};
use crate::scrollback::block::RenderBlock;
use crate::scrollback::blocks::SessionEvent;
use codel_config::Capability;

/// Scan the trailing run of session-event / system blocks for a [`SessionEvent::ReAuthRequired`] prompt.
/// Used by the `PromptResponse` handler to suppress the redundant "Turn failed" block after a 401.
/// The re-auth prompt is pushed by the `RetryState` handler, which runs first.
pub(super) fn scrollback_has_recent_reauth_prompt(
    scrollback: &crate::scrollback::state::ScrollbackState,
) -> bool {
    trailing_session_events(scrollback).any(|(_, ev)| matches!(ev, SessionEvent::ReAuthRequired))
}

/// True if the trailing run of session/system blocks has a terminal context-overflow block ([`SessionEvent::ContextTooLarge`] or `CompactionFailed`).
/// Lets `PromptResponse` suppress the redundant `TurnFailed`, mirroring reauth.
pub(super) fn scrollback_has_recent_context_too_large(
    scrollback: &crate::scrollback::state::ScrollbackState,
) -> bool {
    trailing_session_events(scrollback).any(|(_, ev)| {
        matches!(
            ev,
            SessionEvent::ContextTooLarge | SessionEvent::CompactionFailed { .. }
        )
    })
}

pub(crate) fn scrollback_has_recent_disk_full(
    scrollback: &crate::scrollback::state::ScrollbackState,
) -> bool {
    trailing_session_events(scrollback).any(|(_, ev)| matches!(ev, SessionEvent::DiskFull))
}

/// True if the trailing run already has a dedicated terminal error banner that replaces `TurnFailed`.
/// `CompactionFailed` is deliberately excluded: it can appear mid-turn.
/// On the reconcile/viewer paths a stale one must not swallow the only banner an unrelated error gets.
pub(in crate::app) fn scrollback_has_recent_error_banner(
    scrollback: &crate::scrollback::state::ScrollbackState,
) -> bool {
    trailing_session_events(scrollback).any(|(_, ev)| {
        matches!(
            ev,
            SessionEvent::ReAuthRequired
                | SessionEvent::ContextTooLarge
                | SessionEvent::DiskFull
                | SessionEvent::RequestFailed { .. }
        )
    })
}

/// True if the trailing run already has a formatted [`SessionEvent::RequestFailed`] banner.
/// Lets `PromptResponse` skip the redundant `TurnFailed`.
/// Deliberately does not match `RetryFailed`.
pub(super) fn scrollback_has_recent_request_failed(
    scrollback: &crate::scrollback::state::ScrollbackState,
) -> bool {
    trailing_session_events(scrollback)
        .any(|(_, ev)| matches!(ev, SessionEvent::RequestFailed { .. }))
}

/// The trailing run of session events, newest first: yields `(index, event)` for each session-event block at the tail of the scrollback.
/// It skips interleaved system messages and stops at the first substantive block.
/// Banners for the finishing turn live in this run; they were pushed just before its `PromptResponse` arrived.
pub(super) fn trailing_session_events(
    scrollback: &crate::scrollback::state::ScrollbackState,
) -> impl Iterator<Item = (usize, &SessionEvent)> {
    use crate::scrollback::block::RenderBlock;
    (0..scrollback.len())
        .rev()
        .map(|idx| (idx, scrollback.entry(idx).map(|e| &e.block)))
        .take_while(|(_, block)| {
            matches!(
                block,
                Some(RenderBlock::SessionEvent(_) | RenderBlock::System(_))
            )
        })
        .filter_map(|(idx, block)| match block {
            Some(RenderBlock::SessionEvent(ev)) => Some((idx, &ev.event)),
            _ => None,
        })
}

/// Strip the trailing run of auth-error blocks (the `ReAuthRequired` prompt plus any stale `RetryFailed` / `TurnFailed`) from an agent's scrollback.
/// Called after a successful mid-session re-auth so the prompt disappears once the user returns to the session.
/// Mirrors how the credit-limit upsell strips its stale blocks.
pub(super) fn strip_trailing_auth_error_blocks(agent: &mut AgentView) {
    let to_remove: Vec<usize> = trailing_session_events(&agent.scrollback)
        .filter(|(_, ev)| {
            matches!(
                ev,
                SessionEvent::ReAuthRequired
                    | SessionEvent::RequestFailed { .. }
                    | SessionEvent::RetryFailed { .. }
                    | SessionEvent::TurnFailed { .. }
            )
        })
        .map(|(idx, _)| idx)
        .collect();
    for idx in to_remove {
        agent.scrollback.remove_from(idx);
    }
}

// TaskResult handlers.

pub(super) fn handle_mcp_auth_trigger_done(
    app: &mut AppView,
    agent_id: AgentId,
    server_name: String,
    result: Result<crate::app::actions::McpAuthTriggerOutcome, String>,
) -> Vec<Effect> {
    let Some(agent) = app.agents.get_mut(&agent_id) else {
        return vec![];
    };
    if let Some(ref mut modal) = agent.extensions_modal {
        modal.pending_action = None;
        modal.pending_entry_index = None;
        match result {
            Ok(crate::app::actions::McpAuthTriggerOutcome::Authenticated) => {}
            Ok(crate::app::actions::McpAuthTriggerOutcome::SetupRequired(setup)) => {
                let setup_values = match &modal.mcps_data {
                    crate::views::extensions_modal::TabDataState::Loaded(servers) => servers
                        .iter()
                        .find(|server| server.name == server_name)
                        .map(|server| server.setup_values.clone())
                        .unwrap_or_default(),
                    _ => std::collections::HashMap::new(),
                };
                if let Some(form) = crate::views::extensions_modal::McpSetupFormState::from_setup(
                    server_name.clone(),
                    setup,
                    setup_values,
                ) {
                    modal.mcp_setup = Some(form);
                } else {
                    modal.modal_message =
                        Some(crate::views::extensions_modal::ModalMessage::Error(
                            format!("{server_name}: setup schema is not supported in this UI"),
                        ));
                }
                return vec![];
            }
            Err(e) => {
                let msg = if e.starts_with("To authenticate") {
                    format!("{server_name}: {e}")
                } else if e.contains(&server_name) {
                    format!("Auth failed: {e}")
                } else {
                    format!("{server_name} auth failed: {e}")
                };
                modal.modal_message =
                    Some(crate::views::extensions_modal::ModalMessage::Error(msg));
                if let Some(session_id) = agent.session.session_id.clone() {
                    return vec![Effect::FetchMcpsList {
                        agent_id,
                        session_id,
                        cache: false,
                    }];
                }
                return vec![];
            }
        }
    }
    // No toast on success: the row transition from the FetchMcpsList refresh below is the confirmation
    let Some(session_id) = agent.session.session_id.clone() else {
        return vec![];
    };
    vec![Effect::FetchMcpsList {
        agent_id,
        session_id,
        cache: false,
    }]
}

pub(super) fn handle_mcp_setup_submit_done(
    app: &mut AppView,
    agent_id: AgentId,
    server_name: String,
    result: Result<(), String>,
) -> Vec<Effect> {
    let Some(agent) = app.agents.get_mut(&agent_id) else {
        return vec![];
    };
    if let Some(ref mut modal) = agent.extensions_modal {
        if let Err(e) = result {
            modal.pending_action = None;
            modal.pending_entry_index = None;
            modal.modal_message = Some(crate::views::extensions_modal::ModalMessage::Error(
                format!("{server_name} setup failed: {e}"),
            ));
            return vec![];
        }
        modal.pending_action = Some(format!("Authenticating {server_name}..."));
        modal.pending_entry_index = None;
    }
    let Some(session_id) = agent.session.session_id.clone() else {
        if let Some(ref mut modal) = agent.extensions_modal {
            modal.pending_action = None;
            modal.modal_message = Some(crate::views::extensions_modal::ModalMessage::Error(
                format!("{server_name}: no active session for authentication"),
            ));
        }
        return vec![];
    };
    vec![Effect::McpAuthTrigger {
        agent_id,
        session_id,
        server_name,
    }]
}
