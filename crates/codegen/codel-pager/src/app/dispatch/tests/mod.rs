//! Tests for the dispatch module tree: shared fixtures and per-domain test modules.
mod auth;
mod cta_e2e;
mod dashboard;
mod jump;
mod mid_text_btw;
mod mid_text_goal;
mod modes;
mod notes;
mod permissions;
mod prompt;
mod prompt_ack;
mod queue_release;
mod rewind;
mod router;
mod session;
mod settings;
mod status;
mod status_line;
mod task_result;
mod transcript;
mod turn;
mod voice;
use super::cta::{
    CTA_MCP_ABSENT_MAX_ATTEMPTS, CTA_MCP_POLL_MAX_ATTEMPTS, cta_impression_plugin_name,
    cta_install_error_category, cta_install_relative_path, plugin_cta_phase_for,
};
use super::ctx::{find_agent_by_session_id, get_active_agent, get_active_agent_mut};
use super::dashboard::{
    apply_pending_dispatch_config, dispatch_dashboard_attach, dispatch_dashboard_begin_rename,
    dispatch_dashboard_commit_rename, dispatch_dashboard_confirm_worktree,
    dispatch_dashboard_create_new_agent_with_detail, dispatch_dashboard_delete,
    dispatch_dashboard_dispatch, dispatch_dashboard_dispatch_slash,
    dispatch_dashboard_overlay_cycle, dispatch_dashboard_overlay_exit,
    dispatch_dashboard_overlay_stop, dispatch_dashboard_peek_reply,
    dispatch_dashboard_permission_followup, dispatch_dashboard_permission_select,
    dispatch_dashboard_question_answer, dispatch_dashboard_stop,
    dispatch_dashboard_toggle_auto_approve, dispatch_dashboard_toggle_pin, dispatch_exit_dashboard,
    dispatch_open_dashboard, ensure_dashboard_state, resolve_location_input,
};
use super::modes::{
    YOLO_ON_UNDER_PLAN_TOAST, active_agent_plan_nudge_state, dispatch_cycle_mode_and_sync,
    downgrade_displayed_auto_if_gated, permission_mode_toast,
};
use super::permissions::drain_permission_queue;
use super::prompt::{dispatch_doctor, dispatch_send_prompt, dispatch_send_prompt_inner};
use super::session::fork::build_child_fork_marker;
use super::session::lifecycle::{dispatch_new_session_inner, drain_startup_actions, finish_trust};
use super::session::load::{dispatch_load_session_with_restore, reanchor_grouped_selection};
use super::session::modal::{
    dispatch_rename_session, dispatch_reset_session_title, dispatch_sessions_confirm_close,
    drop_other_agents_in_minimal,
};
use super::settings::handle_feature_override_persisted;
use super::settings::setters::set_default_model_inner;
use super::settings::ui::{action_for_reset, apply_setting_rollback};
use super::status::scrub_error_for_toast;
use super::task_result::dispatch_task_result;
use super::*;
use crate::acp::model_state::ModelState;
use crate::acp::tracker::AcpUpdateTracker;
use crate::app::actions::{
    Action, Effect, ModelChoice, SubagentKillOutcome, SwitchModelError, TaskResult,
    WorkspaceMutation, WorkspaceWriteCompletion,
};
use crate::app::app_view::tests::test_app;
use crate::app::agent::{AgentId, AgentSession, AgentState};
use crate::app::agent_view::{ActivePane, AgentView, PromptMode};
use crate::app::app_view::{
    ActiveView, AppView, AuthState, PendingCodingDataWrite, TrustState, VoiceState, VoiceTarget,
    WelcomeAnnouncementState,
};
use crate::scrollback::block::RenderBlock;
use crate::scrollback::blocks::{SessionEvent, ToolCallBlock};
use crate::scrollback::state::ScrollbackState;
use crate::views::session_picker_surface::SessionPickerHost;
use agent_client_protocol as acp;
use indexmap::IndexMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
/// Build a default `AgentSession` for tests.
/// Centralises the fixture so new fields on `AgentSession` don't break every test that constructs one by hand.
/// The `acp_tx` is cloned from the test `AppView`.
fn make_test_agent_session(app: &AppView, id: AgentId, sid: &str) -> AgentSession {
    AgentSession {
        id,
        acp_tx: app.acp_tx.clone(),
        session_id: Some(sid.to_string().into()),
        models: ModelState::default(),
        state: AgentState::Idle,
        tracker: AcpUpdateTracker::new(),
        cwd: PathBuf::from("/tmp"),
        is_worktree: false,
        forked_from: None,
        pending_prompts: std::collections::VecDeque::new(),
        next_queue_id: 0,
        yolo_mode: false,
        auto_mode: false,
        prompt_history: Vec::new(),
        prompt_history_loading: false,
        loading_replay: false,
        restore_degree: None,
        rate_limited: false,
        model_incompatible: false,
        free_usage_blocked: false,
        available_commands: Vec::new(),
        available_commands_generation: 0,
        available_tools: None,
        model_switch_pending: false,
        hook_block_hold: false,
        blocked_prompt: None,
        user_model_preference: None,
        deferred_model_switch: app.deferred_model_switch_from_cli(),
        bg_tasks: std::collections::BTreeMap::new(),
        bg_tool_call_to_task: std::collections::HashMap::new(),
        scheduled_tasks: std::collections::HashMap::new(),
        in_flight_prompt: None,
        compact_held_prompt: None,
        current_prompt_id: None,
        created_via_new: false,
    }
}
pub(super) fn test_agent_mut(app: &mut AppView, id: AgentId) -> &mut AgentView {
    let Some(agent) = app.agents.get_mut(&id) else {
        panic!("agent {id:?} is not registered");
    };
    agent
}
pub(super) fn test_app_with_agent() -> AppView {
    let mut app = test_app();
    let id = AgentId(0);
    let session = make_test_agent_session(&app, id, "test-session");
    let mut agent = AgentView::new(session, ScrollbackState::new());
    agent.active_pane = ActivePane::Scrollback;
    app.agents.insert(id, agent);
    app.next_agent_id = 1;
    switch_to_agent(&mut app, id, SwitchCause::New);
    app
}
pub(super) fn test_agent(app: &AppView, id: AgentId) -> &AgentView {
    match app.agents.get(&id) {
        Some(agent) => agent,
        None => panic!("missing agent {id:?}"),
    }
}
/// Give a test agent a generated title so the dashboard renders it.
/// The dashboard hides sessions with no real turn (`views::dashboard::row::is_empty_top_level`).
/// Nav and render tests that rely on their placeholder agents being visible call this to opt in.
fn mark_agent_nonempty(app: &mut AppView, id: AgentId) {
    if let Some(a) = app.agents.get_mut(&id) {
        a.generated_session_title = Some(format!("Session {}", id.0));
    }
}
/// Push a plain prompt directly onto the local drip-feed queue (`pending_prompts`), bypassing the server-authoritative immediate send.
/// Used by tests exercising the local `maybe_drain_queue`, editing, and `DrainQueue` machinery.
/// That local path still handles image, skill, bash, and editing prompts and idle drains.
pub(super) fn enqueue_local(app: &mut AppView, id: AgentId, text: &str) {
    app.agents
        .get_mut(&id)
        .unwrap()
        .session
        .enqueue_prompt(text.to_string());
}
fn make_test_subagent(child_sid: &str, sa_id: &str) -> crate::app::subagent::SubagentInfo {
    let now = std::time::Instant::now();
    crate::app::subagent::SubagentInfo {
        subagent_id: Arc::from(sa_id),
        child_session_id: Arc::from(child_sid),
        description: Arc::from("test subagent"),
        subagent_type: Arc::from("general-purpose"),
        attempt: crate::app::subagent::SubagentAttemptInfo {
            lifecycle: crate::app::subagent::SubagentLifecycleState::running_legacy_for_test(),
            persona: None,
            role: None,
            model: None,
            context_source: None,
            resumed_from: None,
            capability_mode: None,
            workflow_run_id: None,
            context_normalized: false,
            parent_prompt_id: None,
            started_at: now,
            last_progress_at: now,
            status: None,
            error: None,
            duration_ms: None,
            tool_calls: None,
            turns: None,
            turn_count: None,
            tool_call_count: None,
            tokens_used: None,
            context_window_tokens: None,
            context_usage_pct: None,
            tools_used: Vec::new(),
            error_count: None,
            activity_label: None,
            is_background: false,
            pending_kill: false,
            kill_requested_at: None,
            scrollback_entry_id: None,
            terminal_entry_id: None,
        },
        completed_attempt_tokens: 0,
        sealed_attempt_tokens: Default::default(),
        prompt: None,
        child_cwd: None,
        worktree_path: None,
        transcript: Default::default(),
    }
}
fn cta_entry(name: &str, status: &str) -> codel_hooks_plugins_types::MarketplacePluginEntry {
    codel_hooks_plugins_types::MarketplacePluginEntry {
        name: name.into(),
        version: None,
        description: None,
        category: None,
        author: None,
        tags: Vec::new(),
        keywords: Vec::new(),
        domains: Vec::new(),
        homepage: None,
        relative_path: format!("plugins/{name}"),
        skill_count: 0,
        has_hooks: false,
        has_agents: false,
        has_mcp: false,
        install_status: status.into(),
        installed_version: None,
        components: None,
        remote_url: None,
        remote_ref: None,
        remote_sha: None,
        remote_subdir: None,
    }
}
fn cta_outcome(
    status: codel_hooks_plugins_types::OutcomeStatus,
    message: &str,
) -> codel_hooks_plugins_types::ActionOutcome {
    codel_hooks_plugins_types::ActionOutcome {
        status,
        message: message.into(),
        requires_reload: false,
        requires_restart: false,
    }
}
fn cta_mcp_server(
    name: &str,
    plugin: Option<&str>,
    status: crate::views::mcps_modal::McpServerDisplayStatus,
) -> crate::views::mcps_modal::McpServerInfo {
    use crate::views::mcps_modal::{McpServerDisplayStatus, McpWireSource};
    crate::views::mcps_modal::McpServerInfo {
        name: name.into(),
        display_name: None,
        status,
        tool_count: 0,
        auth_required: matches!(status, McpServerDisplayStatus::NeedsAuth),
        setup_required: false,
        setup: None,
        setup_values: std::collections::HashMap::new(),
        tools: vec![],
        enabled: true,
        source: plugin
            .map(|p| format!("plugin: {p}"))
            .unwrap_or_else(|| "local".into()),
        blocked_reason: None,
        wire_source: McpWireSource::Local,
        plugin_name: plugin.map(str::to_string),
        is_managed_gateway: false,
    }
}
fn arm_reconcile(
    app: &mut AppView,
    id: AgentId,
    prompt_id: &str,
    stop_reason: &str,
    age: std::time::Duration,
) {
    arm_reconcile_with_trigger(app, id, prompt_id, stop_reason, None, age);
}
/// [`arm_reconcile`] with an explicit `_meta.cancelTrigger`.
fn arm_reconcile_with_trigger(
    app: &mut AppView,
    id: AgentId,
    prompt_id: &str,
    stop_reason: &str,
    cancel_trigger: Option<&str>,
    age: std::time::Duration,
) {
    arm_reconcile_with_meta(app, id, prompt_id, stop_reason, cancel_trigger, None, age);
}
/// [`arm_reconcile`] with explicit `_meta.cancelTrigger` and `_meta.cancellationCategory`.
#[allow(clippy::too_many_arguments)]
fn arm_reconcile_with_meta(
    app: &mut AppView,
    id: AgentId,
    prompt_id: &str,
    stop_reason: &str,
    cancel_trigger: Option<&str>,
    cancellation_category: Option<&str>,
    age: std::time::Duration,
) {
    app.agents.get_mut(&id).unwrap().pending_turn_end_reconcile =
        Some(crate::app::agent_view::PendingTurnEnd {
            prompt_id: prompt_id.into(),
            stop_reason: Some(stop_reason.into()),
            agent_result: None,
            cancel_trigger: cancel_trigger.map(str::to_string),
            cancellation_category: cancellation_category.map(str::to_string),
            cancellation_context: None,
            error_kind: None,
            received_at: std::time::Instant::now() - age,
        });
}
pub(super) fn end_turn() -> Action {
    Action::TaskComplete(TaskResult::PromptResponse {
        agent_id: AgentId(0),
        result: Ok(acp::PromptResponse::new(acp::StopReason::EndTurn)),
        http_status: None,
        prompt_id: None,
    })
}
/// Plant a Build session under the process `codel_home()` (OnceLock-cached; do not rely on setting `CODEL_HOME` mid-process).
/// Caller must remove `sess_dir`.
fn plant_local_build_session(cwd: &std::path::Path, session_id: &str) -> std::path::PathBuf {
    let home = codel_shell::util::codel_home::codel_home();
    let encoded = codel_shell::util::codel_home::encode_cwd_dirname(&cwd.to_string_lossy());
    let sess_dir = home.join("sessions").join(encoded).join(session_id);
    std::fs::create_dir_all(&sess_dir).expect("plant session dir");
    std::fs::write(sess_dir.join("summary.json"), b"{}").expect("plant summary");
    sess_dir
}
/// Extract text from the last system message in an agent's scrollback.
pub(super) fn last_system_text(app: &AppView, id: AgentId) -> String {
    system_text_from_end(app, id, 0)
}
/// Like [`last_system_text`] but takes an offset from the end.
/// `offset = 0` is the last entry, `offset = 1` is second-to-last, etc.
fn system_text_from_end(app: &AppView, id: AgentId, offset: usize) -> String {
    let sb = &test_agent(app, id).scrollback;
    let idx = sb.len() - 1 - offset;
    let entry = sb.get(idx).expect("scrollback index out of bounds");
    match &entry.block {
        RenderBlock::System(sys) => sys.text.clone(),
        other => panic!("expected System block at index {idx}, got {other:?}"),
    }
}
/// Insert a placeholder agent at `id` so `switch_to_agent` recognises it (the helper's defensive check uses `app.agents.contains_key`).
/// `session_id` and `active_pane` are populated to mirror the existing `test_app_with_agent` setup; these tests do not read either field.
fn insert_placeholder_agent(app: &mut AppView, id: AgentId) {
    let mut agent = AgentView::new(
        AgentSession {
            id,
            acp_tx: app.acp_tx.clone(),
            session_id: Some("placeholder".into()),
            models: ModelState::default(),
            state: AgentState::Idle,
            tracker: AcpUpdateTracker::new(),
            cwd: PathBuf::from("/tmp"),
            is_worktree: false,
            forked_from: None,
            pending_prompts: std::collections::VecDeque::new(),
            next_queue_id: 0,
            yolo_mode: false,
            auto_mode: false,
            prompt_history: Vec::new(),
            prompt_history_loading: false,
            loading_replay: false,
            restore_degree: None,
            rate_limited: false,
            model_incompatible: false,
            free_usage_blocked: false,
            available_commands: Vec::new(),
            available_commands_generation: 0,
            available_tools: None,
            model_switch_pending: false,
            hook_block_hold: false,
            blocked_prompt: None,
            user_model_preference: None,
            deferred_model_switch: None,
            bg_tasks: std::collections::BTreeMap::new(),
            bg_tool_call_to_task: std::collections::HashMap::new(),
            scheduled_tasks: std::collections::HashMap::new(),
            in_flight_prompt: None,
            compact_held_prompt: None,
            current_prompt_id: None,
            created_via_new: false,
        },
        ScrollbackState::new(),
    );
    agent.active_pane = ActivePane::Scrollback;
    app.agents.insert(id, agent);
}
/// Build an app with three agents (ids 0, 1, 2) and `active_view` set to agent 0.
pub(super) fn three_agent_app() -> AppView {
    let mut app = test_app_with_agent();
    insert_placeholder_agent(&mut app, AgentId(1));
    insert_placeholder_agent(&mut app, AgentId(2));
    app
}
#[test]
fn local_slash_command_keeps_hook_block_hold() {
    let mut app = test_app_with_agent();
    app.agents
        .get_mut(&AgentId(0))
        .unwrap()
        .session
        .hook_block_hold = true;
    let _ = dispatch_send_prompt_inner(&mut app, "/help".into(), true, false, false);
    assert!(
        app.agents.get(&AgentId(0)).unwrap().session.hook_block_hold,
        "a local-UI slash command is not re-engagement and must keep the hold"
    );
    let _ = dispatch_send_prompt_inner(&mut app, "a real prompt".into(), true, false, false);
    assert!(
        !app.agents.get(&AgentId(0)).unwrap().session.hook_block_hold,
        "a plain prompt submission releases the hold"
    );
}
use crate::slash::commands::fork::ForkArgs;
fn fork_args(worktree_override: Option<bool>, directive: Option<&str>) -> ForkArgs {
    ForkArgs {
        worktree_override,
        directive: directive.map(String::from),
    }
}
/// Build a single-agent app for the `/fork` dispatcher tests.
/// Sets `current_branch` to `Some("main")` so the agent appears to be inside a git repo.
/// `dispatch_fork` skips the worktree question when `current_branch` is `None` (non-git cwd).
fn fork_test_app() -> AppView {
    let mut app = test_app_with_agent();
    app.agents.get_mut(&AgentId(0)).unwrap().current_branch = Some("main".into());
    app
}
/// Build a minimal `AcpArgs<acp::ExtRequest>` for an `codel/ask_user_question` ext-method request.
/// Returns the args and the receiver half of the response oneshot so the test can assert the handler completes the ACP roundtrip.
fn make_ask_user_question_args(
    tool_call_id: &str,
) -> (
    codel_acp_lib::AcpArgs<acp::ExtRequest>,
    tokio::sync::oneshot::Receiver<codel_acp_lib::AcpResult<acp::ExtResponse>>,
) {
    use codel_tools::implementations::codel_build::ask_user_question::{
        AskUserQuestionExtRequest, Question, QuestionOption,
    };
    let req = AskUserQuestionExtRequest {
        session_id: "test-session".into(),
        tool_call_id: tool_call_id.into(),
        mode: codel_tools::implementations::codel_build::ask_user_question::AskUserQuestionMode::Default,
        questions: vec![Question {
                question: "ACP-driven question".into(),
                options: vec![QuestionOption {
                    label: "ok".into(),
                    description: "ok".into(),
                    preview: None,
                    id: None,
                }],
                multi_select: Some(false),
                            id: None,
            }],
    };
    let (tx, rx) = tokio::sync::oneshot::channel();
    let ext = acp::ExtRequest::new(
        "codel/ask_user_question",
        serde_json::value::to_raw_value(&req)
            .expect("serialize AskUserQuestionExtRequest")
            .into(),
    );
    (
        codel_acp_lib::AcpArgs {
            request: ext,
            response_tx: tx,
        },
        rx,
    )
}
fn set_forked_from(app: &mut AppView, child: AgentId, parent: AgentId) {
    if let Some(agent) = app.agents.get_mut(&child) {
        agent.session.forked_from = Some(parent);
    }
}
pub(super) fn make_bg_task(task_id: &str) -> crate::app::agent::BgTaskState {
    crate::app::agent::BgTaskState {
        task_id: task_id.into(),
        tool_call_id: String::new(),
        command: "sleep 99".into(),
        description: None,
        cwd: String::new(),
        output_file: String::new(),
        status: crate::app::agent::BgTaskStatus::Running,
        start_time: std::time::SystemTime::now(),
        end_time: None,
        exit_code: None,
        signal: None,
        stdout: String::new(),
        stdout_line_count: 0,
        truncated: false,
        pending_kill: false,
        kill_requested_at: None,
        scrollback_entry_id: None,
        is_monitor: false,
        restored_from_replay: false,
    }
}
/// Set up a two-agent app: agent 0 is active with "sess-A", agent 1 is inactive with "sess-B" and a bg task.
fn two_agent_app_with_bg_task() -> AppView {
    let mut app = test_app_with_agent();
    let Some(agent) = app.agents.get_mut(&AgentId(0)) else {
        panic!("missing agent AgentId(0)");
    };
    agent.session.session_id = Some(acp::SessionId::new("sess-A"));
    let id1 = AgentId(1);
    let mut agent1 = AgentView::new(
        AgentSession {
            id: id1,
            acp_tx: app.acp_tx.clone(),
            session_id: Some(acp::SessionId::new("sess-B")),
            models: ModelState::default(),
            state: AgentState::Idle,
            tracker: AcpUpdateTracker::new(),
            cwd: PathBuf::from("/tmp"),
            is_worktree: false,
            forked_from: None,
            pending_prompts: std::collections::VecDeque::new(),
            next_queue_id: 0,
            yolo_mode: false,
            auto_mode: false,
            prompt_history: Vec::new(),
            prompt_history_loading: false,
            loading_replay: false,
            restore_degree: None,
            rate_limited: false,
            model_incompatible: false,
            free_usage_blocked: false,
            available_commands: Vec::new(),
            available_commands_generation: 0,
            available_tools: None,
            model_switch_pending: false,
            hook_block_hold: false,
            blocked_prompt: None,
            user_model_preference: None,
            deferred_model_switch: None,
            bg_tasks: std::collections::BTreeMap::new(),
            bg_tool_call_to_task: std::collections::HashMap::new(),
            scheduled_tasks: std::collections::HashMap::new(),
            in_flight_prompt: None,
            compact_held_prompt: None,
            current_prompt_id: None,
            created_via_new: false,
        },
        ScrollbackState::new(),
    );
    let mut task = make_bg_task("task-B-1");
    task.pending_kill = true;
    task.kill_requested_at = Some(std::time::Instant::now());
    agent1.session.bg_tasks.insert("task-B-1".into(), task);
    app.agents.insert(id1, agent1);
    app.next_agent_id = 2;
    assert!(matches!(app.active_view, ActiveView::Agent(AgentId(0))));
    app
}
/// Test helper: open Settings then OpenResetConfirm for `key`.
/// Extracted so individual tests don't have to repeat the two opens.
fn setup_reset_confirm_open(app: &mut AppView, key: crate::settings::SettingKey) {
    use crate::views::modal::ActiveModal;
    let _ = dispatch(Action::OpenSettings, app);
    let _ = dispatch(Action::OpenResetConfirm { key }, app);
    let agent = app.agents.get(&AgentId(0)).expect("agent must exist");
    assert!(
        matches!(
            agent.active_modal,
            Some(ActiveModal::ResetSettingsConfirm { .. })
        ),
        "setup_reset_confirm_open: ResetSettingsConfirm must be active",
    );
}
fn make_picker_entry(id: &str, cwd: &str) -> crate::app::app_view::SessionPickerEntry {
    crate::app::app_view::SessionPickerEntry {
        id: id.into(),
        summary: id.into(),
        updated_at: chrono::Utc::now(),
        created_at: chrono::Utc::now(),
        cwd: cwd.into(),
        hostname: None,
        source: "local".into(),
        model_id: None,
        num_messages: 0,
        last_active_at: None,
        branch: None,
        repo_name: "repo".into(),
        worktree_label: None,
        last_turn_summary: None,
        last_recap: None,
        session_kind: None,
        card_detail: None,
    }
}
fn make_conversation_entry(id: &str) -> crate::app::app_view::SessionPickerEntry {
    let mut e = make_picker_entry(id, "");
    e.source = "conversation".into();
    e
}
/// Open a SessionPicker modal on the active agent seeded with `entries`.
/// Stamps a real allocated generation, as production modals get theirs from `dispatch_fetch_session_list`.
/// Helper-seeded modals can then receive generation-gated results.
fn open_session_picker_with(
    app: &mut AppView,
    entries: Vec<crate::app::app_view::SessionPickerEntry>,
) {
    use crate::views::modal::ActiveModal;
    let generation = app.alloc_picker_generation();
    let agent = get_active_agent_mut(app).expect("active agent");
    agent.active_modal = Some(ActiveModal::SessionPicker {
        state: crate::views::picker::PickerState::default(),
        entries: Some(entries),
        loading: false,
        lanes: Default::default(),
        previous_palette: None,
        window: crate::views::modal_window::ModalWindowState::new(),
        content_results: None,
        content_loading: false,
        deep_search_seq: 0,
        generation,
        detail_seq: 0,
        entries_query: None,
        source_filter: crate::views::session_picker::SourceFilter::default(),
        pending_delete: None,
    });
}
/// Live generation of the active agent's SessionPicker modal, for stamping modal-host results the way the executors echo them.
fn modal_picker_generation(app: &AppView) -> u64 {
    use crate::views::modal::ActiveModal;
    match get_active_agent(app)
        .expect("active agent")
        .active_modal
        .as_ref()
    {
        Some(ActiveModal::SessionPicker { generation, .. }) => *generation,
        _ => panic!("expected SessionPicker modal"),
    }
}
/// Live card-detail seq of the active agent's SessionPicker modal.
fn modal_picker_detail_seq(app: &AppView) -> u64 {
    use crate::views::modal::ActiveModal;
    match get_active_agent(app)
        .expect("active agent")
        .active_modal
        .as_ref()
    {
        Some(ActiveModal::SessionPicker { detail_seq, .. }) => *detail_seq,
        _ => panic!("expected SessionPicker modal"),
    }
}
/// Read the active agent's toast text, panicking if none is set.
fn read_toast(app: &AppView) -> String {
    let agent = app.agents.get(&AgentId(0)).expect("agent must exist");
    agent
        .toast
        .as_ref()
        .map(|(s, _)| s.clone())
        .expect("toast should be set")
}
/// Enqueue one permission whose options mirror the list the shell builds for TUI, Pager, and Desktop.
/// The options: "enable-always-approve" (AllowOnce, position 0, default-selected), "opt-allow-once" (AllowOnce), and "opt-reject-once" (RejectOnce).
/// Returns the response receiver for the injected permission.
fn enqueue_permission_with_enable_always_approve(
    app: &mut AppView,
) -> tokio::sync::oneshot::Receiver<acp::Result<acp::RequestPermissionResponse>> {
    use crate::views::permission_view::{PermissionFocus, PermissionViewState};
    use std::sync::Arc;
    let agent = app.agents.get_mut(&AgentId(0)).unwrap();
    let (response_tx, response_rx) = tokio::sync::oneshot::channel();
    let request = acp::RequestPermissionRequest::new(
        acp::SessionId::new(Arc::from("test-sess")),
        acp::ToolCallUpdate::new(
            acp::ToolCallId::new(Arc::from("tc-enable-aa-1")),
            acp::ToolCallUpdateFields::default(),
        ),
        vec![
            acp::PermissionOption::new(
                acp::PermissionOptionId::new(Arc::from(
                    codel_workspace::permission::ENABLE_ALWAYS_APPROVE_OPTION_ID,
                )),
                "Yes, and don't ask again for anything",
                acp::PermissionOptionKind::AllowOnce,
            ),
            acp::PermissionOption::new(
                acp::PermissionOptionId::new(Arc::from("opt-allow-once")),
                "Yes, proceed",
                acp::PermissionOptionKind::AllowOnce,
            ),
            acp::PermissionOption::new(
                acp::PermissionOptionId::new(Arc::from("opt-reject-once")),
                "No",
                acp::PermissionOptionKind::RejectOnce,
            ),
        ],
    );
    let options = request.options.clone();
    agent.permission_queue.push_back(PermissionViewState {
        request: codel_acp_lib::AcpArgs {
            request,
            response_tx,
        },
        id: 1,
        focus: PermissionFocus::Options,
        options,
        active_idx: 0,
        bash_highlights: None,
        bash_selection_count: 0,
        bash_deny_selection_count: 0,
        bash_command_raw: None,
        mcp_scope: None,
        title: "test-enable-always-approve".to_string(),
        description: vec![],
        args_expanded: false,
        desc_scroll: 0,
        subagent_label: None,
        options_area_height: 0,
        options_scroll_offset: 0,
    });
    response_rx
}
const POLICY_WARNING: &str =
    codel_workspace::permission::resolution::YoloPinReason::DisableBypassPermissionsMode
        .message();
