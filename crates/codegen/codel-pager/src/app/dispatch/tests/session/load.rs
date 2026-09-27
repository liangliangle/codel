//! Tests for session loading, restore, pickers, and deep search.
use super::*;
fn expect_agent(app: &AppView, id: AgentId) -> &AgentView {
    let Some(agent) = app.agents.get(&id) else {
        panic!("expected agent {id:?}");
    };
    agent
}
use crate::views::modal::ActiveModal;
use codel_shell::session::unified_list::ListScope;
/// Opening the cancel-turn picker while scrollback is focused must hand keyboard focus to the picker.
/// Otherwise up/down keys go to scrollback and the modal is only navigable via mouse.
#[test]
fn cancel_turn_picker_grabs_focus_from_scrollback() {
    let mut app = test_app_with_agent();
    let id = AgentId(0);
    {
        let agent = app.agents.get_mut(&id).unwrap();
        agent.session.state = AgentState::TurnRunning;
        agent
            .subagent_sessions
            .insert("child-1".into(), make_test_subagent("child-1", "sa-1"));
        agent.active_pane = ActivePane::Scrollback;
    }
    let effects = dispatch(Action::CancelTurn, &mut app);
    assert!(effects.is_empty());
    assert!(expect_agent(&app, id).cancel_turn_view.is_some());
    assert_eq!(
        expect_agent(&app, id).active_pane,
        ActivePane::Prompt,
        "picker should steal focus from scrollback so keyboard navigation works"
    );
}
#[test]
fn session_loaded_with_flag_emits_five_fetches_and_clears_flag() {
    use crate::views::extensions_modal::{ExtensionsModalState, ExtensionsTab};
    let mut app = test_app_with_agent();
    let id = AgentId(0);
    {
        let a = app.agents.get_mut(&id).unwrap();
        a.session.session_id = None;
        a.pending_extensions_fetch = true;
        a.extensions_modal = Some(ExtensionsModalState::new(ExtensionsTab::Hooks));
    }
    let effects = dispatch(
        Action::TaskComplete(TaskResult::SessionLoaded {
            agent_id: id,
            session_id: acp::SessionId::new("s"),
            models: None,
            modes: None,
            code_restored: false,
            restore_summary: None,
            restore_degree: None,
            running_prompt_id: None,
        }),
        &mut app,
    );
    assert_eq!(count_extension_fetches(&effects), 5);
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::FetchMcpsList { cache: true, .. }))
    );
    assert!(!expect_agent(&app, id).pending_extensions_fetch);
}
#[test]
fn session_restored_does_not_consume_flag_and_defers_to_load() {
    use crate::views::extensions_modal::{ExtensionsModalState, ExtensionsTab};
    let mut app = test_app_with_agent();
    let id = AgentId(0);
    {
        let a = app.agents.get_mut(&id).unwrap();
        a.session.session_id = None;
        a.pending_extensions_fetch = true;
        a.extensions_modal = Some(ExtensionsModalState::new(ExtensionsTab::Hooks));
    }
    let effects = dispatch(
        Action::TaskComplete(TaskResult::SessionRestored {
            agent_id: id,
            local_session_id: "s".to_string(),
        }),
        &mut app,
    );
    assert_eq!(count_extension_fetches(&effects), 0);
    assert!(expect_agent(&app, id).pending_extensions_fetch);
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::LoadSession { .. }))
    );
}
#[test]
fn session_load_failed_clears_flag_no_fetches() {
    use crate::views::extensions_modal::{ExtensionsModalState, ExtensionsTab};
    let mut app = test_app_with_agent();
    let id = AgentId(0);
    {
        let a = app.agents.get_mut(&id).unwrap();
        a.pending_extensions_fetch = true;
        a.extensions_modal = Some(ExtensionsModalState::new(ExtensionsTab::Hooks));
    }
    let effects = dispatch(
        Action::TaskComplete(TaskResult::SessionLoadFailed {
            agent_id: id,
            session_id: acp::SessionId::new("s"),
            error: "boom".to_string(),
        }),
        &mut app,
    );
    assert_eq!(count_extension_fetches(&effects), 0);
    assert!(!expect_agent(&app, id).pending_extensions_fetch);
}
/// `SessionRestored` under `--chat` refuses a non-conversation local Build row (no LoadSession; agent torn down).
#[test]
fn session_restored_refuses_local_build_under_chat_mode() {
    let mut app = test_app_with_agent();
    let id = *app.agents.keys().next().unwrap();
    app.chat_mode = true;
    let cwd = app.cwd.clone();
    let session_id = format!("restored-build-{}", std::process::id());
    let sess_dir = plant_local_build_session(&cwd, &session_id);
    let effects = dispatch(
        Action::TaskComplete(TaskResult::SessionRestored {
            agent_id: id,
            local_session_id: session_id,
        }),
        &mut app,
    );
    let _ = std::fs::remove_dir_all(&sess_dir);
    assert!(
        effects.is_empty(),
        "SessionRestored must refuse Build under --chat, got {effects:?}"
    );
    assert!(
        !app.agents.contains_key(&id),
        "placeholder agent must be removed on refuse"
    );
}
/// The `SessionRestored` follow-up LoadSession stays `chat_kind: false` (not a picker conversation entry).
/// Sticky `--chat` with no local disk still opens as chat, so `conversation_entry` and the rename kind are Chat.
#[test]
fn session_restored_sticky_chat_sets_conversation_entry() {
    let mut app = test_app_with_agent();
    let id = *app.agents.keys().next().unwrap();
    app.chat_mode = true;
    let effects = dispatch(
        Action::TaskComplete(TaskResult::SessionRestored {
            agent_id: id,
            local_session_id: "restored_no_disk".into(),
        }),
        &mut app,
    );
    assert!(matches!(
        effects.as_slice(),
        [Effect::LoadSession {
            session_id,
            chat_kind: false,
            ..
        }] if session_id == "restored_no_disk"
    ));
    let agent = app.agents.get(&id).expect("agent kept");
    assert!(agent.chat_kind, "agent UI bit comes from sticky --chat");
    assert!(
        agent.conversation_entry,
        "sticky --chat restore with no local disk opens as chat (rename kind)"
    );
    assert_eq!(
        agent.rename_kind(),
        codel_shell::session::unified_list::SessionKind::Chat
    );
}
#[test]
fn session_loaded_drains_pending_first_prompt_to_front() {
    let mut app = fork_test_app();
    dispatch(
        Action::Fork(fork_args(Some(false), Some("first directive"))),
        &mut app,
    );
    let new_id = AgentId(1);
    app.agents
        .get_mut(&new_id)
        .unwrap()
        .session
        .enqueue_prompt("user-typed prompt".into());
    app.agents.get_mut(&new_id).unwrap().session.session_id = Some("new-fork-sid".into());
    dispatch(
        Action::TaskComplete(TaskResult::SessionLoaded {
            agent_id: new_id,
            session_id: "new-fork-sid".into(),
            models: None,
            modes: None,
            code_restored: false,
            restore_summary: None,
            restore_degree: None,
            running_prompt_id: None,
        }),
        &mut app,
    );
    let queue: Vec<_> = expect_agent(&app, new_id)
        .session
        .pending_prompts
        .iter()
        .map(|p| p.text.clone())
        .collect();
    assert_eq!(queue, vec!["user-typed prompt".to_string()]);
    assert!(
        expect_agent(&app, new_id).pending_first_prompt.is_none(),
        "drained prompt must be cleared"
    );
}
#[test]
fn session_loaded_with_no_pending_first_prompt_does_not_enqueue() {
    let mut app = fork_test_app();
    let id = AgentId(0);
    let queue_before = expect_agent(&app, id).session.pending_prompts.len();
    dispatch(
        Action::TaskComplete(TaskResult::SessionLoaded {
            agent_id: id,
            session_id: "test-session".into(),
            models: None,
            modes: None,
            code_restored: false,
            restore_summary: None,
            restore_degree: None,
            running_prompt_id: None,
        }),
        &mut app,
    );
    assert_eq!(
        expect_agent(&app, id).session.pending_prompts.len(),
        queue_before,
        "no enqueue when pending_first_prompt is None"
    );
}
#[test]
fn session_load_failed_clears_pending_first_prompt() {
    let mut app = fork_test_app();
    let id = AgentId(0);
    app.agents.get_mut(&id).unwrap().pending_first_prompt = Some("orphaned directive".into());
    dispatch(
        Action::TaskComplete(TaskResult::SessionLoadFailed {
            agent_id: id,
            session_id: "test-session".into(),
            error: "boom".into(),
        }),
        &mut app,
    );
    assert!(
        expect_agent(&app, id).pending_first_prompt.is_none(),
        "load failure must drop the directive"
    );
}
#[test]
fn reanchor_grouped_selection_lands_on_a_row() {
    use crate::views::picker::PickerState;
    let map: Vec<Option<()>> = vec![None, Some(()), Some(())];
    let mut st = PickerState::default();
    st.selected = 9;
    reanchor_grouped_selection(&mut st, &map);
    assert_eq!(st.selected, 2);
    let mut st = PickerState::default();
    reanchor_grouped_selection(&mut st, &map);
    assert_eq!(st.selected, 1);
    let empty: Vec<Option<()>> = vec![];
    let mut st = PickerState::default();
    st.selected = 5;
    reanchor_grouped_selection(&mut st, &empty);
    assert_eq!(st.selected, 0);
}
#[test]
fn entry_title_loading_when_no_session_id() {
    use crate::views::session_title::entry_title;
    let mut app = test_app_with_agent();
    if let Some(a) = app.agents.get_mut(&AgentId(0)) {
        a.session.session_id = None;
    }
    let title = entry_title(expect_agent(&app, AgentId(0)));
    assert_eq!(title, "loading...");
}
/// Picking a conversation row dispatches a direct chat load, never local resolution or GCS restore.
#[test]
fn pick_conversation_row_dispatches_direct_chat_load() {
    let mut app = test_app_with_agent();
    open_session_picker_with(&mut app, vec![make_conversation_entry("conv-pick-1")]);
    let effects = dispatch(Action::PickSession(0), &mut app);
    assert!(
        matches!(
            effects.as_slice(),
            [Effect::LoadSession {
                session_id,
                session_cwd: None,
                chat_kind: true,
                ..
            }] if session_id == "conv-pick-1"
        ),
        "expected a direct chat LoadSession, got {effects:?}"
    );
}
/// Canary: a remote Build row not on disk still takes the GCS-restore path.
#[test]
fn pick_remote_build_row_still_restores() {
    let mut app = test_app_with_agent();
    let id = format!("remote-only-{}", std::process::id());
    let mut e = make_picker_entry(&id, "/r");
    e.source = "remote".into();
    open_session_picker_with(&mut app, vec![e]);
    let effects = dispatch(Action::PickSession(0), &mut app);
    assert!(
        matches!(
            effects.as_slice(),
            [Effect::RestoreAndLoadSession { session_id, .. }] if *session_id == id
        ),
        "expected RestoreAndLoadSession, got {effects:?}"
    );
}
/// A content-search hit that matches a conversation row shown in the same picker also dispatches the direct chat load.
#[test]
fn pick_content_session_conversation_row_dispatches_direct_chat_load() {
    let mut app = test_app_with_agent();
    open_session_picker_with(&mut app, vec![make_conversation_entry("conv-hit-1")]);
    let effects = dispatch(
        Action::PickContentSession {
            session_id: "conv-hit-1".into(),
            cwd: String::new(),
        },
        &mut app,
    );
    assert!(
        matches!(
            effects.as_slice(),
            [Effect::LoadSession {
                session_id,
                session_cwd: None,
                chat_kind: true,
                ..
            }] if session_id == "conv-hit-1"
        ),
        "expected a direct chat LoadSession, got {effects:?}"
    );
}
/// Worktree resume is refused for conversation rows (no cwd to check out); the refusal must not set the one-shot chat bit.
#[test]
fn pick_session_in_worktree_refuses_conversation_row() {
    let mut app = test_app_with_agent();
    open_session_picker_with(&mut app, vec![make_conversation_entry("conv-wt-1")]);
    let effects = dispatch(Action::PickSessionInWorktree(0), &mut app);
    assert!(effects.is_empty(), "no worktree effects, got {effects:?}");
    assert!(
        !app.deferred_startup.pending_chat,
        "refusal must not set the one-shot chat bit"
    );
    assert!(read_toast(&app).contains("worktree"));
}
/// Modal `/resume` surface: the debounce expiry validates against the MODAL's deep-search seq (the welcome counter still sits at 0 here).
#[test]
fn build_mode_modal_debounce_expiry_validates_modal_seq() {
    use crate::views::modal::ActiveModal;
    let mut app = test_app_with_agent();
    open_session_picker_with(&mut app, vec![make_picker_entry("local-dm-1", "/r")]);
    if let Some(ActiveModal::SessionPicker { state, .. }) = get_active_agent_mut(&mut app)
        .expect("active agent")
        .active_modal
        .as_mut()
    {
        state.set_query("abc");
    }
    let effects = dispatch(Action::TriggerDeepSearch, &mut app);
    assert!(
        matches!(
            effects.as_slice(),
            [Effect::DebounceSessionSearch { seq: 1, .. }]
        ),
        "modal query must arm the debounce, got {effects:?}"
    );
    let effects = dispatch(
        Action::TaskComplete(TaskResult::SessionSearchDebounceExpired {
            host: SessionPickerHost::AgentModal,
            generation: modal_picker_generation(&app),
            query: "abc".into(),
            seq: 1,
        }),
        &mut app,
    );
    assert!(
        matches!(
            effects.as_slice(),
            [Effect::DeepSearchSessions { query, seq: 1, .. }] if query == "abc"
        ),
        "expiry must validate against the modal seq, got {effects:?}"
    );
}
/// The modal arms a debounce, then closes.
/// The dismissal bump lands on the WELCOME counter and collides with the carried modal seq (both 1 here).
/// The expiry must still be dropped because no picker surface is live.
#[test]
fn build_mode_modal_close_drops_armed_debounce_despite_seq_collision() {
    use crate::views::modal::ActiveModal;
    let mut app = test_app_with_agent();
    open_session_picker_with(&mut app, vec![make_picker_entry("local-cl-1", "/r")]);
    if let Some(ActiveModal::SessionPicker { state, .. }) = get_active_agent_mut(&mut app)
        .expect("active agent")
        .active_modal
        .as_mut()
    {
        state.set_query("abc");
    }
    let _ = dispatch(Action::TriggerDeepSearch, &mut app);
    assert_eq!(
        app.session_picker_deep_search_seq, 0,
        "modal arm must not touch the welcome counter"
    );
    let armed_generation = modal_picker_generation(&app);
    get_active_agent_mut(&mut app)
        .expect("active agent")
        .active_modal = None;
    let _ = dispatch(Action::SessionPickerClosed, &mut app);
    assert_eq!(
        app.session_picker_deep_search_seq, 1,
        "collision precondition: welcome counter equals the armed modal seq"
    );
    let effects = dispatch(
        Action::TaskComplete(TaskResult::SessionSearchDebounceExpired {
            host: SessionPickerHost::AgentModal,
            generation: armed_generation,
            query: "abc".into(),
            seq: 1,
        }),
        &mut app,
    );
    assert!(
        effects.is_empty(),
        "modal-armed expiry must not search after the modal closed, got {effects:?}"
    );
}
/// The modal `/resume` picker's query (not the welcome picker's) drives the chat-mode search when a modal is open.
#[test]
fn chat_mode_search_reads_modal_query_first() {
    use crate::views::modal::ActiveModal;
    let mut app = test_app_with_agent();
    app.chat_mode = true;
    app.session_picker_state.set_query("welcome-query");
    open_session_picker_with(&mut app, vec![make_conversation_entry("conv-mq-1")]);
    if let Some(ActiveModal::SessionPicker { state, .. }) = get_active_agent_mut(&mut app)
        .expect("active agent")
        .active_modal
        .as_mut()
    {
        state.set_query("modal-query");
    }
    let effects = dispatch(Action::ForceDeepSearch, &mut app);
    assert!(
        matches!(
            effects.as_slice(),
            [Effect::FetchSessionList { query: Some(q), .. }] if q == "modal-query"
        ),
        "modal query must win over the welcome picker's, got {effects:?}"
    );
}
/// Out-of-order list completions: only the response for the current seq lands; stale successes and failures are both dropped.
#[test]
fn stale_session_list_responses_are_dropped() {
    let mut app = test_app_with_agent();
    app.chat_mode = true;
    app.session_picker_state.set_query("abc");
    let _ = dispatch(Action::ForceDeepSearch, &mut app);
    app.session_picker_state.set_query("abcd");
    let _ = dispatch(Action::ForceDeepSearch, &mut app);
    let _ = dispatch(
        Action::TaskComplete(TaskResult::SessionListLoaded {
            host: SessionPickerHost::Welcome,
            generation: app.session_picker_generation,
            scope: ListScope::Cwd,
            sessions: vec![make_conversation_entry("conv-stale-1")],
            partial: None,
            seq: 1,
            query: None,
        }),
        &mut app,
    );
    assert!(
        app.session_picker_entries.is_none(),
        "stale list result must be dropped"
    );
    let _ = dispatch(
        Action::TaskComplete(TaskResult::SessionListFailed {
            host: SessionPickerHost::Welcome,
            generation: app.session_picker_generation,
            error: "boom".into(),
            seq: 1,
            query: Some("abc".into()),
        }),
        &mut app,
    );
    assert!(
        expect_agent(&app, AgentId(0)).toast.is_none(),
        "stale list failure must not toast"
    );
    let _ = dispatch(
        Action::TaskComplete(TaskResult::SessionListLoaded {
            host: SessionPickerHost::Welcome,
            generation: app.session_picker_generation,
            scope: ListScope::Cwd,
            sessions: vec![make_conversation_entry("conv-fresh-2")],
            partial: None,
            seq: 2,
            query: Some("abcd".into()),
        }),
        &mut app,
    );
    let ids: Vec<&str> = app
        .session_picker_entries
        .as_deref()
        .unwrap_or_default()
        .iter()
        .map(|e| e.id.as_str())
        .collect();
    assert_eq!(ids, ["conv-fresh-2"], "current-seq result must land");
    assert_eq!(
        app.session_picker_entries_query.as_deref(),
        Some("abcd"),
        "search results must be stamped with their fetch query"
    );
    assert!(
        !app.session_picker_content_loading,
        "landing the search must drop the in-flight indicator"
    );
}
/// Modal `/resume` surface: a current-seq search response replaces the modal's entries (query-stamped, cursor re-anchored).
/// A stale one leaves them untouched.
#[test]
fn modal_search_response_lands_and_stale_is_dropped() {
    use crate::views::modal::ActiveModal;
    let mut app = test_app_with_agent();
    app.chat_mode = true;
    open_session_picker_with(&mut app, vec![make_conversation_entry("conv-old")]);
    if let Some(ActiveModal::SessionPicker { state, .. }) = get_active_agent_mut(&mut app)
        .expect("active agent")
        .active_modal
        .as_mut()
    {
        state.set_query("hit");
        state.selected = 3;
    }
    let _ = dispatch(Action::ForceDeepSearch, &mut app);
    let _ = dispatch(
        Action::TaskComplete(TaskResult::SessionListLoaded {
            host: SessionPickerHost::AgentModal,
            generation: modal_picker_generation(&app),
            scope: ListScope::Cwd,
            sessions: vec![make_conversation_entry("conv-hit-1")],
            partial: None,
            seq: 1,
            query: Some("hit".into()),
        }),
        &mut app,
    );
    {
        let agent = get_active_agent(&app).expect("active agent");
        let Some(ActiveModal::SessionPicker {
            entries: Some(list),
            entries_query,
            content_loading,
            state,
            ..
        }) = agent.active_modal.as_ref()
        else {
            panic!("expected SessionPicker modal with entries");
        };
        assert_eq!(
            list.first().map(|e| e.id.as_str()),
            Some("conv-hit-1"),
            "search results land in the modal"
        );
        assert_eq!(
            entries_query.as_deref(),
            Some("hit"),
            "modal entries must carry their fetch-query stamp"
        );
        assert!(!content_loading, "landing clears the modal indicator");
        assert_eq!(state.selected, 1, "cursor re-anchors onto the result row");
    }
    if let Some(ActiveModal::SessionPicker { state, .. }) = get_active_agent_mut(&mut app)
        .expect("active agent")
        .active_modal
        .as_mut()
    {
        state.set_query("hits");
    }
    let _ = dispatch(Action::ForceDeepSearch, &mut app);
    let _ = dispatch(
        Action::TaskComplete(TaskResult::SessionListLoaded {
            host: SessionPickerHost::AgentModal,
            generation: modal_picker_generation(&app),
            scope: ListScope::Cwd,
            sessions: vec![make_conversation_entry("conv-stale-m")],
            partial: None,
            seq: 1,
            query: Some("hit".into()),
        }),
        &mut app,
    );
    let agent = get_active_agent(&app).expect("active agent");
    let Some(ActiveModal::SessionPicker {
        entries: Some(list),
        content_loading,
        ..
    }) = agent.active_modal.as_ref()
    else {
        panic!("expected SessionPicker modal with entries");
    };
    assert_eq!(
        list.first().map(|e| e.id.as_str()),
        Some("conv-hit-1"),
        "stale modal response must be dropped"
    );
    assert!(
        content_loading,
        "stale response must not clear the newer search's indicator"
    );
}
/// With the modal gone the response would fall through to the WELCOME picker fields, whose search box never held the modal's query.
/// That would leave mismatched entries and a stale fetch-query stamp for the next resume view.
/// The close must bump the seq so the late response is dropped.
#[test]
fn modal_close_drops_in_flight_search_response() {
    use crate::views::modal::ActiveModal;
    let mut app = test_app_with_agent();
    app.chat_mode = true;
    open_session_picker_with(&mut app, vec![make_conversation_entry("conv-cl-1")]);
    if let Some(ActiveModal::SessionPicker { state, .. }) = get_active_agent_mut(&mut app)
        .expect("active agent")
        .active_modal
        .as_mut()
    {
        state.set_query("hit");
    }
    let _ = dispatch(Action::ForceDeepSearch, &mut app);
    let seq = app.session_picker_list_seq;
    let generation = modal_picker_generation(&app);
    get_active_agent_mut(&mut app)
        .expect("active agent")
        .active_modal = None;
    let effects = dispatch(Action::SessionPickerClosed, &mut app);
    assert!(
        effects.is_empty(),
        "close is a pure invalidation, got {effects:?}"
    );
    let _ = dispatch(
        Action::TaskComplete(TaskResult::SessionListLoaded {
            host: SessionPickerHost::AgentModal,
            generation,
            scope: ListScope::Cwd,
            sessions: vec![make_conversation_entry("conv-late-1")],
            partial: None,
            seq,
            query: Some("hit".into()),
        }),
        &mut app,
    );
    assert!(
        app.session_picker_entries.is_none(),
        "post-close response must not land on the welcome picker"
    );
    assert!(
        app.session_picker_entries_query.is_none(),
        "no stale fetch-query stamp may leak to the welcome picker"
    );
}
/// Sibling of the close test: PICKING from the modal dismisses it too, so the same in-flight chat-mode search must be invalidated.
/// Otherwise its late response falls through to the WELCOME picker fields in `handle_session_list_loaded`'s fallback.
#[test]
fn modal_pick_drops_in_flight_search_response() {
    use crate::views::modal::ActiveModal;
    let mut app = test_app_with_agent();
    app.chat_mode = true;
    open_session_picker_with(&mut app, vec![make_conversation_entry("conv-pk-1")]);
    if let Some(ActiveModal::SessionPicker { state, .. }) = get_active_agent_mut(&mut app)
        .expect("active agent")
        .active_modal
        .as_mut()
    {
        state.set_query("hit");
    }
    let _ = dispatch(Action::ForceDeepSearch, &mut app);
    let seq = app.session_picker_list_seq;
    let generation = modal_picker_generation(&app);
    let effects = dispatch(Action::PickSession(0), &mut app);
    assert!(
        matches!(effects.as_slice(), [Effect::LoadSession { .. }]),
        "pick must still load the session, got {effects:?}"
    );
    assert!(
        app.session_picker_list_seq > seq,
        "pick must invalidate the in-flight search"
    );
    let _ = dispatch(
        Action::TaskComplete(TaskResult::SessionListLoaded {
            host: SessionPickerHost::AgentModal,
            generation,
            scope: ListScope::Cwd,
            sessions: vec![make_conversation_entry("conv-late-p")],
            partial: None,
            seq,
            query: Some("hit".into()),
        }),
        &mut app,
    );
    assert!(
        app.session_picker_entries.is_none(),
        "post-pick response must not land on the welcome picker"
    );
    assert!(
        app.session_picker_entries_query.is_none(),
        "no stale fetch-query stamp may leak to the welcome picker"
    );
}
/// Build mode: an Only-policy modal fetch completing after close and reopen is dropped by the host and incarnation routing.
/// It cannot leak its rows or query stamp into the replacement Exclude-policy modal.
#[test]
fn build_mode_close_and_reopen_drop_opposite_policy_response() {
    use codel_shell::session::unified_list::HeadlessPolicy;
    let mut app = test_app_with_agent();
    assert!(!app.chat_mode);
    let _ = dispatch(Action::ShowSessionPicker, &mut app);
    let old_generation = modal_picker_generation(&app);
    let only_effects = dispatch(Action::CycleSessionSourceFilter, &mut app);
    let [
        Effect::FetchSessionList {
            host: SessionPickerHost::AgentModal,
            generation,
            seq: only_seq,
            headless_policy: HeadlessPolicy::Only,
            ..
        },
    ] = only_effects.as_slice()
    else {
        panic!("expected modal Headless fetch, got {only_effects:?}");
    };
    assert_eq!(*generation, old_generation);
    let only_seq = *only_seq;
    get_active_agent_mut(&mut app)
        .expect("active agent")
        .active_modal = None;
    let _ = dispatch(Action::SessionPickerClosed, &mut app);
    let exclude_effects = dispatch(Action::ShowSessionPicker, &mut app);
    let Some(Effect::FetchSessionList {
        host: SessionPickerHost::AgentModal,
        generation: new_generation,
        seq: exclude_seq,
        headless_policy,
        ..
    }) = exclude_effects.first()
    else {
        panic!("expected reopened picker fetch, got {exclude_effects:?}");
    };
    assert_eq!(*headless_policy, HeadlessPolicy::Exclude);
    assert!(*new_generation > old_generation);
    assert!(*exclude_seq > only_seq);
    let new_generation = *new_generation;
    let exclude_seq = *exclude_seq;
    let effects = dispatch(
        Action::TaskComplete(TaskResult::SessionListLoaded {
            host: SessionPickerHost::AgentModal,
            generation: old_generation,
            scope: ListScope::Cwd,
            sessions: vec![make_picker_entry("stale-generation", "/tmp/repo")],
            partial: None,
            seq: exclude_seq,
            query: Some("modal-only".into()),
        }),
        &mut app,
    );
    assert!(effects.is_empty());
    let Some(ActiveModal::SessionPicker {
        entries,
        entries_query,
        loading,
        ..
    }) = expect_agent(&app, AgentId(0)).active_modal.as_ref()
    else {
        panic!("reopened picker missing");
    };
    assert!(entries.is_none(), "stale incarnation rows must be dropped");
    assert!(entries_query.is_none(), "stale query stamp must be dropped");
    assert!(
        *loading,
        "the replacement modal keeps waiting for its fetch"
    );
    let effects = dispatch(
        Action::TaskComplete(TaskResult::SessionListLoaded {
            host: SessionPickerHost::AgentModal,
            generation: new_generation,
            scope: ListScope::Cwd,
            sessions: vec![make_picker_entry("stale-policy", "/tmp/repo")],
            partial: None,
            seq: only_seq,
            query: None,
        }),
        &mut app,
    );
    assert!(effects.is_empty());
    let Some(ActiveModal::SessionPicker {
        entries, loading, ..
    }) = expect_agent(&app, AgentId(0)).active_modal.as_ref()
    else {
        panic!("reopened picker missing");
    };
    assert!(
        entries.is_none(),
        "obsolete Only-policy rows must be dropped"
    );
    assert!(
        *loading,
        "the replacement modal keeps waiting for its fetch"
    );
    assert!(app.session_picker_entries.is_none());
    assert!(app.session_picker_entries_query.is_none());
    let _ = dispatch(
        Action::TaskComplete(TaskResult::SessionListLoaded {
            host: SessionPickerHost::AgentModal,
            generation: new_generation,
            scope: ListScope::Cwd,
            sessions: vec![make_picker_entry("current", "/tmp/repo")],
            partial: None,
            seq: exclude_seq,
            query: None,
        }),
        &mut app,
    );
    let Some(ActiveModal::SessionPicker {
        entries: Some(entries),
        ..
    }) = expect_agent(&app, AgentId(0)).active_modal.as_ref()
    else {
        panic!("reopened picker missing");
    };
    assert_eq!(entries.first().map(|e| e.id.as_str()), Some("current"));
}
/// A zero-hit search is a normal outcome: the picker shows an empty list.
/// It never shows the misleading "No sessions found for this directory" toast, which would fire on every keystroke.
/// Plain fetches keep the toast.
#[test]
fn zero_hit_search_shows_empty_list_without_toast() {
    let mut app = test_app_with_agent();
    app.chat_mode = true;
    app.session_picker_state.set_query("zzz");
    let _ = dispatch(Action::ForceDeepSearch, &mut app);
    let _ = dispatch(
        Action::TaskComplete(TaskResult::SessionListLoaded {
            host: SessionPickerHost::Welcome,
            generation: app.session_picker_generation,
            scope: ListScope::Cwd,
            sessions: vec![],
            partial: None,
            seq: 1,
            query: Some("zzz".into()),
        }),
        &mut app,
    );
    assert!(
        app.session_picker_entries
            .as_ref()
            .is_some_and(|v| v.is_empty()),
        "zero-hit search keeps an empty (not None) list"
    );
    assert!(
        expect_agent(&app, AgentId(0)).toast.is_none(),
        "zero-hit search must not toast"
    );
    assert!(!app.session_picker_content_loading);
    let _ = dispatch(Action::FetchSessionList, &mut app);
    let _ = dispatch(
        Action::TaskComplete(TaskResult::SessionListLoaded {
            host: SessionPickerHost::Welcome,
            generation: app.session_picker_generation,
            scope: ListScope::Cwd,
            sessions: vec![],
            partial: None,
            seq: 2,
            query: None,
        }),
        &mut app,
    );
    assert!(app.session_picker_entries.is_none());
    assert!(
        read_toast(&app).contains("No sessions found"),
        "plain empty fetch keeps the generic toast"
    );
}
/// A current-seq FAILED search clears the in-flight indicator and the fuzzy-bypass stamp, and shows the toast.
/// A stuck "Searching…" or a stale stamp on the error state would outlive the entries it described.
#[test]
fn current_seq_failed_search_clears_indicator_and_stamp() {
    let mut app = test_app_with_agent();
    app.chat_mode = true;
    app.session_picker_entries_query = Some("old".into());
    app.session_picker_state.set_query("hit");
    let _ = dispatch(Action::ForceDeepSearch, &mut app);
    assert!(app.session_picker_content_loading);
    let _ = dispatch(
        Action::TaskComplete(TaskResult::SessionListFailed {
            host: SessionPickerHost::Welcome,
            generation: app.session_picker_generation,
            error: "boom".into(),
            seq: 1,
            query: Some("hit".into()),
        }),
        &mut app,
    );
    assert!(
        !app.session_picker_content_loading,
        "failed search must drop the in-flight indicator"
    );
    assert!(
        app.session_picker_entries_query.is_none(),
        "failed search must clear the fuzzy-bypass stamp"
    );
    assert!(app.session_picker_entries.is_none());
    assert!(read_toast(&app).contains("Couldn't load sessions"));
}
/// Modal branch of the gated failure clear: a current-seq FAILED search clears the modal's indicator and stamp.
/// A plain (query-less) failure leaves the modal's deep-search spinner alone.
#[test]
fn modal_failed_search_clears_indicator_and_plain_failure_preserves_spinner() {
    use crate::views::modal::ActiveModal;
    let mut app = test_app_with_agent();
    app.chat_mode = true;
    open_session_picker_with(&mut app, vec![make_conversation_entry("conv-mf-1")]);
    if let Some(ActiveModal::SessionPicker {
        state,
        entries_query,
        ..
    }) = get_active_agent_mut(&mut app)
        .expect("active agent")
        .active_modal
        .as_mut()
    {
        state.set_query("hit");
        *entries_query = Some("old".into());
    }
    let _ = dispatch(Action::ForceDeepSearch, &mut app);
    let _ = dispatch(
        Action::TaskComplete(TaskResult::SessionListFailed {
            host: SessionPickerHost::AgentModal,
            generation: modal_picker_generation(&app),
            error: "boom".into(),
            seq: 1,
            query: Some("hit".into()),
        }),
        &mut app,
    );
    {
        let agent = get_active_agent(&app).expect("active agent");
        let Some(ActiveModal::SessionPicker {
            entries,
            loading,
            content_loading,
            entries_query,
            ..
        }) = agent.active_modal.as_ref()
        else {
            panic!("expected SessionPicker modal");
        };
        assert!(
            !content_loading,
            "failed search must drop the modal indicator"
        );
        assert!(
            entries_query.is_none(),
            "failed search must clear the modal stamp"
        );
        assert!(entries.is_none(), "failure drops the modal entries");
        assert!(!loading);
    }
    assert!(read_toast(&app).contains("Couldn't load sessions"));
    let mut app = test_app_with_agent();
    open_session_picker_with(&mut app, vec![make_picker_entry("local-mf-1", "/r")]);
    let _ = dispatch(Action::FetchSessionList, &mut app);
    if let Some(ActiveModal::SessionPicker { state, .. }) = get_active_agent_mut(&mut app)
        .expect("active agent")
        .active_modal
        .as_mut()
    {
        state.set_query("abc");
    }
    let effects = dispatch(Action::ForceDeepSearch, &mut app);
    assert!(
        matches!(effects.as_slice(), [Effect::DeepSearchSessions { .. }]),
        "Build-mode modal deep search armed, got {effects:?}"
    );
    let _ = dispatch(
        Action::TaskComplete(TaskResult::SessionListFailed {
            host: SessionPickerHost::AgentModal,
            generation: modal_picker_generation(&app),
            error: "boom".into(),
            seq: app.session_picker_list_seq,
            query: None,
        }),
        &mut app,
    );
    let agent = get_active_agent(&app).expect("active agent");
    let Some(ActiveModal::SessionPicker {
        content_loading, ..
    }) = agent.active_modal.as_ref()
    else {
        panic!("expected SessionPicker modal");
    };
    assert!(
        content_loading,
        "plain failure must not hide the modal deep-search spinner"
    );
}
/// Build-mode canary: `content_loading` belongs to the FTS5 deep search (guarded by `deep_search_seq`).
/// A plain (query-less) list response or failure landing while the deep search runs must NOT hide its spinner.
#[test]
fn build_mode_list_response_preserves_deep_search_spinner() {
    let mut app = test_app_with_agent();
    let _ = dispatch(Action::FetchSessionList, &mut app);
    app.session_picker_state.set_query("abc");
    let effects = dispatch(Action::ForceDeepSearch, &mut app);
    assert!(
        matches!(effects.as_slice(), [Effect::DeepSearchSessions { .. }]),
        "Build-mode deep search armed, got {effects:?}"
    );
    assert!(app.session_picker_content_loading, "deep search in flight");
    let _ = dispatch(
        Action::TaskComplete(TaskResult::SessionListLoaded {
            host: SessionPickerHost::Welcome,
            generation: app.session_picker_generation,
            scope: ListScope::Cwd,
            sessions: vec![make_picker_entry("local-1", "/r")],
            partial: None,
            seq: app.session_picker_list_seq,
            query: None,
        }),
        &mut app,
    );
    assert!(
        app.session_picker_content_loading,
        "plain list response must not hide the deep-search spinner"
    );
    assert!(app.session_picker_entries.is_some(), "entries still land");
    let _ = dispatch(
        Action::TaskComplete(TaskResult::SessionListFailed {
            host: SessionPickerHost::Welcome,
            generation: app.session_picker_generation,
            error: "boom".into(),
            seq: app.session_picker_list_seq,
            query: None,
        }),
        &mut app,
    );
    assert!(
        app.session_picker_content_loading,
        "plain list failure must not hide the deep-search spinner"
    );
}
/// Picker incarnation generations: every fetch reallocates the welcome picker's generation.
/// A fetch with the modal open overwrites the modal's constructed 0 placeholder with a fresh allocation (distinct from the welcome one).
/// A dismissal reallocates the welcome generation again.
#[test]
fn picker_generations_reallocate_on_fetch_and_dismissal() {
    use crate::views::modal::ActiveModal;
    let mut app = test_app_with_agent();
    assert_eq!(app.session_picker_generation, 0);
    let _ = dispatch(Action::FetchSessionList, &mut app);
    let welcome_after_fetch = app.session_picker_generation;
    assert!(welcome_after_fetch > 0, "fetch must allocate a generation");
    let _ = dispatch(Action::ShowSessionPicker, &mut app);
    let Some(&ActiveModal::SessionPicker { generation, .. }) = get_active_agent(&app)
        .expect("active agent")
        .active_modal
        .as_ref()
    else {
        panic!("expected SessionPicker modal");
    };
    assert!(
        generation > welcome_after_fetch,
        "open must overwrite the modal's 0 placeholder with a fresh allocation"
    );
    assert!(
        app.session_picker_generation > welcome_after_fetch,
        "the modal-open fetch reallocates the welcome generation too"
    );
    assert_ne!(
        generation, app.session_picker_generation,
        "generations are unique across hosts"
    );
    get_active_agent_mut(&mut app)
        .expect("active agent")
        .active_modal = None;
    let welcome_before_dismiss = app.session_picker_generation;
    let _ = dispatch(Action::SessionPickerClosed, &mut app);
    assert!(
        app.session_picker_generation > welcome_before_dismiss,
        "dismissal must reallocate the welcome generation"
    );
}
/// Reopening the modal picker starts a new incarnation.
/// The first incarnation's late list response is dropped; the reopened incarnation's own response applies.
/// The test also pins the producer side: the modal fetch carries the modal host and the modal's live generation.
#[test]
fn reopened_modal_drops_prior_incarnation_list_result() {
    use crate::views::modal::ActiveModal;
    let mut app = test_app_with_agent();
    assert!(!app.chat_mode);
    let effects = dispatch(Action::ShowSessionPicker, &mut app);
    let first_generation = modal_picker_generation(&app);
    let [
        Effect::FetchSessionList {
            host: SessionPickerHost::AgentModal,
            generation,
            seq: first_seq,
            ..
        },
    ] = effects.as_slice()
    else {
        panic!("expected modal fetch, got {effects:?}");
    };
    assert_eq!(*generation, first_generation);
    let first_seq = *first_seq;
    get_active_agent_mut(&mut app)
        .expect("active agent")
        .active_modal = None;
    let _ = dispatch(Action::SessionPickerClosed, &mut app);
    let second_effects = dispatch(Action::ShowSessionPicker, &mut app);
    let second_generation = modal_picker_generation(&app);
    let [
        Effect::FetchSessionList {
            host: SessionPickerHost::AgentModal,
            generation,
            seq: second_seq,
            ..
        },
    ] = second_effects.as_slice()
    else {
        panic!("expected reopened modal fetch, got {second_effects:?}");
    };
    assert_eq!(*generation, second_generation);
    assert!(second_generation > first_generation);
    assert!(*second_seq > first_seq);
    let second_seq = *second_seq;
    let effects = dispatch(
        Action::TaskComplete(TaskResult::SessionListLoaded {
            host: SessionPickerHost::AgentModal,
            generation: first_generation,
            scope: ListScope::Cwd,
            sessions: vec![make_picker_entry("stale-open", "/r")],
            partial: None,
            seq: first_seq,
            query: None,
        }),
        &mut app,
    );
    assert!(effects.is_empty());
    {
        let agent = get_active_agent(&app).expect("active agent");
        let Some(ActiveModal::SessionPicker {
            entries, loading, ..
        }) = agent.active_modal.as_ref()
        else {
            panic!("expected SessionPicker modal");
        };
        assert!(
            entries.is_none(),
            "stale-incarnation response must not land"
        );
        assert!(loading, "reopened modal keeps waiting for its own fetch");
    }
    let _ = dispatch(
        Action::TaskComplete(TaskResult::SessionListLoaded {
            host: SessionPickerHost::AgentModal,
            generation: second_generation,
            scope: ListScope::Cwd,
            sessions: vec![make_picker_entry("fresh-open", "/r")],
            partial: None,
            seq: second_seq,
            query: None,
        }),
        &mut app,
    );
    let agent = get_active_agent(&app).expect("active agent");
    let Some(ActiveModal::SessionPicker {
        entries: Some(list),
        loading,
        ..
    }) = agent.active_modal.as_ref()
    else {
        panic!("expected SessionPicker modal with entries");
    };
    assert_eq!(list.first().map(|e| e.id.as_str()), Some("fresh-open"));
    assert!(!loading);
}
/// A welcome-issued fetch whose response completes after a modal opened on top is dropped.
/// The modal-open fetch reallocated the welcome generation, so the response can neither retarget the modal nor land in the welcome picker's fields.
#[test]
fn welcome_fetch_response_does_not_retarget_open_modal() {
    use crate::views::modal::ActiveModal;
    let mut app = test_app_with_agent();
    assert!(!app.chat_mode);
    let effects = dispatch(Action::FetchSessionList, &mut app);
    let [
        Effect::FetchSessionList {
            host: SessionPickerHost::Welcome,
            generation: welcome_generation,
            seq: welcome_seq,
            ..
        },
    ] = effects.as_slice()
    else {
        panic!("expected welcome fetch, got {effects:?}");
    };
    let welcome_generation = *welcome_generation;
    let welcome_seq = *welcome_seq;
    let _ = dispatch(Action::ShowSessionPicker, &mut app);
    let effects = dispatch(
        Action::TaskComplete(TaskResult::SessionListLoaded {
            host: SessionPickerHost::Welcome,
            generation: welcome_generation,
            scope: ListScope::Cwd,
            sessions: vec![make_picker_entry("welcome-late", "/r")],
            partial: None,
            seq: welcome_seq,
            query: None,
        }),
        &mut app,
    );
    assert!(effects.is_empty());
    {
        let agent = get_active_agent(&app).expect("active agent");
        let Some(ActiveModal::SessionPicker {
            entries, loading, ..
        }) = agent.active_modal.as_ref()
        else {
            panic!("expected SessionPicker modal");
        };
        assert!(
            entries.is_none(),
            "welcome response must not land in the modal"
        );
        assert!(loading, "the modal keeps waiting for its own fetch");
    }
    assert!(
        app.session_picker_entries.is_none(),
        "the superseded welcome incarnation's response is dropped everywhere"
    );
}
/// A fetch issued for one agent's modal cannot land on another agent's modal: the generations differ, so the late result is dropped.
/// The requesting (now background) modal stays loading; nothing routes back to a non-active modal.
#[test]
fn modal_result_does_not_cross_agents_and_background_modal_starves() {
    use crate::views::modal::ActiveModal;
    let mut app = test_app_with_agent();
    assert!(!app.chat_mode);
    let a_effects = dispatch(Action::ShowSessionPicker, &mut app);
    let a_generation = modal_picker_generation(&app);
    let [Effect::FetchSessionList { seq: a_seq, .. }] = a_effects.as_slice() else {
        panic!("expected agent A modal fetch, got {a_effects:?}");
    };
    let a_seq = *a_seq;
    insert_placeholder_agent(&mut app, AgentId(1));
    app.active_view = ActiveView::Agent(AgentId(1));
    let _ = dispatch(Action::ShowSessionPicker, &mut app);
    let b_generation = modal_picker_generation(&app);
    assert_ne!(a_generation, b_generation);
    let effects = dispatch(
        Action::TaskComplete(TaskResult::SessionListLoaded {
            host: SessionPickerHost::AgentModal,
            generation: a_generation,
            scope: ListScope::Cwd,
            sessions: vec![make_picker_entry("agent-a-late", "/r")],
            partial: None,
            seq: a_seq,
            query: None,
        }),
        &mut app,
    );
    assert!(effects.is_empty());
    {
        let agent = get_active_agent(&app).expect("active agent");
        let Some(ActiveModal::SessionPicker {
            entries, loading, ..
        }) = agent.active_modal.as_ref()
        else {
            panic!("expected agent B's SessionPicker modal");
        };
        assert!(
            entries.is_none(),
            "another agent's result must not land here"
        );
        assert!(loading, "B's modal keeps waiting for its own fetch");
    }
    let Some(ActiveModal::SessionPicker {
        entries, loading, ..
    }) = expect_agent(&app, AgentId(0)).active_modal.as_ref()
    else {
        panic!("expected agent A's SessionPicker modal");
    };
    assert!(entries.is_none());
    assert!(
        loading,
        "the background modal stays loading: its result is dropped, not delivered"
    );
}
/// A background modal's in-flight card detail survives welcome-picker activity.
/// Only the routed surface's detail seq moves, so the detail still applies when the user returns and its own host, generation, and seq match.
/// The modal's OWN list changes still invalidate its in-flight details.
#[test]
fn background_modal_card_detail_survives_welcome_refetch() {
    use crate::views::modal::ActiveModal;
    let mut app = test_app_with_agent();
    assert!(!app.chat_mode);
    let modal_effects = dispatch(Action::ShowSessionPicker, &mut app);
    let modal_generation = modal_picker_generation(&app);
    let [Effect::FetchSessionList { seq: modal_seq, .. }] = modal_effects.as_slice() else {
        panic!("expected modal fetch, got {modal_effects:?}");
    };
    let modal_seq = *modal_seq;
    let modal_list = |generation, seq| {
        Action::TaskComplete(TaskResult::SessionListLoaded {
            host: SessionPickerHost::AgentModal,
            generation,
            scope: ListScope::Cwd,
            sessions: vec![
                make_picker_entry("card-target", "/r"),
                make_picker_entry("card-other", "/r"),
            ],
            partial: None,
            seq,
            query: None,
        })
    };
    let _ = dispatch(modal_list(modal_generation, modal_seq), &mut app);
    let effects = dispatch(
        Action::ExpandSessionCard {
            source: "local".into(),
            session_id: "card-target".into(),
        },
        &mut app,
    );
    let [
        Effect::LoadCardDetail {
            host: SessionPickerHost::AgentModal,
            generation,
            session_id,
            seq,
            ..
        },
    ] = effects.as_slice()
    else {
        panic!("expected a modal-stamped card detail load, got {effects:?}");
    };
    assert_eq!(*generation, modal_generation);
    assert_eq!(session_id, "card-target");
    let detail_seq = *seq;
    app.active_view = ActiveView::Welcome;
    let welcome_effects = dispatch(Action::FetchSessionList, &mut app);
    let [
        Effect::FetchSessionList {
            seq: welcome_seq, ..
        },
    ] = welcome_effects.as_slice()
    else {
        panic!("expected welcome fetch, got {welcome_effects:?}");
    };
    let welcome_seq = *welcome_seq;
    let welcome_detail_seq = app.session_picker_detail_seq;
    let _ = dispatch(
        Action::TaskComplete(TaskResult::SessionListLoaded {
            host: SessionPickerHost::Welcome,
            generation: app.session_picker_generation,
            scope: ListScope::Cwd,
            sessions: vec![make_picker_entry("welcome-row", "/r")],
            partial: None,
            seq: welcome_seq,
            query: None,
        }),
        &mut app,
    );
    assert!(
        app.session_picker_detail_seq > welcome_detail_seq,
        "the welcome result advances the welcome detail seq"
    );
    {
        let Some(&ActiveModal::SessionPicker {
            detail_seq: modal_detail_seq,
            generation: live_modal_generation,
            ..
        }) = expect_agent(&app, AgentId(0)).active_modal.as_ref()
        else {
            panic!("agent A's modal must survive the view switch");
        };
        assert_eq!(
            modal_detail_seq, detail_seq,
            "welcome activity must not advance the modal's detail seq"
        );
        assert_eq!(live_modal_generation, modal_generation);
    }
    app.active_view = ActiveView::Agent(AgentId(0));
    let detail = crate::app::app_view::CardDetail {
        turn_count: 4,
        tool_call_count: 2,
        first_prompt_preview: "first".into(),
    };
    let _ = dispatch(
        Action::TaskComplete(TaskResult::CardDetailLoaded {
            host: SessionPickerHost::AgentModal,
            generation: modal_generation,
            source: "local".into(),
            session_id: "card-target".into(),
            seq: detail_seq,
            detail: detail.clone(),
        }),
        &mut app,
    );
    let modal_card_detail = |app: &AppView, id: &str| {
        let Some(ActiveModal::SessionPicker {
            entries: Some(entries),
            ..
        }) = expect_agent(app, AgentId(0)).active_modal.as_ref()
        else {
            panic!("expected SessionPicker modal with entries");
        };
        entries
            .iter()
            .find(|e| e.id == id)
            .expect("entry")
            .card_detail
            .as_ref()
            .map(|d| d.turn_count)
    };
    assert_eq!(
        modal_card_detail(&app, "card-target"),
        Some(4),
        "the surviving detail must stamp the background modal's own row"
    );
    let effects = dispatch(
        Action::ExpandSessionCard {
            source: "local".into(),
            session_id: "card-other".into(),
        },
        &mut app,
    );
    let [Effect::LoadCardDetail { seq, .. }] = effects.as_slice() else {
        panic!("expected a card detail load, got {effects:?}");
    };
    let second_detail_seq = *seq;
    let _ = dispatch(
        modal_list(modal_generation, app.session_picker_list_seq),
        &mut app,
    );
    let _ = dispatch(
        Action::TaskComplete(TaskResult::CardDetailLoaded {
            host: SessionPickerHost::AgentModal,
            generation: modal_generation,
            source: "local".into(),
            session_id: "card-other".into(),
            seq: second_detail_seq,
            detail,
        }),
        &mut app,
    );
    assert_eq!(
        modal_card_detail(&app, "card-other"),
        None,
        "a modal-targeted list change must still invalidate the modal's own in-flight detail"
    );
}