fn agent_toast(app: &AppView) -> Option<String> {
    test_agent(app, AgentId(0))
        .toast
        .as_ref()
        .map(|(s, _)| s.clone())
}
/// Use the `theme_cache::test_lock` to serialize tests that touch the in-memory theme state (single mutable global).
/// Mirrors the pattern used by `theme::cache::tests`.
fn with_theme_test_env(f: impl FnOnce()) {
    let _guard = crate::theme::cache::test_lock()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    crate::theme::cache::reset_for_test();
    crate::theme::cache::seed_auto_theme_defaults_for_test();
    crate::theme::cache::set(crate::theme::ThemeKind::CodelNight);
    crate::theme::system_appearance::clear_mock();
    f();
    crate::theme::system_appearance::clear_mock();
    crate::theme::cache::reset_for_test();
}
fn agent_scrollback_len(app: &AppView) -> usize {
    app.agents.get(&AgentId(0)).unwrap().scrollback.len()
}
use crate::scrollback::blocks::UserPromptBlock;
/// Helper: open the dashboard against an existing `app`.
fn open_dashboard(app: &mut AppView) {
    let _ = dispatch_open_dashboard(app);
}
/// Display-order list of selectable row ids, the same order `dashboard_neighbor_row` and the renderer walk.
fn dashboard_row_order(app: &AppView) -> Vec<crate::views::dashboard::DashboardRowId> {
    super::dashboard::dashboard_focusables(app)
        .into_iter()
        .filter_map(|f| match f {
            crate::views::dashboard::Focusable::Row(id) => Some(id),
            crate::views::dashboard::Focusable::Section(_)
            | crate::views::dashboard::Focusable::IdleOverflow => None,
        })
        .collect()
}
/// Build a synthetic `PermissionViewState` with the given id and options, and push it onto the agent's permission_queue.
///
/// Returns the response receiver so tests can assert the response was actually sent through the oneshot, not merely that the queue was popped.
fn push_synthetic_permission(
    agent: &mut crate::app::agent_view::AgentView,
    id: usize,
    options: Vec<(&str, &str)>,
) -> tokio::sync::oneshot::Receiver<Result<acp::RequestPermissionResponse, acp::Error>> {
    use crate::views::permission_view::{PermissionFocus, PermissionViewState};
    let (tx, rx) =
        tokio::sync::oneshot::channel::<Result<acp::RequestPermissionResponse, acp::Error>>();
    let request = codel_acp_lib::AcpArgs {
        request: acp::RequestPermissionRequest::new(
            acp::SessionId::new(std::sync::Arc::from("sess-1")),
            acp::ToolCallUpdate::new(
                acp::ToolCallId::new(std::sync::Arc::from("tc-1")),
                acp::ToolCallUpdateFields::default(),
            ),
            options
                .iter()
                .map(|(oid, name)| {
                    acp::PermissionOption::new(
                        acp::PermissionOptionId::new(std::sync::Arc::from(*oid)),
                        name.to_string(),
                        if *oid == "reject" {
                            acp::PermissionOptionKind::RejectOnce
                        } else {
                            acp::PermissionOptionKind::AllowOnce
                        },
                    )
                })
                .collect(),
        ),
        response_tx: tx,
    };
    let opts = request.request.options.clone();
    let state = PermissionViewState {
        request,
        id,
        focus: PermissionFocus::Options,
        options: opts,
        active_idx: 0,
        bash_highlights: None,
        bash_selection_count: 0,
        bash_deny_selection_count: 0,
        bash_command_raw: None,
        mcp_scope: None,
        title: "Test permission".to_string(),
        description: Vec::new(),
        args_expanded: false,
        desc_scroll: 0,
        subagent_label: None,
        options_area_height: 0,
        options_scroll_offset: 0,
    };
    agent.permission_queue.push_back(state);
    rx
}
const MOUSE_OFF_STICKY: &str = crate::app::MOUSE_OFF_HINT_SCROLLBACK;
fn reset_mouse_capture_enabled(on: bool) {
    crate::app::MOUSE_CAPTURE_ENABLED.store(on, std::sync::atomic::Ordering::Release);
}
fn mouse_capture_is_enabled() -> bool {
    crate::app::MOUSE_CAPTURE_ENABLED.load(std::sync::atomic::Ordering::Acquire)
}
