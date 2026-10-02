//! Tests for dashboard dispatchers: attach, overlays, rows, and permissions.
use super::*;
use crate::app::app_view::InputOutcome;
use crate::app::dispatch::queue::maybe_drain_queue;
use crate::app::workspace_test_fixtures::{
    member, new_member, snapshot as workspace_snapshot, temp_store,
};
use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
#[test]
fn workspace_identity_rebind_only_activates_for_live_v2_dashboard_state() {
    let mut app = test_app_with_agent();
    assert!(!super::super::dashboard::WorkspaceIdentityRebind::capture(&app).is_active());
    app.workspace_dashboard_enabled = true;
    assert!(!super::super::dashboard::WorkspaceIdentityRebind::capture(&app).is_active());
    ensure_dashboard_state(&mut app);
    assert!(super::super::dashboard::WorkspaceIdentityRebind::capture(&app).is_active());
}
/// `app` as a dashboard v2 client over `store`, ready for writes.
fn ready_workspace_app(mut app: AppView, store: codel_dashboard_store::WorkspaceStore) -> AppView {
    let snapshot = store.snapshot().unwrap();
    app.workspace_dashboard_enabled = true;
    app.workspace_membership.set_ready_for_test(store, snapshot);
    app
}
#[test]
fn voice_final_appends_to_dashboard_dispatch() {
    let mut app = test_app_with_agent();
    app.active_view = ActiveView::AgentDashboard;
    ensure_dashboard_state(&mut app);
    let dispatch = &mut app.dashboard.as_mut().unwrap().dispatch;
    dispatch.set_text("fix");
    dispatch.set_cursor("fix".len());
    app.voice_state = VoiceState::Stopping {
        target: VoiceTarget::DashboardDispatch,
        partial: Partial::None,
        route: None,
    };
    crate::voice::handle_voice_event(
        &mut app,
        codel_voice::VoiceEvent::UtteranceFinal {
            text: "the build".into(),
        },
    );
    assert_eq!(
        app.dashboard.as_ref().unwrap().dispatch.text(),
        "fix the build"
    );
}
#[test]
fn voice_final_appends_to_peek_reply_when_peek_open() {
    use crate::views::dashboard::DashboardRowId;
    use crate::views::dashboard::peek::{PeekFields, PeekPanelState};
    let mut app = test_app_with_agent();
    app.active_view = ActiveView::AgentDashboard;
    ensure_dashboard_state(&mut app);
    let id = AgentId(0);
    let dash = app.dashboard.as_mut().unwrap();
    dash.peek = Some(PeekPanelState::new(
        DashboardRowId::TopLevel(id),
        PeekFields {
            label: "l".into(),
            time_ago: "1m".into(),
            response_type: "Response".into(),
            last_user_message: None,
            question: None,
            options: vec![],
            request_id: None,
            reject_option: None,
        },
    ));
    dash.peek_reply.set_text("reply");
    dash.peek_reply.set_cursor("reply".len());
    app.voice_state = VoiceState::Stopping {
        target: VoiceTarget::DashboardPeekReply(id),
        partial: Partial::None,
        route: None,
    };
    crate::voice::handle_voice_event(
        &mut app,
        codel_voice::VoiceEvent::UtteranceFinal {
            text: "with voice".into(),
        },
    );
    let dash = app.dashboard.as_ref().unwrap();
    assert_eq!(
        dash.peek_reply.text(),
        "reply with voice",
        "dictation must append to the live peek reply"
    );
    assert_eq!(
        dash.dispatch.text(),
        "",
        "the hidden dispatch box must be untouched while the peek is open"
    );
}
#[test]
fn voice_final_discarded_when_peek_row_changed_after_stop() {
    use crate::views::dashboard::DashboardRowId;
    use crate::views::dashboard::peek::{PeekFields, PeekPanelState};
    let peek_for = |row: DashboardRowId| {
        PeekPanelState::new(
            row,
            PeekFields {
                label: "l".into(),
                time_ago: "1m".into(),
                response_type: "Response".into(),
                last_user_message: None,
                question: None,
                options: vec![],
                request_id: None,
                reject_option: None,
            },
        )
    };
    let mut app = test_app_with_agent();
    app.active_view = ActiveView::AgentDashboard;
    ensure_dashboard_state(&mut app);
    app.voice_state = VoiceState::Stopping {
        target: VoiceTarget::DashboardPeekReply(AgentId(0)),
        partial: Partial::None,
        route: None,
    };
    app.dashboard.as_mut().unwrap().peek = Some(peek_for(DashboardRowId::TopLevel(AgentId(1))));
    crate::voice::handle_voice_event(
        &mut app,
        codel_voice::VoiceEvent::UtteranceFinal {
            text: "late words".into(),
        },
    );
    assert_eq!(
        app.dashboard.as_ref().unwrap().peek_reply.text(),
        "",
        "a final for a no-longer-peeked row must be discarded"
    );
}
#[serial_test::serial(CODEL_AGENT_DASHBOARD)]
#[test]
fn voice_dashboard_peek_reply_submit_tears_down_voice() {
    let mut app = test_app_with_agent();
    open_dashboard(&mut app);
    let (tx, mut rx) = tokio::sync::mpsc::channel(8);
    app.voice_mode_enabled = true;
    app.voice_cmd_tx = Some(tx);
    app.voice_state = VoiceState::Recording {
        hold: false,
        target: VoiceTarget::DashboardPeekReply(AgentId(0)),
        partial: Partial::None,
        route: None,
    };
    let _ = dispatch_dashboard_peek_reply(
        &mut app,
        crate::views::dashboard::DashboardRowId::TopLevel(AgentId(0)),
        "ship it".into(),
        false,
    );
    assert!(!app.voice_listening(), "peek reply submit stops capture");
    assert!(
        app.voice_recording_target().is_none(),
        "submit drops the target"
    );
    assert!(matches!(
        rx.try_recv(),
        Ok(codel_voice::VoiceCommand::Abort)
    ));
}
#[test]
fn voice_target_bound_at_start_dispatch_vs_peek() {
    if !codel_voice::AUDIO_SUPPORTED {
        return;
    }
    use crate::views::dashboard::DashboardRowId;
    use crate::views::dashboard::peek::{PeekFields, PeekPanelState};
    let mut app = test_app_with_agent();
    app.active_view = ActiveView::AgentDashboard;
    ensure_dashboard_state(&mut app);
    let (tx, _rx) = tokio::sync::mpsc::channel(8);
    app.voice_mode_enabled = true;
    app.voice_cmd_tx = Some(tx);
    dispatch(Action::EnableVoiceMode, &mut app);
    assert_eq!(
        app.voice_recording_target(),
        Some(VoiceTarget::DashboardDispatch)
    );
    app.voice_state = VoiceState::Idle;
    app.dashboard.as_mut().unwrap().peek = Some(PeekPanelState::new(
        DashboardRowId::TopLevel(AgentId(0)),
        PeekFields {
            label: "l".into(),
            time_ago: "1m".into(),
            response_type: "Response".into(),
            last_user_message: None,
            question: None,
            options: vec![],
            request_id: None,
            reject_option: None,
        },
    ));
    let (tx, _rx) = tokio::sync::mpsc::channel(8);
    app.voice_cmd_tx = Some(tx);
    dispatch(Action::EnableVoiceMode, &mut app);
    assert_eq!(
        app.voice_recording_target(),
        Some(VoiceTarget::DashboardPeekReply(AgentId(0)))
    );
}
/// Changing the peeked dashboard row while dictating into a peek reply stops
/// capture, so a final can't land on the newly-selected agent's reply.
#[test]
fn voice_auto_stops_when_peek_row_changes() {
    use crate::views::dashboard::DashboardRowId;
    use crate::views::dashboard::peek::{PeekFields, PeekPanelState};
    let peek_for = |row: DashboardRowId| {
        PeekPanelState::new(
            row,
            PeekFields {
                label: "l".into(),
                time_ago: "1m".into(),
                response_type: "Response".into(),
                last_user_message: None,
                question: None,
                options: vec![],
                request_id: None,
                reject_option: None,
            },
        )
    };
    let mut app = test_app_with_agent();
    app.active_view = ActiveView::AgentDashboard;
    ensure_dashboard_state(&mut app);
    let (tx, _rx) = tokio::sync::mpsc::channel(8);
    app.voice_cmd_tx = Some(tx);
    app.voice_state = VoiceState::Recording {
        hold: false,
        target: VoiceTarget::DashboardPeekReply(AgentId(0)),
        partial: Partial::None,
        route: None,
    };
    app.dashboard.as_mut().unwrap().peek = Some(peek_for(DashboardRowId::TopLevel(AgentId(0))));
    app.enforce_voice_session_bound();
    assert!(app.voice_listening());
    app.dashboard.as_mut().unwrap().peek = Some(peek_for(DashboardRowId::TopLevel(AgentId(1))));
    app.enforce_voice_session_bound();
    assert!(!app.voice_listening(), "row change must stop capture");
    assert!(app.voice_recording_target().is_none());
}
/// Opening the attached-agent popup hides the dashboard inputs, so dictation
/// must not bind there: starting is a no-op and an active capture auto-stops.
#[test]
fn voice_suppressed_while_dashboard_popup_open() {
    if !codel_voice::AUDIO_SUPPORTED {
        return;
    }
    let mut app = test_app_with_agent();
    app.active_view = ActiveView::AgentDashboard;
    ensure_dashboard_state(&mut app);
    app.dashboard.as_mut().unwrap().attached_agent = Some(AgentId(0));
    let (tx, _rx) = tokio::sync::mpsc::channel(8);
    app.voice_mode_enabled = true;
    app.voice_cmd_tx = Some(tx);
    dispatch(Action::EnableVoiceMode, &mut app);
    assert!(!app.voice_listening());
    assert!(app.voice_recording_target().is_none());
    app.voice_state = VoiceState::Recording {
        hold: false,
        target: VoiceTarget::DashboardDispatch,
        partial: Partial::None,
        route: None,
    };
    app.enforce_voice_session_bound();
    assert!(
        !app.voice_listening(),
        "popup must stop dashboard dictation"
    );
    assert!(app.voice_recording_target().is_none());
}
#[test]
fn voice_off_target_surface_does_not_enable_or_record() {
    let mut app = test_app_with_agent();
    app.active_view = ActiveView::AgentDashboard;
    ensure_dashboard_state(&mut app);
    app.dashboard.as_mut().unwrap().attached_agent = Some(AgentId(0));
    let (tx, mut rx) = tokio::sync::mpsc::channel(8);
    app.voice_mode_enabled = true;
    app.voice_cmd_tx = Some(tx);
    let agents_before = app.agents.len();
    dispatch(Action::EnableVoiceMode, &mut app);
    assert_eq!(
        app.agents.len(),
        agents_before,
        "no session spawned off-target"
    );
    assert!(!app.voice_ui_active, "voice mode must not arm off-target");
    assert!(!app.voice_listening());
    assert!(!app.voice_state.is_pending_cold_start());
    assert!(rx.try_recv().is_err(), "no PttPress without a target");
}
#[test]
fn resolve_location_input_expands_and_joins() {
    let cwd = PathBuf::from("/work/dir");
    assert_eq!(
        resolve_location_input("/abs/path", &cwd),
        Some(PathBuf::from("/abs/path"))
    );
    assert_eq!(
        resolve_location_input("sub/dir", &cwd),
        Some(PathBuf::from("/work/dir/sub/dir"))
    );
    let home = codel_dirs::home_dir().expect("home dir");
    assert_eq!(resolve_location_input("~", &cwd), Some(home.clone()));
    assert_eq!(resolve_location_input("~/x", &cwd), Some(home.join("x")));
    assert_eq!(resolve_location_input("   ", &cwd), None);
}
/// Confirming the worktree dialog with `attach` set (Ctrl+S / the new-agent button)
/// creates a worktree session, replays the stashed prompt, and opens the new agent
/// as the dashboard's detail view.
#[test]
fn dashboard_confirm_worktree_creates_session_with_prompt() {
    let mut app = test_app_with_agent();
    app.cwd_has_git_ancestor = true;
    app.dashboard = Some(crate::views::dashboard::DashboardState::new());
    if let Some(d) = app.dashboard.as_mut() {
        d.dispatch.set_text("hello wt");
        d.pending_worktree_prompt = Some(d.dispatch.stash());
        d.pending_worktree_attach = true;
    }
    let effects = dispatch_dashboard_confirm_worktree(&mut app, Some("my-wt".into()));
    let wt_id = effects
        .iter()
        .find_map(|e| match e {
            Effect::CreateWorktreeSession {
                agent_id, label, ..
            } => {
                assert_eq!(label.as_deref(), Some("my-wt"));
                Some(*agent_id)
            }
            _ => None,
        })
        .expect("expected a CreateWorktreeSession effect");
    assert_eq!(
        test_agent(&app, wt_id).session.queue_len(),
        1,
        "the stashed prompt must be enqueued on the worktree agent",
    );
    assert!(
        app.dashboard
            .as_ref()
            .unwrap()
            .pending_worktree_prompt
            .is_none(),
        "the stash must be consumed",
    );
    assert_eq!(
        app.dashboard.as_ref().unwrap().attached_agent,
        Some(wt_id),
        "the worktree agent must attach to the dashboard overlay",
    );
    assert!(
        matches!(app.active_view, ActiveView::Agent(id) if id == wt_id),
        "the active view must be the new worktree agent",
    );
}
/// Confirming the worktree dialog from a plain `Enter` prompt-send
/// (`attach == false`) creates the worktree session and replays the prompt
/// but STAYS on the dashboard: no detail view, no overlay attach.
#[test]
fn dashboard_confirm_worktree_without_attach_stays_on_dashboard() {
    let mut app = test_app_with_agent();
    app.cwd_has_git_ancestor = true;
    app.active_view = ActiveView::AgentDashboard;
    app.dashboard = Some(crate::views::dashboard::DashboardState::new());
    if let Some(d) = app.dashboard.as_mut() {
        d.dispatch.set_text("hello wt");
        d.pending_worktree_prompt = Some(d.dispatch.stash());
        d.pending_worktree_attach = false;
    }
    let effects = dispatch_dashboard_confirm_worktree(&mut app, Some("my-wt".into()));
    let wt_id = effects
        .iter()
        .find_map(|e| match e {
            Effect::CreateWorktreeSession { agent_id, .. } => Some(*agent_id),
            _ => None,
        })
        .expect("expected a CreateWorktreeSession effect");
    assert_eq!(
        test_agent(&app, wt_id).session.queue_len(),
        1,
        "the stashed prompt must still be enqueued on the worktree agent",
    );
    assert_eq!(
        app.dashboard.as_ref().unwrap().attached_agent,
        None,
        "a plain prompt-send must not attach the worktree agent to the overlay",
    );
    assert!(
        matches!(app.active_view, ActiveView::AgentDashboard),
        "a plain prompt-send must stay on the dashboard, got {:?}",
        app.active_view,
    );
}
/// Confirming the worktree dialog outside a git repo creates nothing and surfaces a dashboard error instead.
/// surfaces a dashboard error instead.
#[test]
fn dashboard_confirm_worktree_without_git_repo_creates_nothing() {
    let mut app = test_app_with_agent();
    app.cwd_has_git_ancestor = false;
    app.dashboard = Some(crate::views::dashboard::DashboardState::new());
    {
        let dashboard = app.dashboard.as_mut().unwrap();
        dashboard.dispatch.set_text("fix the bug");
        dashboard.pending_worktree_prompt = Some(dashboard.dispatch.stash());
    }
    let before = app.agents.len();
    let effects = dispatch_dashboard_confirm_worktree(&mut app, None);
    assert_eq!(app.agents.len(), before, "no agent without a git repo");
    assert!(
        !effects
            .iter()
            .any(|e| matches!(e, Effect::CreateWorktreeSession { .. })),
        "no worktree session without a git repo",
    );
    let d = app.dashboard.as_ref().unwrap();
    assert_eq!(
        d.dispatch.text(),
        "fix the bug",
        "the stashed prompt must be restored to the dispatch input",
    );
    assert!(
        d.pending_worktree_prompt.is_none(),
        "the stash must be consumed",
    );
}
/// Images pasted into the dispatch input survive a worktree dispatch:
/// stashed when the dialog opens, replayed onto the worktree agent's queued prompt on confirm. Regression: the worktree branch dropped them while the normal dispatch path carried them.
#[test]
fn dashboard_confirm_worktree_replays_pasted_images() {
    let mut app = test_app_with_agent();
    app.cwd_has_git_ancestor = true;
    app.dashboard = Some(crate::views::dashboard::DashboardState::new());
    let img = crate::prompt_images::from_clipboard_data(&crate::clipboard::ImageData {
        data: vec![1, 2, 3],
        mime_type: "image/png".into(),
    });
    if let Some(d) = app.dashboard.as_mut() {
        d.dispatch.set_text("look at this ");
        d.dispatch.insert_image(img).unwrap();
        d.pending_worktree_prompt = Some(d.dispatch.stash());
    }
    let effects = dispatch_dashboard_confirm_worktree(&mut app, Some("my-wt".into()));
    let wt_id = effects
        .iter()
        .find_map(|e| match e {
            Effect::CreateWorktreeSession { agent_id, .. } => Some(*agent_id),
            _ => None,
        })
        .expect("expected a CreateWorktreeSession effect");
    let entry = test_agent(&app, wt_id)
        .session
        .pending_prompts
        .back()
        .expect("the replayed prompt must be queued");
    assert_eq!(
        entry.images.len(),
        1,
        "pasted images must travel to the worktree agent",
    );
    assert!(entry.text.contains("[Image #1]"));
    assert_eq!(
        entry
            .chip_elements
            .iter()
            .filter(|element| element.kind == crate::views::prompt_widget::KIND_IMAGE)
            .count(),
        1,
        "worktree replay must preserve the image chip element"
    );
    assert!(
        app.dashboard
            .as_ref()
            .unwrap()
            .pending_worktree_prompt
            .is_none(),
        "the image-bearing prompt stash must be consumed",
    );
}
#[serial_test::serial(CODEL_AGENT_DASHBOARD)]
#[test]
fn dashboard_image_dispatch_cancel_rewind_resends_attachment() {
    let mut app = test_app_with_agent();
    app.dashboard = Some(crate::views::dashboard::DashboardState::new());
    let text = {
        let dashboard = app.dashboard.as_mut().unwrap();
        dashboard.dispatch.set_text("inspect ");
        dashboard
            .dispatch
            .insert_image(crate::prompt_images::from_clipboard_data(
                &crate::clipboard::ImageData {
                    data: vec![1, 2, 3],
                    mime_type: "image/png".into(),
                },
            ))
            .unwrap();
        dashboard.dispatch.text().to_owned()
    };
    let create = dispatch_dashboard_dispatch(&mut app, text, true);
    let new_id = create
        .iter()
        .find_map(|effect| match effect {
            Effect::CreateSession { agent_id, .. } => Some(*agent_id),
            _ => None,
        })
        .unwrap();
    {
        let agent = app.agents.get_mut(&new_id).unwrap();
        let queued = agent.session.pending_prompts.front().unwrap();
        assert_eq!(queued.images.len(), 1);
        assert_eq!(queued.chip_elements.len(), 1);
        agent.session.session_id = Some(acp::SessionId::new("dashboard-image"));
        agent.session.state = AgentState::Idle;
        assert!(matches!(
            maybe_drain_queue(agent, &mut app.pending_image_notices)
                .effects
                .as_slice(),
            [Effect::SendPromptBlocks { .. }]
        ));
    }
    app.active_view = ActiveView::Agent(new_id);
    let _ = dispatch(Action::CancelTurn, &mut app);
    let agent = app.agents.get(&new_id).unwrap();
    assert_eq!(agent.prompt.images.len(), 1);
    assert_eq!(
        agent
            .prompt
            .textarea()
            .elements()
            .iter()
            .filter(|element| element.kind == crate::views::prompt_widget::KIND_IMAGE)
            .count(),
        1
    );
    let restored = agent.prompt.text().to_owned();
    let resend = dispatch(Action::SendPrompt(restored), &mut app);
    assert!(matches!(
        resend.as_slice(),
        [Effect::SendPromptBlocks { .. }]
    ));
    assert_eq!(
        test_agent(&app, new_id)
            .session
            .in_flight_prompt
            .as_ref()
            .unwrap()
            .images
            .len(),
        1
    );
}
/// Paste-then-immediate-send race (dashboard dispatch): the same guarantee
/// for the dashboard's session-spawning input: the new session must carry
/// the pasted image even when Enter beats the deferred probe.
#[test]
fn dashboard_dispatch_send_before_paste_probe_keeps_image() {
    use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
    let mut app = test_app_with_agent();
    open_dashboard(&mut app);
    crate::clipboard::set_clipboard_probe_hook(crate::clipboard::ClipboardProbeHook::with_raster(
        None,
    ));
    {
        let reg = crate::actions::ActionRegistry::defaults();
        let d = app.dashboard.as_mut().unwrap();
        d.dispatch.set_text("look at this");
        let _ = d.handle_input(
            &Event::Key(KeyEvent::new(KeyCode::Char('v'), KeyModifiers::CONTROL)),
            &reg,
        );
    }
    let ctx = app
        .dashboard
        .as_ref()
        .unwrap()
        .pending_effects
        .iter()
        .find_map(|e| match e {
            Effect::ProbeClipboardAttachment { ctx, .. } => Some(ctx.clone()),
            _ => None,
        })
        .expect("Cmd+V of an image must defer a probe");
    crate::clipboard::clear_clipboard_probe_hook();
    assert_eq!(app.dashboard.as_ref().unwrap().paste_probe_in_flight, 1);
    let before = app.agents.len();
    let effects = dispatch(
        Action::DashboardDispatch {
            text: "look at this".into(),
            attach: false,
        },
        &mut app,
    );
    assert!(
        effects.is_empty(),
        "the dispatch must be stashed while the probe is in flight"
    );
    assert!(
        app.dashboard
            .as_ref()
            .unwrap()
            .deferred_dispatch_send
            .is_some()
    );
    assert_eq!(
        app.agents.len(),
        before,
        "no new session before the image attaches"
    );
    let pasted = crate::prompt_images::from_clipboard_data(&crate::clipboard::ImageData {
        data: vec![1, 2, 3],
        mime_type: "image/png".into(),
    });
    let _ = dispatch(
        Action::TaskComplete(TaskResult::ClipboardAttachmentProbed {
            ctx,
            image: crate::app::actions::ProbedAttachment::Image(pasted),
            file_urls: None,
        }),
        &mut app,
    );
    assert_eq!(app.dashboard.as_ref().unwrap().paste_probe_in_flight, 0);
    assert!(
        app.dashboard
            .as_ref()
            .unwrap()
            .deferred_dispatch_send
            .is_none(),
        "the stashed dispatch was consumed"
    );
    assert_eq!(
        app.agents.len(),
        before + 1,
        "the re-issued dispatch created the session"
    );
    let entry = test_agent(&app, AgentId(1))
        .session
        .pending_prompts
        .back()
        .expect("the new session has a queued prompt");
    assert_eq!(
        entry.images.len(),
        1,
        "the dispatched prompt carries the pasted image"
    );
}
/// Per-surface stashes: a dispatch send AND a peek reply stashed during the same probe window must both survive (the old single slot let the second stash silently overwrite the first) and both re-issue on completion:
/// dispatch first, then peek.
#[test]
fn dashboard_second_stash_does_not_overwrite_first() {
    use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
    let mut app = test_app_with_agent();
    open_dashboard(&mut app);
    let row = crate::views::dashboard::DashboardRowId::TopLevel(AgentId(0));
    let fields = crate::views::dashboard::peek::compute_peek_fields(&row, &app.agents)
        .expect("agent must be peekable");
    if let Some(d) = app.dashboard.as_mut() {
        d.focus_row(row.clone());
        d.peek = Some(crate::views::dashboard::peek::PeekPanelState::new(
            row.clone(),
            fields,
        ));
        d.dispatch.set_text("spawn a fixer");
        d.peek_reply.set_text("please look");
    }
    crate::clipboard::set_clipboard_probe_hook(crate::clipboard::ClipboardProbeHook::with_raster(
        None,
    ));
    {
        let reg = crate::actions::ActionRegistry::defaults();
        let d = app.dashboard.as_mut().unwrap();
        let _ = d.handle_input(
            &Event::Key(KeyEvent::new(KeyCode::Char('v'), KeyModifiers::CONTROL)),
            &reg,
        );
    }
    let ctx = app
        .dashboard
        .as_ref()
        .unwrap()
        .pending_effects
        .iter()
        .find_map(|e| match e {
            Effect::ProbeClipboardAttachment { ctx, .. } => Some(ctx.clone()),
            _ => None,
        })
        .expect("Cmd+V of an image must defer a probe");
    crate::clipboard::clear_clipboard_probe_hook();
    let effects = dispatch(
        Action::DashboardDispatch {
            text: "spawn a fixer".into(),
            attach: false,
        },
        &mut app,
    );
    assert!(effects.is_empty());
    let effects = dispatch(
        Action::DashboardPeekReply {
            row: row.clone(),
            text: "please look".into(),
            attach: false,
        },
        &mut app,
    );
    assert!(effects.is_empty());
    {
        let d = app.dashboard.as_ref().unwrap();
        assert!(
            d.deferred_dispatch_send.is_some() && d.deferred_peek_send.is_some(),
            "each surface keeps its own stash"
        );
    }
    let before = app.agents.len();
    let pasted = crate::prompt_images::from_clipboard_data(&crate::clipboard::ImageData {
        data: vec![1, 2, 3],
        mime_type: "image/png".into(),
    });
    let effects = dispatch(
        Action::TaskComplete(TaskResult::ClipboardAttachmentProbed {
            ctx,
            image: crate::app::actions::ProbedAttachment::Image(pasted),
            file_urls: None,
        }),
        &mut app,
    );
    let d = app.dashboard.as_ref().unwrap();
    assert!(d.deferred_dispatch_send.is_none() && d.deferred_peek_send.is_none());
    assert_eq!(
        app.agents.len(),
        before + 1,
        "the stashed dispatch spawned its session"
    );
    let reply_sent = effects.iter().any(|e| {
        matches!(
            e,
            Effect::SendPromptBlocks { agent_id, blocks, .. }
                if *agent_id == AgentId(0)
                    && blocks.iter().any(|b| matches!(b, acp::ContentBlock::Image(_)))
        )
    });
    assert!(
        reply_sent,
        "the stashed peek reply must reach the peeked agent with the image; effects = {effects:?}"
    );
}
/// The staged-mode write sits outside `set_yolo_mode_inner`; its own
/// backstop must downgrade Always-Approve under the pin and stay the
/// identity without one.
#[test]
fn apply_pending_dispatch_config_always_approve_blocked_by_policy_pin() {
    use crate::views::dashboard::DashboardDispatchMode;
    let mut app = test_app_with_agent();
    let agent = app.agents.get_mut(&AgentId(0)).unwrap();
    apply_pending_dispatch_config(
        agent,
        None,
        DashboardDispatchMode::AlwaysApprove,
        Some(POLICY_WARNING),
    );
    assert!(
        !agent.session.is_yolo(),
        "staged Always-Approve must not enable yolo under the pin"
    );
    assert_eq!(
        agent.toast.as_ref().map(|(s, _)| s.as_str()),
        Some(POLICY_WARNING),
    );
    apply_pending_dispatch_config(agent, None, DashboardDispatchMode::AlwaysApprove, None);
    assert!(agent.session.is_yolo());
    assert!(!agent.session.is_auto());
}
#[test]
fn apply_pending_dispatch_config_auto_sets_classifier_mode() {
    use crate::views::dashboard::DashboardDispatchMode;
    let mut app = test_app_with_agent();
    let agent = app.agents.get_mut(&AgentId(0)).unwrap();
    agent.session.yolo_mode = true;
    apply_pending_dispatch_config(agent, None, DashboardDispatchMode::Auto, None);
    assert!(agent.session.is_auto());
    assert!(!agent.session.is_yolo());
}
/// Dashboard per-agent toggle under the pin: refused, warning lands on the dashboard's OWN error slot (the user is looking at the dashboard, not the agent). OFF stays allowed.
/// the dashboard's OWN error slot (the user is looking at the
/// dashboard, not the agent). OFF stays allowed.
#[serial_test::serial(CODEL_AGENT_DASHBOARD)]
#[test]
fn dashboard_toggle_auto_approve_blocked_by_policy_pin() {
    let mut app = test_app_with_agent();
    let id = AgentId(0);
    app.yolo_policy_block = Some(POLICY_WARNING);
    app.active_view = ActiveView::Welcome;
    open_dashboard(&mut app);
    if let Some(d) = app.dashboard.as_mut() {
        d.focus_row(crate::views::dashboard::DashboardRowId::TopLevel(id));
    }
    let effects = dispatch_dashboard_toggle_auto_approve(&mut app);
    assert!(effects.is_empty(), "blocked toggle must not emit effects");
    assert!(
        !app.agents.get(&id).unwrap().session.is_yolo(),
        "toggle ON must be refused under the pin"
    );
    assert_eq!(
        app.dashboard.as_ref().unwrap().error_toast.as_deref(),
        Some(format!("{} {POLICY_WARNING}", crate::glyphs::ballot_x()).as_str()),
        "warning must land on the dashboard error slot",
    );
    app.agents.get_mut(&id).unwrap().session.yolo_mode = true;
    let _ = dispatch_dashboard_toggle_auto_approve(&mut app);
    assert!(
        !app.agents.get(&id).unwrap().session.is_yolo(),
        "toggle OFF must still work under the pin"
    );
}
/// Shift+Tab in the peek cycles the PEEKED agent's live mode
/// (Normal to Plan) and leaves the dashboard foregrounded, the same
/// effect as Shift+Tab inside that agent's chat view.
#[serial_test::serial(CODEL_AGENT_DASHBOARD)]
#[test]
fn dashboard_peek_cycle_mode_cycles_peeked_agent() {
    let mut app = test_app_with_agent();
    open_dashboard(&mut app);
    let row = crate::views::dashboard::DashboardRowId::TopLevel(AgentId(0));
    let fields = crate::views::dashboard::peek::compute_peek_fields(&row, &app.agents)
        .expect("agent must be peekable");
    if let Some(d) = app.dashboard.as_mut() {
        d.focus_row(row.clone());
        d.peek = Some(crate::views::dashboard::peek::PeekPanelState::new(
            row.clone(),
            fields,
        ));
    }
    assert_eq!(app.agents.get(&AgentId(0)).unwrap().plan_mode_pending, None);
    assert!(!app.agents.get(&AgentId(0)).unwrap().session.yolo_mode);
    let _ = dispatch(Action::DashboardPeekCycleMode, &mut app);
    assert_eq!(
        app.agents.get(&AgentId(0)).unwrap().plan_mode_pending,
        Some(true),
        "peek cycle must put the peeked agent into plan mode",
    );
    assert!(
        matches!(app.active_view, ActiveView::AgentDashboard),
        "peek cycle must not switch away from the dashboard, got {:?}",
        app.active_view,
    );
}
/// A dashboard-peek Shift+Tab cycles the peeked agent into plan mode but must
/// NOT attribute a plan-nudge acceptance: the user is on the dashboard, not that agent's prompt, so the nudge (still within TTL) is left intact. This pins that the peek routes through the telemetry-free cycle body.
#[serial_test::serial(CODEL_AGENT_DASHBOARD)]
#[test]
fn dashboard_peek_cycle_does_not_retire_the_nudge() {
    let mut app = test_app_with_agent();
    open_dashboard(&mut app);
    let row = crate::views::dashboard::DashboardRowId::TopLevel(AgentId(0));
    let fields = crate::views::dashboard::peek::compute_peek_fields(&row, &app.agents)
        .expect("agent must be peekable");
    if let Some(d) = app.dashboard.as_mut() {
        d.focus_row(row.clone());
        d.peek = Some(crate::views::dashboard::peek::PeekPanelState::new(
            row.clone(),
            fields,
        ));
    }
    let _ = app.agents.get_mut(&AgentId(0)).unwrap().ephemeral_tip.show(
        crate::tips::plan_nudge::plan_nudge_tip(),
        &mut std::collections::HashMap::new(),
    );
    let _ = dispatch(Action::DashboardPeekCycleMode, &mut app);
    assert_eq!(
        app.agents.get(&AgentId(0)).unwrap().plan_mode_pending,
        Some(true),
        "peek cycle must still put the peeked agent into plan mode",
    );
    assert_eq!(
        app.agents
            .get(&AgentId(0))
            .unwrap()
            .ephemeral_tip
            .current_key(),
        Some(crate::tips::plan_nudge::PLAN_NUDGE_KEY),
        "the dashboard peek must not retire (or attribute) the nudge",
    );
}
#[test]
fn dashboard_open_or_merges_session_is_worktree_when_probe_is_plain() {
    let repo = crate::test_util::TempGitRepo::init("main");
    let mut app = test_app_with_agent();
    let id = AgentId(0);
    {
        let agent = app.agents.get_mut(&id).unwrap();
        agent.session.cwd = repo.path.clone();
        agent.session.is_worktree = true;
        agent.is_worktree = false;
        agent.current_branch = Some("stale".into());
    }
    app.active_view = ActiveView::Agent(id);
    let _ = dispatch_open_dashboard(&mut app);
    let agent = test_agent(&app, id);
    assert!(
        agent.is_worktree,
        "session.is_worktree must not be clobbered by a plain-repo probe"
    );
    assert_eq!(agent.current_branch.as_deref(), Some("main"));
}
#[test]
fn dashboard_open_clears_stale_agent_is_worktree_when_probe_and_session_false() {
    let repo = crate::test_util::TempGitRepo::init("main");
    let mut app = test_app_with_agent();
    let id = AgentId(0);
    {
        let agent = app.agents.get_mut(&id).unwrap();
        agent.session.cwd = repo.path.clone();
        agent.session.is_worktree = false;
        agent.is_worktree = true;
        agent.current_branch = Some("stale".into());
    }
    app.active_view = ActiveView::Agent(id);
    let _ = dispatch_open_dashboard(&mut app);
    let agent = test_agent(&app, id);
    assert!(
        !agent.is_worktree,
        "stale agent.is_worktree must clear when probe and session are false"
    );
    assert_eq!(agent.current_branch.as_deref(), Some("main"));
}
#[test]
fn dashboard_open_detects_standalone_codel_worktree() {
    let main = crate::test_util::TempGitRepo::init("main-only");
    let clone = main.standalone_clone("wt-branch");
    let mut app = test_app_with_agent();
    let id = AgentId(0);
    {
        let agent = app.agents.get_mut(&id).unwrap();
        agent.session.cwd = clone.path.clone();
        agent.session.is_worktree = false;
        agent.is_worktree = false;
        agent.current_branch = Some("main-random".into());
        agent.main_repo = None;
    }
    app.active_view = ActiveView::Agent(id);
    let _ = dispatch_open_dashboard(&mut app);
    let agent = test_agent(&app, id);
    assert!(agent.is_worktree);
    assert_eq!(agent.current_branch.as_deref(), Some("wt-branch"));
    assert_eq!(
        agent.main_repo.as_deref(),
        Some(crate::test_util::collapsed_path_display(&main.path).as_str())
    );
}
/// Leader-mode independence: opening the dashboard works even when NOT in leader mode. The dashboard renders local sessions regardless; leader mode only adds the roster poll. Every entry point funnels through
/// `Action::OpenDashboard`, so this covers `/dashboard`, `Ctrl+\`, `codel dashboard`, and the startup hook.
#[serial_test::serial(CODEL_AGENT_DASHBOARD)]
#[test]
fn dashboard_open_works_without_leader() {
    let mut app = test_app_with_agent();
    app.leader_mode = false;
    app.active_view = ActiveView::Agent(AgentId(0));
    let _ = dispatch_open_dashboard(&mut app);
    assert!(
        matches!(app.active_view, ActiveView::AgentDashboard),
        "dashboard must open outside leader mode",
    );
    assert!(
        app.dashboard.is_some(),
        "dashboard state should be created outside leader mode",
    );
}
/// Build a dormant roster entry for the local idle-session tests.
fn idle_roster_entry(session_id: &str, title: &str) -> crate::app::roster::RosterEntry {
    crate::app::roster::RosterEntry {
        session_id: session_id.to_string(),
        title: Some(title.to_string()),
        cwd: "/repo".to_string(),
        is_worktree: false,
        session_kind: None,
        model_id: None,
        yolo: false,
        activity: crate::app::roster::RosterActivity::Dormant,
        last_turn_summary: None,
        resident: false,
        last_change_unix_ms: 1,
        origin: crate::app::roster::RosterOrigin::default(),
    }
}
/// Without a leader there is no live roster to poll, so opening the dashboard must kick off a fetch of the local on-disk idle sessions so the view isn't empty.
/// dashboard must kick off a fetch of the local on-disk idle sessions so
/// the view isn't empty.
#[serial_test::serial(CODEL_AGENT_DASHBOARD)]
#[test]
fn dashboard_open_without_leader_fetches_local_sessions() {
    let mut app = test_app_with_agent();
    app.leader_mode = false;
    app.active_view = ActiveView::Agent(AgentId(0));
    let effects = dispatch_open_dashboard(&mut app);
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::FetchDashboardSessions)),
        "non-leader dashboard open must fetch the local idle-session list",
    );
}
#[test]
fn workspace_sync_request_before_dashboard_does_not_activate_store() {
    let mut app = test_app_with_agent();
    app.workspace_dashboard_enabled = true;
    crate::app::workspace_sync::request(&mut app);
    assert!(
        crate::app::workspace_sync::drain(&mut app).is_empty(),
        "ordinary ACP/task activity must not activate workspace persistence"
    );
    assert!(app.workspace_membership.snapshot().is_none());
}
#[test]
fn workspace_dashboard_open_loads_one_snapshot_and_skips_rosters() {
    let mut app = test_app_with_agent();
    app.workspace_dashboard_enabled = true;
    app.active_view = ActiveView::Agent(AgentId(0));
    let effects = dispatch_open_dashboard(&mut app);
    assert!(matches!(
        effects.as_slice(),
        [Effect::LoadWorkspaceSnapshot { .. }]
    ));
    assert!(app.dashboard_sessions_loading);
    assert!(
        !effects
            .iter()
            .any(|effect| matches!(effect, Effect::FetchRoster | Effect::FetchDashboardSessions))
    );
    let _ = dispatch_exit_dashboard(&mut app);
    let effects = dispatch_open_dashboard(&mut app);
    assert!(
        effects.is_empty(),
        "an in-flight workspace read must suppress duplicate opens"
    );
}
#[test]
fn workspace_busy_open_retry_stays_loading_and_defers_failure_toast() {
    let mut app = test_app_with_agent();
    app.workspace_dashboard_enabled = true;
    app.active_view = ActiveView::Agent(AgentId(0));
    assert!(matches!(
        dispatch_open_dashboard(&mut app).as_slice(),
        [Effect::LoadWorkspaceSnapshot { .. }]
    ));
    let retry = dispatch(
        Action::TaskComplete(TaskResult::WorkspaceSnapshotFailed {
            error: "busy".into(),
            retryable: true,
        }),
        &mut app,
    );
    assert!(matches!(
        retry.as_slice(),
        [Effect::LoadWorkspaceSnapshot { .. }]
    ));
    assert!(app.dashboard_sessions_loading);
    assert!(app.dashboard.as_ref().unwrap().error_toast.is_none());
    let parked = dispatch(
        Action::TaskComplete(TaskResult::WorkspaceSnapshotFailed {
            error: "still busy".into(),
            retryable: true,
        }),
        &mut app,
    );
    assert!(parked.is_empty());
    assert!(!app.dashboard_sessions_loading);
    assert!(
        app.dashboard
            .as_ref()
            .unwrap()
            .error_toast
            .as_deref()
            .is_some_and(|toast| toast.contains("Could not load dashboard workspace"))
    );
}
#[test]
fn workspace_snapshot_load_requests_live_adoption() {
    let (_temp, store) = temp_store();
    let snapshot = store.snapshot().unwrap();
    let mut app = test_app_with_agent();
    app.workspace_dashboard_enabled = true;
    crate::app::workspace_sync::activate(&mut app);
    let effects = dispatch(
        Action::TaskComplete(TaskResult::WorkspaceSnapshotLoaded { store, snapshot }),
        &mut app,
    );
    assert!(effects.is_empty());
    assert!(matches!(
        crate::app::workspace_sync::drain(&mut app).as_slice(),
        [Effect::WriteWorkspace {
            mutation: WorkspaceMutation::Upsert(members),
            ..
        }] if members.len() == 1
    ));
}
#[test]
fn workspace_grouping_toggle_emits_sqlite_layout_write_not_config_write() {
    let (_temp, store) = temp_store();
    let mut app = ready_workspace_app(test_app_with_agent(), store);
    let mut persisted = crate::views::dashboard::PersistedDashboard::defaults();
    persisted.grouping = crate::views::dashboard::Grouping::Directory;
    app.dashboard_persisted = Some(persisted);
    ensure_dashboard_state(&mut app);
    let dashboard = app.dashboard.as_mut().unwrap();
    dashboard.focus_section(crate::views::dashboard::SectionKey::State(
        crate::views::dashboard::RowState::Idle,
    ));
    dashboard.manual_scroll_active = true;
    let effects = dispatch(Action::DashboardToggleGrouping, &mut app);
    assert!(matches!(
        effects.as_slice(),
        [Effect::WriteWorkspace {
            mutation: WorkspaceMutation::Layout(patch),
            ..
        }] if patch.grouping == Some(codel_dashboard_store::LayoutGrouping::Directory)
    ));
    assert!(
        effects
            .iter()
            .all(|effect| !matches!(effect, Effect::PersistDashboard(_)))
    );
    let dashboard = app.dashboard.as_ref().unwrap();
    assert!(dashboard.new_agent_button_focused());
    assert!(!dashboard.manual_scroll_active);
}
#[test]
fn v1_grouping_toggle_emits_config_write_not_sqlite_layout_write() {
    let mut app = test_app_with_agent();
    app.workspace_dashboard_enabled = false;
    ensure_dashboard_state(&mut app);
    let effects = dispatch(Action::DashboardToggleGrouping, &mut app);
    assert!(matches!(
        effects.as_slice(),
        [Effect::PersistDashboard(persisted)]
            if persisted.grouping == crate::views::dashboard::Grouping::Directory
    ));
    assert!(
        effects
            .iter()
            .all(|effect| !matches!(effect, Effect::WriteWorkspace { .. }))
    );
}
#[test]
fn v2_pin_gesture_updates_overlay_and_emits_only_layout_write() {
    let (_temp, store) = temp_store();
    let snapshot = workspace_snapshot(vec![member("test-session", "Saved")]);
    let mut app = test_app_with_agent();
    app.workspace_dashboard_enabled = true;
    app.workspace_membership.set_ready_for_test(store, snapshot);
    ensure_dashboard_state(&mut app);
    app.dashboard
        .as_mut()
        .unwrap()
        .focus_row(crate::views::dashboard::DashboardRowId::TopLevel(AgentId(
            0,
        )));
    let effects = dispatch(Action::DashboardTogglePin, &mut app);
    assert!(matches!(
        effects.as_slice(),
        [Effect::WriteWorkspace {
            mutation: WorkspaceMutation::Layout(patch),
            ..
        }] if patch.pin_assignments.len() == 1
            && patch.pin_assignments.first().is_some_and(|a| a.pinned)
    ));
    assert!(
        app.workspace_membership
            .view()
            .unwrap()
            .members
            .first()
            .is_some_and(|m| m.pin_rank.is_some())
    );
    assert!(
        effects
            .iter()
            .all(|effect| !matches!(effect, Effect::PersistDashboard(_)))
    );
}
#[test]
fn workspace_refresh_result_does_not_request_an_upsert() {
    let (_temp, store) = temp_store();
    let mut app = ready_workspace_app(test_app_with_agent(), store);
    let Effect::RefreshWorkspace { store, .. } =
        crate::app::workspace_sync::refresh(&mut app).pop().unwrap()
    else {
        panic!("expected refresh effect");
    };
    let effects = dispatch(
        Action::TaskComplete(TaskResult::WorkspaceRefreshed {
            store,
            snapshot: Ok(None),
        }),
        &mut app,
    );
    assert!(effects.is_empty());
    assert!(crate::app::workspace_sync::drain(&mut app).is_empty());
}
#[test]
fn workspace_dashboard_reopens_store_when_only_stale_snapshot_remains() {
    let mut app = test_app_with_agent();
    app.workspace_dashboard_enabled = true;
    app.workspace_membership
        .set_snapshot_for_test(codel_dashboard_store::WorkspaceSnapshot {
            grouping: codel_dashboard_store::Grouping::State,
            members: vec![],
            data_version: 1,
        });
    app.active_view = ActiveView::Agent(AgentId(0));
    let effects = dispatch_open_dashboard(&mut app);
    assert!(matches!(
        effects.as_slice(),
        [Effect::LoadWorkspaceSnapshot { .. }]
    ));
}
#[test]
fn workspace_session_created_becomes_an_upsert_candidate() {
    let (_temp, store) = temp_store();
    let mut app = ready_workspace_app(test_app_with_agent(), store);
    app.agents.get_mut(&AgentId(0)).unwrap().session.session_id = None;
    let _ = dispatch(
        Action::TaskComplete(TaskResult::SessionCreated {
            agent_id: AgentId(0),
            session_id: acp::SessionId::new("created"),
            models: None,
            modes: None,
        }),
        &mut app,
    );
    let effects = crate::app::workspace_sync::drain(&mut app);
    assert!(matches!(
        effects.as_slice(),
        [Effect::WriteWorkspace {
            mutation: WorkspaceMutation::Upsert(members),
            ..
        }]
            if members.len() == 1 && members.first().is_some_and(|m| m.key.session_id.as_ref() == "created")
    ));
}
/// An archive in flight hides the live agent's row even though the agent is still loaded.
#[test]
fn workspace_live_row_hidden_while_archive_is_in_flight() {
    let (_temp, mut store) = temp_store();
    store.insert_member(new_member("saved", "Saved")).unwrap();
    let mut app = ready_workspace_app(test_app_with_agent(), store);
    {
        let agent = app.agents.get_mut(&AgentId(0)).unwrap();
        agent.session.session_id = Some(acp::SessionId::new("saved"));
        agent.display_name = Some("Saved".into());
    }
    ensure_dashboard_state(&mut app);
    assert_eq!(dashboard_row_order(&app).len(), 1);
    assert!(crate::app::workspace_sync::request_removal(
        &mut app,
        "saved",
        crate::app::workspace_membership::RemovalCause::HistoryDeletedWithRetainedView,
    ));
    assert!(
        dashboard_row_order(&app).is_empty(),
        "a pending removal must hide both the member row and any provisional row",
    );
}
#[test]
fn workspace_overlay_closes_unbound_agent_without_store_removal() {
    let mut app = test_app_with_agent();
    let id = AgentId(0);
    app.workspace_dashboard_enabled = true;
    app.agents.get_mut(&id).unwrap().session.session_id = None;
    ensure_dashboard_state(&mut app);
    app.dashboard.as_mut().unwrap().attached_agent = Some(id);
    app.dashboard
        .as_mut()
        .unwrap()
        .focus_row(crate::views::dashboard::DashboardRowId::TopLevel(id));
    app.active_view = ActiveView::Agent(id);
    let effects = dispatch_dashboard_overlay_stop(&mut app);
    assert!(effects.is_empty());
    assert!(app.agents.is_empty());
    assert_eq!(app.active_view, ActiveView::AgentDashboard);
    let dashboard = app.dashboard.as_ref().unwrap();
    assert!(dashboard.attached_agent.is_none());
    assert!(dashboard.new_agent_button_focused());
}
#[test]
fn workspace_list_allows_archiving_waiting_for_user_row() {
    let mut app = test_app_with_agent();
    let id = AgentId(0);
    app.workspace_dashboard_enabled = true;
    ensure_dashboard_state(&mut app);
    app.dashboard
        .as_mut()
        .unwrap()
        .focus_row(crate::views::dashboard::DashboardRowId::TopLevel(id));
    app.active_view = ActiveView::AgentDashboard;
    app.agents
        .get_mut(&id)
        .unwrap()
        .permission_queue
        .push_back(crate::app::agent_view::test_fixtures::make_followup_permission_state());
    let effects = dispatch_dashboard_stop(&mut app);
    assert!(effects.is_empty());
    assert!(app.dashboard.as_ref().unwrap().delete_confirm.is_some());
    let effects = dispatch_dashboard_stop(&mut app);
    assert!(
        effects
            .iter()
            .all(|effect| !matches!(effect, Effect::DeleteSession { .. }))
    );
    assert!(!app.agents.contains_key(&id));
}
#[test]
fn workspace_list_silently_blocks_archive_during_replay() {
    let mut app = test_app_with_agent();
    let id = AgentId(0);
    app.workspace_dashboard_enabled = true;
    ensure_dashboard_state(&mut app);
    app.dashboard
        .as_mut()
        .unwrap()
        .focus_row(crate::views::dashboard::DashboardRowId::TopLevel(id));
    app.active_view = ActiveView::AgentDashboard;
    let agent = app.agents.get_mut(&id).unwrap();
    agent.session.loading_replay = true;
    agent.session.state = crate::app::agent::AgentState::TurnRunning;
    let effects = dispatch_dashboard_stop(&mut app);
    assert!(effects.is_empty());
    assert!(app.dashboard.as_ref().unwrap().delete_confirm.is_none());
    assert!(app.agents.contains_key(&id));
}
/// In leader mode the live FleetView roster is the source, so opening must
/// fetch that roster immediately (not wait for the poll tick) and must NOT
/// also fetch the local on-disk list.
#[serial_test::serial(CODEL_AGENT_DASHBOARD)]
#[test]
fn dashboard_open_with_leader_fetches_roster_not_local_sessions() {
    let mut app = test_app_with_agent();
    app.leader_mode = true;
    app.active_view = ActiveView::Agent(AgentId(0));
    let effects = dispatch_open_dashboard(&mut app);
    assert!(
        effects.iter().any(|e| matches!(e, Effect::FetchRoster)),
        "leader dashboard open must fetch the live roster immediately",
    );
    assert!(
        !effects
            .iter()
            .any(|e| matches!(e, Effect::FetchDashboardSessions)),
        "leader dashboard open must not fetch the local on-disk list",
    );
    assert!(
        app.dashboard_sessions_loading,
        "leader open must show Loading sessions until RosterLoaded",
    );
}
/// Seed a model into the app catalog for `/model` tests.
fn seed_model(app: &mut AppView, id: &str, name: &str) {
    let model_id = acp::ModelId::new(std::sync::Arc::from(id));
    app.models.available.insert(
        model_id.clone(),
        acp::ModelInfo::new(model_id, name.to_string()),
    );
}
/// The sessions picker modal was removed; `/sessions` survives as an alias
/// of `/dashboard`. It must resolve to the dashboard command and inherit
/// the dashboard feature-flag gate (hidden by canonical name, fail-closed).
#[serial_test::serial(CODEL_AGENT_DASHBOARD)]
#[test]
fn dashboard_slash_sessions_aliases_dashboard() {
    let mut app = three_agent_app();
    open_dashboard(&mut app);
    let dashboard = app.dashboard.as_mut().unwrap();
    let registry = dashboard.dispatch.slash_controller.registry_mut();
    registry.set_dashboard_visible(false);
    assert!(
        registry.get_for_dispatch("sessions").is_none(),
        "/sessions must inherit the dashboard feature-flag gate"
    );
    registry.set_dashboard_visible(true);
    let cmd = registry
        .get("sessions")
        .expect("/sessions must resolve as an alias of /dashboard");
    assert_eq!(
        cmd.name(),
        "dashboard",
        "/sessions must be an alias of the dashboard command"
    );
}
#[serial_test::serial(CODEL_AGENT_DASHBOARD)]
#[test]
fn dashboard_does_not_advertise_or_dispatch_doctor() {
    let mut app = three_agent_app();
    open_dashboard(&mut app);
    let expected = format!(
        "{} /doctor only works in a session",
        crate::glyphs::ballot_x()
    );
    for name in [
        "doctor",
        "terminal-setup",
        "terminal-check",
        "terminal-info",
    ] {
        let command = format!("/{name}");
        let dashboard = app.dashboard.as_mut().unwrap();
        dashboard.dispatch.set_text(&command);
        dashboard.dispatch.refresh_slash(&app.models);
        assert!(
            !dashboard.dispatch.slash_snapshot().command_recognized,
            "{command} must not be advertised on the session-less dashboard"
        );
        let effects = dispatch_dashboard_dispatch_slash(&mut app, command);
        assert!(effects.is_empty(), "must not enqueue spawn effects");
        assert_eq!(app.agents.len(), 3, "must not add an agent");
        let dashboard = app.dashboard.as_ref().unwrap();
        assert_eq!(dashboard.dispatch.text(), "");
        assert_eq!(dashboard.error_toast.as_deref(), Some(expected.as_str()));
    }
}
/// The dashboard modal's own fetch generation and its open state.
fn dashboard_usage_modal(app: &AppView) -> &crate::views::usage_modal::UsageInfoModalState {
    app.dashboard
        .as_ref()
        .unwrap()
        .usage_modal
        .as_ref()
        .expect("usage modal open on the dashboard")
}
/// Leaving the dashboard back into an overlay keeps a live subagent takeover and selects the
/// parent's top-level row.
#[serial_test::serial(CODEL_AGENT_DASHBOARD)]
#[test]
fn dashboard_overlay_return_keeps_takeover_on_top_level_row() {
    let mut app = test_app_with_agent();
    open_dashboard(&mut app);
    let parent = AgentId(0);
    mark_agent_nonempty(&mut app, parent);
    let child_sid = "child-return".to_string();
    {
        let agent = app.agents.get_mut(&parent).unwrap();
        agent
            .subagent_sessions
            .insert(child_sid.clone(), make_test_subagent(&child_sid, "sa-ret"));
        agent.active_subagent = Some(child_sid.clone());
    }
    app.active_view = ActiveView::Agent(parent);
    if let Some(d) = app.dashboard.as_mut() {
        d.attached_agent = Some(parent);
    }
    let _ = dispatch_dashboard_overlay_exit(&mut app);
    let _ = dispatch_exit_dashboard(&mut app);
    assert_eq!(app.active_view, ActiveView::Agent(parent));
    assert_eq!(
        app.dashboard.as_ref().and_then(|d| d.attached_agent),
        Some(parent)
    );
    assert_eq!(
        test_agent(&app, parent).active_subagent.as_deref(),
        Some(child_sid.as_str())
    );
    assert_eq!(
        app.dashboard.as_ref().and_then(|d| d.selected.clone()),
        Some(crate::views::dashboard::DashboardRowId::TopLevel(parent))
    );
}
/// A stale takeover (child id absent from `subagent_sessions`) is cleared on the same overlay
/// return, and the dashboard selection is still the parent's top-level row.
#[serial_test::serial(CODEL_AGENT_DASHBOARD)]
#[test]
fn dashboard_overlay_return_clears_stale_takeover_on_top_level_row() {
    let mut app = test_app_with_agent();
    open_dashboard(&mut app);
    let parent = AgentId(0);
    mark_agent_nonempty(&mut app, parent);
    {
        let agent = app.agents.get_mut(&parent).unwrap();
        agent.active_subagent = Some("missing-child".to_string());
    }
    app.active_view = ActiveView::Agent(parent);
    if let Some(d) = app.dashboard.as_mut() {
        d.attached_agent = Some(parent);
    }
    let _ = dispatch_dashboard_overlay_exit(&mut app);
    let _ = dispatch_exit_dashboard(&mut app);
    assert!(test_agent(&app, parent).active_subagent.is_none());
    assert_eq!(
        app.dashboard.as_ref().and_then(|d| d.selected.clone()),
        Some(crate::views::dashboard::DashboardRowId::TopLevel(parent))
    );
}
/// `DashboardOverlayExit` returns the user to the dashboard from an attached agent view and clears the overlay state.
/// the dashboard from an attached agent view and clears the
/// overlay state.
#[serial_test::serial(CODEL_AGENT_DASHBOARD)]
#[test]
fn dashboard_overlay_exit_returns_to_dashboard() {
    let mut app = test_app_with_agent();
    open_dashboard(&mut app);
    let id = AgentId(0);
    let _ = dispatch_dashboard_attach(
        &mut app,
        crate::views::dashboard::DashboardRowId::TopLevel(id),
    );
    assert!(matches!(app.active_view, ActiveView::Agent(a) if a == id));
    assert_eq!(app.dashboard.as_ref().unwrap().attached_agent, Some(id));
    let _ = dispatch_dashboard_overlay_exit(&mut app);
    assert!(matches!(app.active_view, ActiveView::AgentDashboard));
    assert_eq!(app.dashboard.as_ref().unwrap().attached_agent, None);
}
/// Overlay stop on the ONLY session: the close is refused (same guard as session close), but the user still lands on the dashboard with the refusal toast surfaced there; the session itself survives.
#[serial_test::serial(CODEL_AGENT_DASHBOARD)]
#[test]
fn dashboard_overlay_stop_only_session_refused_lands_on_dashboard() {
    let mut app = test_app_with_agent();
    mark_agent_nonempty(&mut app, AgentId(0));
    open_dashboard(&mut app);
    let id = AgentId(0);
    let _ = dispatch_dashboard_attach(
        &mut app,
        crate::views::dashboard::DashboardRowId::TopLevel(id),
    );
    let effects = dispatch_dashboard_overlay_stop(&mut app);
    assert!(effects.is_empty(), "refused close must produce no effects");
    assert!(matches!(app.active_view, ActiveView::AgentDashboard));
    assert!(
        app.agents.contains_key(&id),
        "the only session must survive the refused close",
    );
    assert!(
        app.dashboard
            .as_ref()
            .unwrap()
            .error_toast
            .as_deref()
            .is_some_and(|t| t.contains("Cannot close")),
        "the refusal toast must surface on the dashboard",
    );
}
/// Cycling with only one agent is a no-op (no view switch, no state mutation).
/// view switch, no state mutation).
#[serial_test::serial(CODEL_AGENT_DASHBOARD)]
#[test]
fn dashboard_overlay_cycle_noop_with_single_agent() {
    let mut app = test_app_with_agent();
    open_dashboard(&mut app);
    let id = AgentId(0);
    let _ = dispatch_dashboard_attach(
        &mut app,
        crate::views::dashboard::DashboardRowId::TopLevel(id),
    );
    let _ = dispatch_dashboard_overlay_cycle(&mut app, 1);
    assert_eq!(app.dashboard.as_ref().unwrap().attached_agent, Some(id));
    assert!(matches!(app.active_view, ActiveView::Agent(a) if a == id));
}
#[serial_test::serial(CODEL_AGENT_DASHBOARD)]
#[test]
fn dashboard_overlay_cycle_non_overlay_single_agent_is_noop() {
    let mut app = test_app_with_agent();
    let id = AgentId(0);
    mark_agent_nonempty(&mut app, id);
    app.active_view = ActiveView::Agent(id);
    assert!(app.dashboard.is_none());
    let _ = dispatch_dashboard_overlay_cycle(&mut app, 1);
    assert!(
        matches!(app.active_view, ActiveView::Agent(a) if a == id),
        "single-agent next must not switch views, got {:?}",
        app.active_view,
    );
    assert_eq!(
        app.dashboard.as_ref().and_then(|d| d.attached_agent),
        None,
        "single-agent cycle must not attach overlay chrome",
    );
    let _ = dispatch_dashboard_overlay_cycle(&mut app, -1);
    assert!(
        matches!(app.active_view, ActiveView::Agent(a) if a == id),
        "single-agent prev must not switch views, got {:?}",
        app.active_view,
    );
    assert_eq!(app.dashboard.as_ref().and_then(|d| d.attached_agent), None,);
    assert!(
        app.dashboard.is_none(),
        "single-agent no-op must not materialize the dashboard (no load_persisted side effect)",
    );
}
/// `DashboardToggleAutoApprove` flips `yolo_mode` on the selected row's owning agent. Reuses `set_yolo_mode` by temporarily switching `active_view`, so the existing toast
/// / persist / queue-drain logic all apply.
#[serial_test::serial(CODEL_AGENT_DASHBOARD)]
#[test]
fn dashboard_toggle_auto_approve_flips_yolo_on_selected_agent() {
    let mut app = test_app_with_agent();
    let id = AgentId(0);
    app.agents.get_mut(&id).unwrap().session.yolo_mode = false;
    app.active_view = ActiveView::Welcome;
    open_dashboard(&mut app);
    if let Some(d) = app.dashboard.as_mut() {
        d.focus_row(crate::views::dashboard::DashboardRowId::TopLevel(id));
    }
    let _ = dispatch_dashboard_toggle_auto_approve(&mut app);
    assert!(
        app.agents.get(&id).unwrap().session.yolo_mode,
        "first toggle must turn YOLO on",
    );
    let _ = dispatch_dashboard_toggle_auto_approve(&mut app);
    assert!(
        !app.agents.get(&id).unwrap().session.yolo_mode,
        "second toggle must turn YOLO off",
    );
    assert!(matches!(app.active_view, ActiveView::AgentDashboard));
}
/// End-to-end rename flow: begin rename, type characters, then commit.
/// Untitled fixture agent has no display_name / generated title, so the draft prefills empty; typing then commit emit `RenameSession` and stamp
/// `display_name`.
#[serial_test::serial(CODEL_AGENT_DASHBOARD)]
#[test]
fn dashboard_rename_end_to_end_top_level_row() {
    let mut app = test_app_with_agent();
    open_dashboard(&mut app);
    let id = AgentId(0);
    if let Some(d) = app.dashboard.as_mut() {
        d.selected = Some(crate::views::dashboard::DashboardRowId::TopLevel(id));
    }
    dispatch_dashboard_begin_rename(&mut app);
    assert_eq!(
        app.dashboard
            .as_ref()
            .unwrap()
            .rename
            .as_ref()
            .expect("begin_rename must arm the rename overlay")
            .text(),
        "",
        "untitled agent prefills an empty draft",
    );
    let registry = crate::actions::ActionRegistry::defaults();
    for character in "My renamed session".chars() {
        let outcome = app.dashboard.as_mut().unwrap().handle_input(
            &crossterm::event::Event::Key(crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Char(character),
                crossterm::event::KeyModifiers::NONE,
            )),
            &registry,
        );
        assert!(matches!(
            outcome,
            crate::app::app_view::InputOutcome::Changed
        ));
    }
    assert_eq!(
        app.dashboard
            .as_ref()
            .unwrap()
            .rename
            .as_ref()
            .unwrap()
            .text(),
        "My renamed session",
    );
    let effects = dispatch(Action::DashboardCommitRename, &mut app);
    assert!(
        effects.iter().any(|e| matches!(
            e,
            Effect::RenameSession { agent_id, title, kind, .. }
                if *agent_id == id
                    && title == "My renamed session"
                    && *kind == codel_shell::session::unified_list::SessionKind::Build
        )),
        "commit must emit a RenameSession effect, got {effects:?}",
    );
    assert_eq!(
        app.agents.get(&id).and_then(|a| a.display_name.clone()),
        Some("My renamed session".to_string()),
    );
    assert!(
        app.dashboard.as_ref().unwrap().rename.is_none(),
        "commit must clear the rename overlay",
    );
}
/// Dashboard rename of a chat-kind agent must stamp `kind: Chat` so the shell takes the conversations fork.
/// shell takes the conversations fork.
#[serial_test::serial(CODEL_AGENT_DASHBOARD)]
#[test]
fn dashboard_rename_chat_kind_stamps_kind_chat() {
    let mut app = test_app_with_agent();
    let agent = app.agents.get_mut(&AgentId(0)).unwrap();
    agent.chat_kind = true;
    agent.conversation_entry = true;
    open_dashboard(&mut app);
    let id = AgentId(0);
    if let Some(d) = app.dashboard.as_mut() {
        d.selected = Some(crate::views::dashboard::DashboardRowId::TopLevel(id));
    }
    dispatch_dashboard_begin_rename(&mut app);
    app.dashboard
        .as_mut()
        .and_then(|dashboard| dashboard.rename.as_mut())
        .expect("rename draft")
        .set_text("Chat title");
    let effects = dispatch(Action::DashboardCommitRename, &mut app);
    assert!(
        effects.iter().any(|e| matches!(
            e,
            Effect::RenameSession { agent_id, title, kind, .. }
                if *agent_id == id
                    && title == "Chat title"
                    && *kind == codel_shell::session::unified_list::SessionKind::Chat
        )),
        "chat-lane dashboard rename must send kind=chat, got {effects:?}",
    );
}
/// `DashboardCancelRename` emits no effects and leaves `display_name` untouched. Previously named
/// `dashboard_rename_cancel_via_esc_does_not_emit_effect` but that name implied Esc keystroke routing; the test actually dispatches `Action::DashboardCancelRename` directly.
/// The Esc-keystroke routing is now pinned by the sibling test
#[serial_test::serial(CODEL_AGENT_DASHBOARD)]
#[test]
fn dashboard_rename_cancel_action_emits_no_effect() {
    let mut app = test_app_with_agent();
    open_dashboard(&mut app);
    let id = AgentId(0);
    if let Some(d) = app.dashboard.as_mut() {
        d.selected = Some(crate::views::dashboard::DashboardRowId::TopLevel(id));
    }
    dispatch_dashboard_begin_rename(&mut app);
    app.dashboard
        .as_mut()
        .and_then(|dashboard| dashboard.rename.as_mut())
        .expect("rename draft")
        .set_text("scratch");
    let effects = dispatch(Action::DashboardCancelRename, &mut app);
    assert!(effects.is_empty(), "cancel must not emit effects");
    assert!(
        app.dashboard.as_ref().unwrap().rename.is_none(),
        "cancel must clear the rename overlay",
    );
    assert!(
        app.agents
            .get(&id)
            .and_then(|a| a.display_name.clone())
            .is_none(),
        "cancel must not stamp display_name",
    );
}
/// Drive Esc through `state.handle_input`
/// (the real keystroke path) to verify rename-mode wiring. A future change that rewires Esc to a different action in rename mode would silently break user expectation; the action-level test wouldn't catch it.
#[serial_test::serial(CODEL_AGENT_DASHBOARD)]
#[test]
fn dashboard_rename_esc_keystroke_routes_to_cancel() {
    use crate::actions::ActionRegistry;
    use crate::app::app_view::InputOutcome;
    use crate::views::dashboard::state::{DashboardState, RenameDraft};
    use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
    let registry = ActionRegistry::defaults();
    let mut state = DashboardState::new();
    let id = crate::views::dashboard::DashboardRowId::TopLevel(AgentId(0));
    state.selected = Some(id.clone());
    state.rename = Some(RenameDraft::new(id.clone(), "draft"));
    let esc = Event::Key(KeyEvent {
        code: KeyCode::Esc,
        modifiers: KeyModifiers::NONE,
        kind: KeyEventKind::Press,
        state: KeyEventState::NONE,
    });
    let outcome = state.handle_input(&esc, &registry);
    assert!(
        matches!(outcome, InputOutcome::Action(Action::DashboardCancelRename)),
        "Esc in rename mode must produce DashboardCancelRename, got {outcome:?}",
    );
}
/// Empty rename draft cancels without emitting an Effect.
#[serial_test::serial(CODEL_AGENT_DASHBOARD)]
#[test]
fn dashboard_commit_rename_empty_does_not_emit_effect() {
    let mut app = test_app_with_agent();
    open_dashboard(&mut app);
    if let Some(d) = app.dashboard.as_mut() {
        d.selected = Some(crate::views::dashboard::DashboardRowId::TopLevel(AgentId(
            0,
        )));
        d.rename = Some(crate::views::dashboard::state::RenameDraft::new(
            crate::views::dashboard::DashboardRowId::TopLevel(AgentId(0)),
            "   ",
        ));
    }
    let effects = dispatch_dashboard_commit_rename(&mut app);
    assert!(
        effects.is_empty(),
        "empty draft must not produce a RenameSession effect"
    );
    assert!(app.dashboard.as_ref().unwrap().rename.is_none());
}
/// Begin-rename prefills the draft from the agent's `display_name`.
#[serial_test::serial(CODEL_AGENT_DASHBOARD)]
#[test]
fn dashboard_begin_rename_prefills_display_name() {
    let mut app = test_app_with_agent();
    let id = AgentId(0);
    app.agents.get_mut(&id).unwrap().display_name = Some("  My Session Name  ".into());
    open_dashboard(&mut app);
    if let Some(d) = app.dashboard.as_mut() {
        d.selected = Some(crate::views::dashboard::DashboardRowId::TopLevel(id));
    }
    dispatch_dashboard_begin_rename(&mut app);
    assert_eq!(
        app.dashboard
            .as_ref()
            .unwrap()
            .rename
            .as_ref()
            .expect("begin_rename must arm the rename overlay")
            .text(),
        "My Session Name",
        "begin-rename must prefill trimmed display_name",
    );
}
/// Whitespace-only `display_name` falls through to `generated_session_title`.
#[serial_test::serial(CODEL_AGENT_DASHBOARD)]
#[test]
fn dashboard_begin_rename_whitespace_display_name_falls_through() {
    let mut app = test_app_with_agent();
    let id = AgentId(0);
    {
        let agent = app.agents.get_mut(&id).unwrap();
        agent.display_name = Some("   ".into());
        agent.generated_session_title = Some("Generated".into());
    }
    open_dashboard(&mut app);
    if let Some(d) = app.dashboard.as_mut() {
        d.selected = Some(crate::views::dashboard::DashboardRowId::TopLevel(id));
    }
    dispatch_dashboard_begin_rename(&mut app);
    assert_eq!(
        app.dashboard
            .as_ref()
            .unwrap()
            .rename
            .as_ref()
            .expect("begin_rename must arm the rename overlay")
            .text(),
        "Generated",
        "whitespace-only display_name must fall through to generated_session_title",
    );
}
/// Dispatch text and filter survive a close
/// and reopen of the dashboard. The contract is
/// "in-memory state preserved across reopen"; this test pins it.
#[serial_test::serial(CODEL_AGENT_DASHBOARD)]
#[test]
fn dashboard_state_preserved_across_reopen() {
    let mut app = test_app_with_agent();
    open_dashboard(&mut app);
    if let Some(d) = app.dashboard.as_mut() {
        d.dispatch.set_text("draft prompt");
        d.filter = crate::views::dashboard::Filter::Substring("foo".into());
    }
    let _ = dispatch_exit_dashboard(&mut app);
    let _ = dispatch_open_dashboard(&mut app);
    let d = app.dashboard.as_ref().unwrap();
    assert_eq!(
        d.dispatch.text(),
        "draft prompt",
        "dispatch text must survive reopen",
    );
    assert!(
        matches!(d.filter, crate::views::dashboard::Filter::Substring(ref s) if s == "foo"),
        "filter must survive reopen, got {:?}",
        d.filter
    );
}
/// Opening dashboard while it's already open closes it.
#[serial_test::serial(CODEL_AGENT_DASHBOARD)]
#[test]
fn dashboard_open_then_open_again_closes() {
    let mut app = test_app_with_agent();
    let _ = dispatch_open_dashboard(&mut app);
    assert!(matches!(app.active_view, ActiveView::AgentDashboard));
    let _ = dispatch_open_dashboard(&mut app);
    assert!(!matches!(app.active_view, ActiveView::AgentDashboard));
}
/// Stale pinned ids are dropped at open.
#[serial_test::serial(CODEL_AGENT_DASHBOARD)]
#[test]
fn dashboard_open_drops_pinned_ids_for_missing_agents() {
    let mut app = test_app_with_agent();
    app.dashboard = Some(crate::views::dashboard::DashboardState::new());
    if let Some(d) = app.dashboard.as_mut() {
        d.pinned
            .insert(crate::views::dashboard::DashboardRowId::TopLevel(AgentId(
                99,
            )));
    }
    app.active_view = ActiveView::Welcome;
    let _ = dispatch_open_dashboard(&mut app);
    let d = app.dashboard.as_ref().unwrap();
    assert!(d.pinned.is_empty(), "stale pin should be gc'd at open");
}
#[test]
fn confirmed_workspace_archive_reports_when_session_became_busy() {
    let mut app = test_app_with_agent();
    let id = AgentId(0);
    app.agents.get_mut(&id).unwrap().session.session_id = Some(acp::SessionId::new("archive-race"));
    app.workspace_dashboard_enabled = true;
    app.workspace_membership
        .set_snapshot_for_test(codel_dashboard_store::WorkspaceSnapshot {
            grouping: codel_dashboard_store::Grouping::State,
            members: vec![codel_dashboard_store::Member {
                session_id: codel_dashboard_store::SessionId::new("archive-race").unwrap(),
                kind: codel_dashboard_store::MemberKind::Build,
                origin: codel_dashboard_store::MemberOrigin::Local,
                cwd: Some("/tmp".into()),
                title: Some("archive race".into()),
                model: None,
                last_turn_summary: None,
                is_worktree: false,
                last_change_unix_ms: 1,
                pin_rank: None,
                order_rank: None,
            }],
            data_version: 1,
        });
    ensure_dashboard_state(&mut app);
    app.active_view = ActiveView::AgentDashboard;
    app.dashboard
        .as_mut()
        .unwrap()
        .focus_row(crate::views::dashboard::DashboardRowId::TopLevel(id));
    assert!(dispatch_dashboard_stop(&mut app).is_empty());
    app.agents.get_mut(&id).unwrap().session.loading_replay = true;
    let effects = dispatch_dashboard_delete(&mut app);
    assert!(effects.is_empty());
    assert!(app.agents.contains_key(&id));
    assert!(
        !app.workspace_membership.removal_pending_for_test(
            &codel_dashboard_store::SessionId::new("archive-race").unwrap()
        )
    );
    let dashboard = app.dashboard.as_ref().unwrap();
    assert!(dashboard.delete_confirm.is_none());
    assert_eq!(
        dashboard.error_toast.as_deref(),
        Some("Session became active; stop it before archiving")
    );
}
#[test]
fn workspace_overlay_ctrl_x_archives_settled_only_agent() {
    let mut app = test_app_with_agent();
    let id = AgentId(0);
    {
        let agent = app.agents.get_mut(&id).unwrap();
        agent.display_name = Some("only agent".into());
        agent.session.session_id = Some(acp::SessionId::new("only-archive"));
    }
    app.workspace_dashboard_enabled = true;
    app.workspace_membership
        .set_snapshot_for_test(codel_dashboard_store::WorkspaceSnapshot {
            grouping: codel_dashboard_store::Grouping::State,
            members: vec![codel_dashboard_store::Member {
                session_id: codel_dashboard_store::SessionId::new("only-archive").unwrap(),
                kind: codel_dashboard_store::MemberKind::Build,
                origin: codel_dashboard_store::MemberOrigin::Local,
                cwd: Some("/tmp".into()),
                title: Some("only agent".into()),
                model: None,
                last_turn_summary: None,
                is_worktree: false,
                last_change_unix_ms: 1,
                pin_rank: None,
                order_rank: None,
            }],
            data_version: 1,
        });
    ensure_dashboard_state(&mut app);
    app.dashboard.as_mut().unwrap().attached_agent = Some(id);
    app.dashboard
        .as_mut()
        .unwrap()
        .focus_row(crate::views::dashboard::DashboardRowId::TopLevel(id));
    app.active_view = ActiveView::Agent(id);
    let effects = dispatch_dashboard_overlay_stop(&mut app);
    assert!(app.agents.is_empty());
    assert!(matches!(app.active_view, ActiveView::AgentDashboard));
    assert!(app.dashboard.as_ref().unwrap().attached_agent.is_none());
    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, Effect::UnregisterActiveSession { .. }))
    );
    assert!(
        !effects
            .iter()
            .any(|effect| matches!(effect, Effect::DeleteSession { .. }))
    );
    assert!(app.dashboard.as_ref().unwrap().error_toast.is_none());
}
#[test]
fn workspace_overlay_ctrl_x_stops_background_work_before_archive() {
    let mut app = test_app_with_agent();
    let id = AgentId(0);
    let agent = app.agents.get_mut(&id).unwrap();
    agent.session.scheduled_tasks.insert(
        "loop-1".into(),
        crate::app::agent::ScheduledTaskInfo {
            task_id: "loop-1".into(),
            prompt: "keep going".into(),
            human_schedule: "every 5m".into(),
            created_at: std::time::Instant::now(),
            next_fire_at: None,
            tag: "loop".into(),
            last_subagent_id: None,
        },
    );
    app.workspace_dashboard_enabled = true;
    ensure_dashboard_state(&mut app);
    app.dashboard.as_mut().unwrap().attached_agent = Some(id);
    app.active_view = ActiveView::Agent(id);
    let effects = dispatch_dashboard_overlay_stop(&mut app);
    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, Effect::DeleteScheduledTask { .. })),
        "background work must stop before archive can arm: {effects:?}"
    );
    assert!(app.agents.contains_key(&id));
    assert!(!app.workspace_membership.has_pending_removals_for_test());
    assert_eq!(app.dashboard.as_ref().unwrap().attached_agent, Some(id));
}
/// Types `text` into the dashboard input and sends it with Enter.
fn send_dashboard_draft(app: &mut AppView, text: &str) {
    for ch in text.chars() {
        app.handle_input(&Event::Key(KeyEvent::new(
            KeyCode::Char(ch),
            KeyModifiers::NONE,
        )));
    }
    let enter = Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    let InputOutcome::Action(send @ Action::DashboardDispatch { .. }) = app.handle_input(&enter)
    else {
        panic!("Enter with a draft must send");
    };
    let _ = dispatch(send, app);
}
#[serial_test::serial(CODEL_AGENT_DASHBOARD)]
#[test]
fn dashboard_focus_new_agent_button_action_clears_selection() {
    let mut app = test_app_with_agent();
    open_dashboard(&mut app);
    if let Some(d) = app.dashboard.as_mut() {
        d.focus_row(crate::views::dashboard::DashboardRowId::TopLevel(AgentId(
            0,
        )));
    }
    assert!(app.dashboard.as_ref().unwrap().selected.is_some());
    let _ = dispatch(Action::DashboardFocusNewAgentButton, &mut app);
    let d = app.dashboard.as_ref().unwrap();
    assert!(
        d.new_agent_button_focused(),
        "FocusNewAgentButton must light up the button flag",
    );
    assert!(
        d.selected.is_none(),
        "FocusNewAgentButton must clear `selected` so the invariant holds",
    );
}
/// Up-arrow on the FIRST row hands focus over to the `[+ New Agent]` button: the button behaves as a virtual row at index -1 so the user can walk straight off the top of the list onto it without an extra Esc.
#[serial_test::serial(CODEL_AGENT_DASHBOARD)]
#[test]
fn dashboard_up_arrow_from_first_row_focuses_button() {
    let mut app = test_app_with_agent();
    let id = AgentId(0);
    open_dashboard(&mut app);
    if let Some(d) = app.dashboard.as_mut() {
        d.focus_row(crate::views::dashboard::DashboardRowId::TopLevel(id));
    }
    let _ = dispatch(Action::DashboardSelectPrev, &mut app);
    let d = app.dashboard.as_ref().unwrap();
    assert!(
        d.new_agent_button_focused(),
        "Up from the first row must focus the button",
    );
    assert!(d.selected.is_none());
}
/// Up-arrow on the button is a no-op (no wrap). Mirrors the agents modal: the cursor sits on the button and stays there until you press Down or click a row.
/// agents modal: the cursor sits on the button and stays
/// there until you press Down or click a row.
#[serial_test::serial(CODEL_AGENT_DASHBOARD)]
#[test]
fn dashboard_up_arrow_on_button_is_noop() {
    let mut app = test_app_with_agent();
    open_dashboard(&mut app);
    if let Some(d) = app.dashboard.as_mut() {
        d.focus_new_agent_button();
    }
    let _ = dispatch(Action::DashboardSelectPrev, &mut app);
    let d = app.dashboard.as_ref().unwrap();
    assert!(
        d.new_agent_button_focused(),
        "Up on button must stay on button"
    );
    assert!(d.selected.is_none());
}
/// Down-arrow on the button walks to the first focusable. With state grouping ON (the default), that's the first section header; a second Down steps into the first row inside it. When there are no rows the cursor stays on the button.
#[serial_test::serial(CODEL_AGENT_DASHBOARD)]
#[test]
fn dashboard_down_arrow_on_button_selects_first_focusable() {
    let mut app = test_app_with_agent();
    let id = AgentId(0);
    mark_agent_nonempty(&mut app, id);
    open_dashboard(&mut app);
    if let Some(d) = app.dashboard.as_mut() {
        d.focus_new_agent_button();
    }
    let _ = dispatch(Action::DashboardSelectNext, &mut app);
    let d = app.dashboard.as_ref().unwrap();
    assert!(
        d.selected_section.is_some(),
        "Down on button must land on the first section header",
    );
    assert!(d.selected.is_none());
    assert!(!d.new_agent_button_focused());
    let _ = dispatch(Action::DashboardSelectNext, &mut app);
    let d = app.dashboard.as_ref().unwrap();
    assert_eq!(
        d.selected,
        Some(crate::views::dashboard::DashboardRowId::TopLevel(id)),
        "second Down must step into the first row",
    );
    assert!(d.selected_section.is_none());
}
#[serial_test::serial(CODEL_AGENT_DASHBOARD)]
#[test]
fn dashboard_down_arrow_from_open_session_selects_first_focusable() {
    let mut app = test_app_with_agent();
    let id = AgentId(0);
    mark_agent_nonempty(&mut app, id);
    open_dashboard(&mut app);
    app.dashboard.as_mut().unwrap().focus_open_session_button();
    let _ = dispatch(Action::DashboardSelectNext, &mut app);
    let dashboard = app.dashboard.as_ref().unwrap();
    assert!(dashboard.selected_section.is_some());
    assert!(!dashboard.open_session_button_focused());
    assert!(!dashboard.new_agent_button_focused());
}
/// Filter parser plumbing: State known.
#[serial_test::serial(CODEL_AGENT_DASHBOARD)]
#[test]
fn dashboard_filter_state_known_token_via_dispatch() {
    use crate::views::dashboard::{FilterValue, RowState};
    let mut app = test_app_with_agent();
    open_dashboard(&mut app);
    let _ = dispatch(
        crate::app::actions::Action::DashboardSetFilter(FilterValue::State(RowState::Idle)),
        &mut app,
    );
    let d = app.dashboard.as_ref().unwrap();
    assert!(matches!(
        d.filter,
        crate::views::dashboard::Filter::State(RowState::Idle)
    ));
}
/// count=0 returns empty even when scrollback has entries.
#[test]
fn extract_recent_lines_count_zero_is_empty() {
    use crate::scrollback::block::RenderBlock;
    let mut app = test_app_with_agent();
    let agent = app.agents.get_mut(&AgentId(0)).unwrap();
    agent
        .scrollback
        .push_block(RenderBlock::user_prompt("hello"));
    let out = crate::views::dashboard::peek::extract_recent_lines(agent, 0);
    assert!(out.is_empty());
}
/// Empty scrollback returns empty regardless of count.
#[test]
fn extract_recent_lines_empty_scrollback() {
    let app = test_app_with_agent();
    let agent = app.agents.get(&AgentId(0)).unwrap();
    let out = crate::views::dashboard::peek::extract_recent_lines(agent, 6);
    assert!(out.is_empty());
}
/// Pin the placeholder strings for the "no meaningful body" `RenderBlock` variants. A regression that changed `"(tool call)"` to `"(tool)"` would otherwise slip through.
#[test]
fn extract_recent_lines_tool_call_placeholder() {
    use crate::scrollback::block::RenderBlock;
    let mut app = test_app_with_agent();
    let agent = app.agents.get_mut(&AgentId(0)).unwrap();
    agent.scrollback.push_block(RenderBlock::tool_call(
        "Read",
        "src/main.rs (50 lines)",
        true,
    ));
    let out = crate::views::dashboard::peek::extract_recent_lines(agent, 6);
    assert_eq!(out, vec!["(tool call)".to_string()]);
}
/// `BgTask` projects to the `(background task)`
/// placeholder.
#[test]
fn extract_recent_lines_bg_task_placeholder() {
    use crate::scrollback::block::RenderBlock;
    let mut app = test_app_with_agent();
    let agent = app.agents.get_mut(&AgentId(0)).unwrap();
    agent
        .scrollback
        .push_block(RenderBlock::bg_task("npm install", "task-1"));
    let out = crate::views::dashboard::peek::extract_recent_lines(agent, 6);
    assert_eq!(out, vec!["(background task)".to_string()]);
}
/// Explicit first-line projection assertion for `RenderBlock::AgentMessage` (no UserPrompt prefix). Asserts the exact result vector to catch regressions that the "evidence by absence" test below would miss.
#[test]
fn extract_recent_lines_agent_message_explicit_first_line() {
    use crate::scrollback::block::RenderBlock;
    let mut app = test_app_with_agent();
    let agent = app.agents.get_mut(&AgentId(0)).unwrap();
    agent
        .scrollback
        .push_block(RenderBlock::agent_message("first\nSECOND\nTHIRD"));
    let out = crate::views::dashboard::peek::extract_recent_lines(agent, 6);
    assert_eq!(
        out,
        vec!["first".to_string()],
        "AgentMessage first-line projection must return exactly the first line",
    );
}
/// Each entry projects to its FIRST line.
#[test]
fn extract_recent_lines_first_line_only() {
    use crate::scrollback::block::RenderBlock;
    let mut app = test_app_with_agent();
    let agent = app.agents.get_mut(&AgentId(0)).unwrap();
    agent
        .scrollback
        .push_block(RenderBlock::user_prompt("line one\nline two\nline three"));
    let out = crate::views::dashboard::peek::extract_recent_lines(agent, 6);
    assert_eq!(out.len(), 1);
    let Some(line) = out.first() else {
        panic!("expected extracted line: {out:?}");
    };
    assert!(
        line.contains("line one"),
        "first line must be present, got {line:?}"
    );
    assert!(
        !line.contains("line two"),
        "second line must not appear, got {line:?}"
    );
    assert!(
        !line.contains("line three"),
        "third line must not appear, got {line:?}"
    );
}
/// Two entries returned in chronological order
/// (newest-last after the internal reverse).
#[test]
fn extract_recent_lines_chronological_order() {
    use crate::scrollback::block::RenderBlock;
    let mut app = test_app_with_agent();
    let agent = app.agents.get_mut(&AgentId(0)).unwrap();
    agent
        .scrollback
        .push_block(RenderBlock::user_prompt("first"));
    agent
        .scrollback
        .push_block(RenderBlock::user_prompt("second"));
    let out = crate::views::dashboard::peek::extract_recent_lines(agent, 6);
    assert_eq!(out.len(), 2);
    let [first, second] = out.as_slice() else {
        panic!("expected two extracted lines: {out:?}");
    };
    assert!(first.contains("first"));
    assert!(second.contains("second"));
}
/// ANSI escapes in scrollback content are stripped.
#[test]
fn extract_recent_lines_strips_ansi() {
    use crate::scrollback::block::RenderBlock;
    let mut app = test_app_with_agent();
    let agent = app.agents.get_mut(&AgentId(0)).unwrap();
    agent
        .scrollback
        .push_block(RenderBlock::user_prompt("hello \x1b[31mevil\x1b[0m world"));
    let out = crate::views::dashboard::peek::extract_recent_lines(agent, 6);
    assert_eq!(out.len(), 1);
    let Some(line) = out.first() else {
        panic!("expected extracted line: {out:?}");
    };
    assert!(
        !line.contains('\x1b'),
        "ANSI must be stripped, got: {line:?}"
    );
    assert!(line.contains("evil"));
}
#[serial_test::serial(CODEL_AGENT_DASHBOARD)]
#[test]
fn dashboard_delete_complete_returns_from_foreground_agent() {
    let mut app = test_app_with_agent();
    let id = AgentId(0);
    app.agents.get_mut(&id).unwrap().session.session_id = Some(acp::SessionId::new("sess-dash"));
    open_dashboard(&mut app);
    app.active_view = ActiveView::Agent(id);
    let _ = dispatch_task_result(
        crate::app::actions::TaskResult::DeleteSessionComplete {
            source: "current".into(),
            session_id: "sess-dash".into(),
            after: crate::app::actions::AfterSessionDelete::Dashboard,
        },
        &mut app,
    );
    assert!(!app.agents.contains_key(&id));
    assert!(matches!(app.active_view, ActiveView::AgentDashboard));
}
#[test]
fn workspace_dashboard_unbound_row_arms_local_close() {
    let mut app = test_app_with_agent();
    let id = AgentId(0);
    {
        let agent = app.agents.get_mut(&id).unwrap();
        agent.session.session_id = None;
        agent.session.state = AgentState::CommandCancelling {
            command: crate::app::agent::AgentCommand::Compact,
        };
    }
    app.workspace_dashboard_enabled = true;
    ensure_dashboard_state(&mut app);
    app.active_view = ActiveView::AgentDashboard;
    app.dashboard
        .as_mut()
        .unwrap()
        .focus_row(crate::views::dashboard::DashboardRowId::TopLevel(id));
    assert!(dispatch_dashboard_stop(&mut app).is_empty());
    let dashboard = app.dashboard.as_ref().unwrap();
    assert!(matches!(
        dashboard.delete_confirm.as_ref().map(|(row, _)| row),
        Some(crate::views::dashboard::DashboardRowId::TopLevel(AgentId(
            0
        )))
    ));
    assert!(dashboard.error_toast.is_none());
}
/// Happy path: matching ids, so no panic and the queue is popped.
/// Also assert the response was actually sent through the oneshot (not just popped). A regression that pops without sending the response would otherwise slip through this test.
#[serial_test::serial(CODEL_AGENT_DASHBOARD)]
#[test]
fn dashboard_permission_select_happy_path() {
    let mut app = test_app_with_agent();
    let agent = app.agents.get_mut(&AgentId(0)).unwrap();
    let mut rx = push_synthetic_permission(agent, 42, vec![("allow", "Allow")]);
    open_dashboard(&mut app);
    let _ = dispatch_dashboard_permission_select(
        &mut app,
        crate::views::dashboard::DashboardRowId::TopLevel(AgentId(0)),
        42,
        acp::PermissionOptionId::new(std::sync::Arc::from("allow")),
    );
    assert!(test_agent(&app, AgentId(0)).permission_queue.is_empty());
    let resp = rx
        .try_recv()
        .expect("response oneshot must have a value after dispatch");
    let resp = resp.expect("response must be Ok(RequestPermissionResponse)");
    match resp.outcome {
        acp::RequestPermissionOutcome::Selected(acp::SelectedPermissionOutcome {
            option_id,
            ..
        }) => {
            assert_eq!(
                option_id.0.as_ref(),
                "allow",
                "selected option_id must round-trip"
            );
        }
        other => panic!("expected Selected outcome, got {other:?}"),
    }
}
/// Stale request_id: refuses, sets toast, clears peek.
#[serial_test::serial(CODEL_AGENT_DASHBOARD)]
#[test]
fn dashboard_permission_select_drops_stale_request() {
    let mut app = test_app_with_agent();
    let agent = app.agents.get_mut(&AgentId(0)).unwrap();
    let _rx = push_synthetic_permission(agent, 99, vec![("allow", "Allow")]);
    open_dashboard(&mut app);
    if let Some(d) = app.dashboard.as_mut() {
        d.peek = Some(crate::views::dashboard::peek::PeekPanelState::new(
            crate::views::dashboard::DashboardRowId::TopLevel(AgentId(0)),
            crate::views::dashboard::peek::PeekFields {
                label: "label".into(),
                time_ago: String::new(),
                response_type: "Awaiting your input".into(),
                last_user_message: None,
                question: Some("q?".into()),
                options: vec![("allow".into(), "Allow".into())],
                request_id: Some(123),
                reject_option: None,
            },
        ));
    }
    let _ = dispatch_dashboard_permission_select(
        &mut app,
        crate::views::dashboard::DashboardRowId::TopLevel(AgentId(0)),
        123,
        acp::PermissionOptionId::new(std::sync::Arc::from("allow")),
    );
    assert_eq!(test_agent(&app, AgentId(0)).permission_queue.len(), 1);
    let d = app.dashboard.as_ref().unwrap();
    assert!(d.peek.is_none(), "peek must be closed");
    assert!(d.error_toast.is_some(), "toast must surface the mismatch");
}
/// Missing row: toasts, closes peek, returns no effects.
#[serial_test::serial(CODEL_AGENT_DASHBOARD)]
#[test]
fn dashboard_permission_select_for_missing_row_clears_peek() {
    let mut app = test_app_with_agent();
    open_dashboard(&mut app);
    if let Some(d) = app.dashboard.as_mut() {
        d.peek = Some(crate::views::dashboard::peek::PeekPanelState::new(
            crate::views::dashboard::DashboardRowId::TopLevel(AgentId(99)),
            crate::views::dashboard::peek::PeekFields {
                label: "label".into(),
                time_ago: String::new(),
                response_type: "Idle".into(),
                last_user_message: None,
                question: None,
                options: Vec::new(),
                request_id: None,
                reject_option: None,
            },
        ));
    }
    let effects = dispatch_dashboard_permission_select(
        &mut app,
        crate::views::dashboard::DashboardRowId::TopLevel(AgentId(99)),
        1,
        acp::PermissionOptionId::new(std::sync::Arc::from("allow")),
    );
    assert!(effects.is_empty());
    let d = app.dashboard.as_ref().unwrap();
    assert!(d.peek.is_none());
    assert!(d.error_toast.is_some());
}
/// A workspace (or roster) row is not a loaded session, so a peek reply is refused.
#[serial_test::serial(CODEL_AGENT_DASHBOARD)]
#[test]
fn dashboard_peek_reply_to_non_top_level_row_toasts() {
    let mut app = test_app_with_agent();
    open_dashboard(&mut app);
    let row = crate::views::dashboard::DashboardRowId::Workspace {
        session_id: "ws-not-loaded".to_string(),
    };
    if let Some(d) = app.dashboard.as_mut() {
        d.focus_row(row.clone());
    }
    let queued = test_agent(&app, AgentId(0)).session.queue_len();
    let effects = dispatch_dashboard_peek_reply(&mut app, row, "hi".into(), false);
    assert!(effects.is_empty());
    assert_eq!(test_agent(&app, AgentId(0)).session.queue_len(), queued);
    assert_eq!(
        app.dashboard.as_ref().unwrap().error_toast.as_deref(),
        Some(
            format!(
                "{} Load the session before replying",
                crate::glyphs::ballot_x()
            )
            .as_str()
        ),
    );
}
/// A tab whose session never opened refuses a peek reply and keeps nothing queued
#[serial_test::serial(CODEL_AGENT_DASHBOARD)]
#[test]
fn dashboard_peek_reply_to_failed_load_tab_toasts() {
    let mut app = test_app_with_agent();
    open_dashboard(&mut app);
    app.agents.get_mut(&AgentId(0)).unwrap().load_failed = true;
    let row = crate::views::dashboard::DashboardRowId::TopLevel(AgentId(0));
    let effects = dispatch_dashboard_peek_reply(&mut app, row, "hi".into(), false);
    assert!(effects.is_empty());
    assert_eq!(0, test_agent(&app, AgentId(0)).session.queue_len());
    assert!(
        app.dashboard
            .as_ref()
            .and_then(|d| d.error_toast.as_deref())
            .is_some_and(|toast| toast.contains("didn't open"))
    );
}
/// Peek reply to an IDLE agent sends immediately: the prompt drains
/// (one `SendPrompt` effect), the turn starts, and the reply draft
/// is cleared.
#[serial_test::serial(CODEL_AGENT_DASHBOARD)]
#[test]
fn dashboard_peek_reply_to_idle_agent_sends() {
    let mut app = test_app_with_agent();
    open_dashboard(&mut app);
    if let Some(d) = app.dashboard.as_mut() {
        d.selected = Some(crate::views::dashboard::DashboardRowId::TopLevel(AgentId(
            0,
        )));
        d.peek = Some(crate::views::dashboard::peek::PeekPanelState::new(
            crate::views::dashboard::DashboardRowId::TopLevel(AgentId(0)),
            crate::views::dashboard::peek::PeekFields {
                label: "label".into(),
                time_ago: String::new(),
                response_type: "Idle".into(),
                last_user_message: None,
                question: None,
                options: Vec::new(),
                request_id: None,
                reject_option: None,
            },
        ));
        d.peek_reply.set_text("please continue");
    }
    let effects = dispatch_dashboard_peek_reply(
        &mut app,
        crate::views::dashboard::DashboardRowId::TopLevel(AgentId(0)),
        "please continue".into(),
        false,
    );
    assert_eq!(effects.len(), 1);
    assert!(
        matches!(effects.first(), Some(Effect::SendPrompt { text, .. }) if text == "please continue")
    );
    assert!(test_agent(&app, AgentId(0)).session.state.is_turn_running());
    assert_eq!(test_agent(&app, AgentId(0)).session.queue_len(), 0);
    assert!(app.dashboard.as_ref().unwrap().peek_reply.text().is_empty());
}
/// Peek reply to a RUNNING agent queues the prompt (no effect) so it
/// drains after the current turn; the draft is still cleared.
#[serial_test::serial(CODEL_AGENT_DASHBOARD)]
#[test]
fn dashboard_peek_reply_to_running_agent_queues() {
    let mut app = test_app_with_agent();
    {
        let agent = app.agents.get_mut(&AgentId(0)).unwrap();
        agent.session.state = AgentState::TurnRunning;
        agent
            .scrollback
            .push_block(RenderBlock::user_prompt("current turn"));
        agent.scrollback.prepare_layout(80, 24);
        let current = agent.scrollback.len().saturating_sub(1);
        agent.scrollback.set_selected(Some(current));
        agent.scrollback.scroll_to_entry_top(current);
        agent.scrollback.enable_follow_with_preserve();
    }
    open_dashboard(&mut app);
    if let Some(d) = app.dashboard.as_mut() {
        d.selected = Some(crate::views::dashboard::DashboardRowId::TopLevel(AgentId(
            0,
        )));
        d.peek = Some(crate::views::dashboard::peek::PeekPanelState::new(
            crate::views::dashboard::DashboardRowId::TopLevel(AgentId(0)),
            crate::views::dashboard::peek::PeekFields {
                label: "label".into(),
                time_ago: String::new(),
                response_type: "Running\u{2026}".into(),
                last_user_message: None,
                question: None,
                options: Vec::new(),
                request_id: None,
                reject_option: None,
            },
        ));
        d.peek_reply.set_text("after this");
        d.begin_peek_viewport(
            crate::views::dashboard::DashboardRowId::TopLevel(AgentId(0)),
            &mut app.agents,
        );
    }
    let effects = dispatch_dashboard_peek_reply(
        &mut app,
        crate::views::dashboard::DashboardRowId::TopLevel(AgentId(0)),
        "after this".into(),
        false,
    );
    assert!(effects.is_empty());
    assert_eq!(test_agent(&app, AgentId(0)).session.queue_len(), 1);
    assert!(test_agent(&app, AgentId(0)).session.state.is_turn_running());
    let dashboard = app.dashboard.as_ref().unwrap();
    assert!(dashboard.peek_reply.text().is_empty());
    assert!(
        dashboard
            .peek_viewport
            .as_ref()
            .unwrap()
            .page_flip_entry
            .is_none(),
        "a blocked drain must not claim the current turn as the queued reply"
    );
}
/// Peek reply with an attached image drains into the queued
/// prompt and idle agents send `SendPromptBlocks` (not text-only).
#[serial_test::serial(CODEL_AGENT_DASHBOARD)]
#[test]
fn dashboard_peek_reply_with_image_sends_blocks() {
    let mut app = test_app_with_agent();
    open_dashboard(&mut app);
    let reply_text = {
        let d = app.dashboard.as_mut().unwrap();
        d.selected = Some(crate::views::dashboard::DashboardRowId::TopLevel(AgentId(
            0,
        )));
        d.peek = Some(crate::views::dashboard::peek::PeekPanelState::new(
            crate::views::dashboard::DashboardRowId::TopLevel(AgentId(0)),
            crate::views::dashboard::peek::PeekFields {
                label: "label".into(),
                time_ago: String::new(),
                response_type: "Idle".into(),
                last_user_message: None,
                question: None,
                options: Vec::new(),
                request_id: None,
                reject_option: None,
            },
        ));
        let img = crate::prompt_images::PastedImage {
            element_id: codel_ratatui_textarea::ElementId::from_raw(0),
            display_number: 0,
            mime_type: "image/png".into(),
            dimensions: Some((10, 10)),
            byte_len: 16,
            encoded_bytes: Some(vec![0u8; 16].into()),
            source_path: None,
            staged_temp_path: None,
            session_image_path: None,
            preview: crate::prompt_images::PromptImagePreview::default(),
        };
        d.peek_reply.insert_image(img).unwrap();
        d.peek_reply.text().to_string()
    };
    let effects = dispatch_dashboard_peek_reply(
        &mut app,
        crate::views::dashboard::DashboardRowId::TopLevel(AgentId(0)),
        reply_text,
        false,
    );
    assert_eq!(effects.len(), 1);
    assert!(
        matches!(effects.first(), Some(Effect::SendPromptBlocks { .. })),
        "image reply must send blocks, got {:?}",
        effects.first()
    );
    assert!(app.dashboard.as_ref().unwrap().peek_reply.text().is_empty());
    assert!(app.dashboard.as_ref().unwrap().peek_reply.images.is_empty());
}
/// Regression: whitespace around a peek-reply image chip must not desync chip ranges from the stored text (panicked on rewind restore).
/// desync chip ranges from the stored text (panicked on rewind restore).
#[serial_test::serial(CODEL_AGENT_DASHBOARD)]
#[test]
fn dashboard_peek_reply_image_with_whitespace_survives_rewind_restore() {
    let mut app = test_app_with_agent();
    open_dashboard(&mut app);
    let reply_text = {
        let d = app.dashboard.as_mut().unwrap();
        d.selected = Some(crate::views::dashboard::DashboardRowId::TopLevel(AgentId(
            0,
        )));
        d.peek = Some(crate::views::dashboard::peek::PeekPanelState::new(
            crate::views::dashboard::DashboardRowId::TopLevel(AgentId(0)),
            crate::views::dashboard::peek::PeekFields {
                label: "label".into(),
                time_ago: String::new(),
                response_type: "Idle".into(),
                last_user_message: None,
                question: None,
                options: Vec::new(),
                request_id: None,
                reject_option: None,
            },
        ));
        d.peek_reply.set_text("   ");
        d.peek_reply.set_cursor(3);
        let img = crate::prompt_images::PastedImage {
            element_id: codel_ratatui_textarea::ElementId::from_raw(0),
            display_number: 0,
            mime_type: "image/png".into(),
            dimensions: Some((10, 10)),
            byte_len: 16,
            encoded_bytes: Some(vec![0u8; 16].into()),
            source_path: None,
            staged_temp_path: None,
            session_image_path: None,
            preview: crate::prompt_images::PromptImagePreview::default(),
        };
        d.peek_reply.insert_image(img).unwrap();
        d.peek_reply.text().to_string()
    };
    let _ = dispatch_dashboard_peek_reply(
        &mut app,
        crate::views::dashboard::DashboardRowId::TopLevel(AgentId(0)),
        reply_text,
        false,
    );
    let stashed = test_agent(&app, AgentId(0))
        .session
        .in_flight_prompt
        .clone()
        .expect("idle send sets in_flight_prompt");
    for chip in &stashed.chip_elements {
        assert!(
            chip.range.end <= stashed.text.len(),
            "chip range {:?} out of bounds for stored text {:?} (len {})",
            chip.range,
            stashed.text,
            stashed.text.len()
        );
    }
    let agent = app.agents.get_mut(&AgentId(0)).unwrap();
    agent.prompt.set_text(&stashed.text);
    agent.prompt.restore_chip_elements(&stashed.chip_elements);
    agent.prompt.set_images(stashed.images);
    assert!(agent.prompt.text().contains("[Image #1]"));
    assert_eq!(agent.prompt.images.len(), 1);
}
/// Image on peek reply is preserved on the queued entry when the agent is mid-turn.
/// the agent is mid-turn.
#[serial_test::serial(CODEL_AGENT_DASHBOARD)]
#[test]
fn dashboard_peek_reply_with_image_queues_images() {
    let mut app = test_app_with_agent();
    app.agents.get_mut(&AgentId(0)).unwrap().session.state = AgentState::TurnRunning;
    open_dashboard(&mut app);
    let reply_text = {
        let d = app.dashboard.as_mut().unwrap();
        d.selected = Some(crate::views::dashboard::DashboardRowId::TopLevel(AgentId(
            0,
        )));
        d.peek = Some(crate::views::dashboard::peek::PeekPanelState::new(
            crate::views::dashboard::DashboardRowId::TopLevel(AgentId(0)),
            crate::views::dashboard::peek::PeekFields {
                label: "label".into(),
                time_ago: String::new(),
                response_type: "Running\u{2026}".into(),
                last_user_message: None,
                question: None,
                options: Vec::new(),
                request_id: None,
                reject_option: None,
            },
        ));
        let img = crate::prompt_images::PastedImage {
            element_id: codel_ratatui_textarea::ElementId::from_raw(0),
            display_number: 0,
            mime_type: "image/png".into(),
            dimensions: Some((10, 10)),
            byte_len: 16,
            encoded_bytes: Some(vec![0u8; 16].into()),
            source_path: None,
            staged_temp_path: None,
            session_image_path: None,
            preview: crate::prompt_images::PromptImagePreview::default(),
        };
        d.peek_reply.insert_image(img).unwrap();
        d.peek_reply.text().to_string()
    };
    let effects = dispatch_dashboard_peek_reply(
        &mut app,
        crate::views::dashboard::DashboardRowId::TopLevel(AgentId(0)),
        reply_text,
        false,
    );
    assert!(effects.is_empty());
    let queued = test_agent(&app, AgentId(0))
        .session
        .pending_prompts
        .front()
        .expect("queued prompt");
    assert_eq!(queued.images.len(), 1);
    assert!(
        !queued.chip_elements.is_empty(),
        "peek send must snapshot chips for cancel-restore"
    );
    assert!(app.dashboard.as_ref().unwrap().peek_reply.images.is_empty());
}
/// Peek "No, type to add feedback" path: resolves the front
/// permission with the `RejectOnce` option and attaches the typed
/// text as `followup_message` meta.
#[serial_test::serial(CODEL_AGENT_DASHBOARD)]
#[test]
fn dashboard_permission_followup_rejects_with_message() {
    let mut app = test_app_with_agent();
    let agent = app.agents.get_mut(&AgentId(0)).unwrap();
    let mut rx = push_synthetic_permission(agent, 5, vec![("allow", "Allow"), ("reject", "No")]);
    open_dashboard(&mut app);
    let effects = dispatch_dashboard_permission_followup(
        &mut app,
        crate::views::dashboard::DashboardRowId::TopLevel(AgentId(0)),
        5,
        "do it differently".into(),
    );
    assert!(effects.is_empty());
    let resp = rx.try_recv().expect("response sent").expect("ok");
    match resp.outcome {
        acp::RequestPermissionOutcome::Selected(acp::SelectedPermissionOutcome {
            option_id,
            ..
        }) => {
            assert_eq!(
                option_id.0.as_ref(),
                "reject",
                "must pick the RejectOnce option"
            );
        }
        other => panic!("expected Selected(reject), got {other:?}"),
    }
    let meta = resp.meta.expect("followup meta present");
    assert_eq!(
        meta.get("followup_message").and_then(|v| v.as_str()),
        Some("do it differently"),
    );
    assert_eq!(test_agent(&app, AgentId(0)).permission_queue.len(), 0);
}
/// Peek answering of the Ask tool (`AskUserQuestion`): selecting an option sends the ext-response and clears the question view.
/// option sends the ext-response and clears the question view.
#[serial_test::serial(CODEL_AGENT_DASHBOARD)]
#[test]
fn dashboard_question_answer_sends_and_clears() {
    use crate::views::prompt_widget::StashedPrompt;
    use crate::views::question_view::QuestionViewState;
    use codel_tools::implementations::codel_build::ask_user_question::{
        AskUserQuestionMode, Question, QuestionOption,
    };
    let mut app = test_app_with_agent();
    let opt = |label: &str| QuestionOption {
        label: label.to_string(),
        description: String::new(),
        preview: None,
        id: None,
    };
    let question = Question {
        question: "Which DB?".to_string(),
        options: vec![opt("Redis"), opt("Postgres")],
        multi_select: Some(false),
        id: None,
    };
    let (tx, mut rx) = tokio::sync::oneshot::channel();
    let qv = QuestionViewState::with_response_tx(
        "tc-1".to_string(),
        vec![question],
        StashedPrompt::default(),
        Some(tx),
        AskUserQuestionMode::Default,
    );
    app.agents.get_mut(&AgentId(0)).unwrap().question_view = Some(qv);
    open_dashboard(&mut app);
    let effects = dispatch_dashboard_question_answer(
        &mut app,
        crate::views::dashboard::DashboardRowId::TopLevel(AgentId(0)),
        Some(1),
        String::new(),
    );
    assert!(effects.is_empty());
    assert!(rx.try_recv().is_ok(), "ext-response must be sent");
    assert!(test_agent(&app, AgentId(0)).question_view.is_none());
}
/// A multi-question Ask form is walked one question at a time in the peek: answering advances to the next question (no submit yet) and resets the panel's per-question draft; the last answer submits.
/// peek: answering advances to the next question (no submit yet) and
/// resets the panel's per-question draft; the last answer submits.
#[serial_test::serial(CODEL_AGENT_DASHBOARD)]
#[test]
fn dashboard_question_answer_walks_multiple_questions() {
    use crate::views::dashboard::peek::{PeekPanelState, compute_peek_fields};
    use crate::views::prompt_widget::StashedPrompt;
    use crate::views::question_view::QuestionViewState;
    use codel_tools::implementations::codel_build::ask_user_question::{
        AskUserQuestionMode, Question, QuestionOption,
    };
    let mut app = test_app_with_agent();
    let opt = |label: &str| QuestionOption {
        label: label.to_string(),
        description: String::new(),
        preview: None,
        id: None,
    };
    let q1 = Question {
        question: "Which DB?".to_string(),
        options: vec![opt("Redis"), opt("Postgres")],
        multi_select: Some(false),
        id: None,
    };
    let q2 = Question {
        question: "Which cache?".to_string(),
        options: vec![opt("LRU"), opt("LFU")],
        multi_select: Some(false),
        id: None,
    };
    let (tx, mut rx) = tokio::sync::oneshot::channel();
    let qv = QuestionViewState::with_response_tx(
        "tc-1".to_string(),
        vec![q1, q2],
        StashedPrompt::default(),
        Some(tx),
        AskUserQuestionMode::Default,
    );
    app.agents.get_mut(&AgentId(0)).unwrap().question_view = Some(qv);
    open_dashboard(&mut app);
    let row = crate::views::dashboard::DashboardRowId::TopLevel(AgentId(0));
    let fields = compute_peek_fields(&row, &app.agents).expect("ask surfaced");
    assert!(fields.question.as_deref().unwrap().starts_with("(1/2)"));
    assert!(fields.request_id.is_none());
    assert_eq!(fields.reject_option, Some(2));
    if let Some(d) = app.dashboard.as_mut() {
        let mut p = PeekPanelState::new(row.clone(), fields);
        p.selected_option = Some(1);
        d.peek = Some(p);
        d.peek_reply.set_text("stale draft");
    }
    let effects = dispatch_dashboard_question_answer(&mut app, row.clone(), Some(1), String::new());
    assert!(effects.is_empty());
    assert!(
        rx.try_recv().is_err(),
        "must not submit until the last question"
    );
    let qv = test_agent(&app, AgentId(0)).question_view.as_ref().unwrap();
    assert_eq!(qv.active_tab, 1, "advanced to the second question");
    let d = app.dashboard.as_ref().unwrap();
    assert_eq!(d.peek.as_ref().unwrap().selected_option, None);
    assert!(d.peek_reply.text().is_empty());
    let effects = dispatch_dashboard_question_answer(&mut app, row, Some(0), String::new());
    assert!(effects.is_empty());
    assert!(rx.try_recv().is_ok(), "ext-response sent after last answer");
    assert!(test_agent(&app, AgentId(0)).question_view.is_none());
    assert!(app.dashboard.as_ref().unwrap().peek.is_none());
}
/// An empty session that is actively working stays visible: its first
/// user message may not be in scrollback yet, but it is doing real work.
#[test]
fn build_rows_keeps_empty_working_local_session() {
    use crate::views::dashboard::build_rows_with_roster;
    let mut app = test_app_with_agent();
    app.agents.get_mut(&AgentId(0)).unwrap().session.state = AgentState::TurnRunning;
    let rows = build_rows_with_roster(
        &app.agents,
        &std::collections::BTreeSet::new(),
        &[],
        crate::views::dashboard::Grouping::State,
        &crate::views::dashboard::Filter::None,
        None,
        &[],
    );
    assert_eq!(rows.len(), 1, "an actively-working session stays visible");
}
/// A session with a generated title renders normally.
#[test]
fn build_rows_keeps_titled_local_session() {
    use crate::views::dashboard::build_rows_with_roster;
    let mut app = test_app_with_agent();
    mark_agent_nonempty(&mut app, AgentId(0));
    let rows = build_rows_with_roster(
        &app.agents,
        &std::collections::BTreeSet::new(),
        &[],
        crate::views::dashboard::Grouping::State,
        &crate::views::dashboard::Filter::None,
        None,
        &[],
    );
    assert_eq!(rows.len(), 1, "a titled session renders");
}
/// A pinned empty session is kept (explicit user intent overrides the empty-session hide).
/// empty-session hide).
#[test]
fn build_rows_keeps_pinned_empty_local_session() {
    use crate::views::dashboard::build_rows_with_roster;
    let app = test_app_with_agent();
    let mut pinned = std::collections::BTreeSet::new();
    pinned.insert(crate::views::dashboard::DashboardRowId::TopLevel(AgentId(
        0,
    )));
    let rows = build_rows_with_roster(
        &app.agents,
        &pinned,
        &[],
        crate::views::dashboard::Grouping::State,
        &crate::views::dashboard::Filter::None,
        None,
        &[],
    );
    assert_eq!(rows.len(), 1, "a pinned empty session is kept");
}
/// The allocation-free readiness predicate must agree with the owned plan a stop would build, in every state that flips it.
#[test]
fn stop_readiness_predicate_matches_the_built_plan() {
    use crate::app::dispatch::dashboard::DashboardStopPlan;
    let mut app = test_app_with_agent();
    let agree = |agent: &AgentView| {
        assert_eq!(
            DashboardStopPlan::would_stop_anything(agent),
            !DashboardStopPlan::for_agent(agent).is_empty()
        );
    };
    let agent = test_agent_mut(&mut app, AgentId(0));
    agent.session.session_id = Some("bound".into());
    assert!(!DashboardStopPlan::would_stop_anything(agent));
    agree(agent);
    agent.session.state = crate::app::agent::AgentState::TurnRunning;
    assert!(DashboardStopPlan::would_stop_anything(agent));
    agree(agent);
    agent.session.state = crate::app::agent::AgentState::Idle;
    agent.running_wake_turn = Some(crate::app::agent_view::RunningWakeTurn {
        prompt_id: "wake-1".into(),
        cancel_sent: false,
    });
    assert!(DashboardStopPlan::would_stop_anything(agent));
    agree(agent);
    agent.running_wake_turn = None;
    let mut done = super::make_bg_task("bg-done");
    done.status = crate::app::agent::BgTaskStatus::Done;
    agent.session.bg_tasks.insert("bg-done".into(), done);
    assert!(!DashboardStopPlan::would_stop_anything(agent));
    agree(agent);
    agent.session.bg_tasks.clear();
    agent
        .session
        .bg_tasks
        .insert("bg-1".into(), super::make_bg_task("bg-1"));
    assert!(DashboardStopPlan::would_stop_anything(agent));
    agree(agent);
    agent.session.bg_tasks.clear();
    agent.session.scheduled_tasks.insert(
        "loop-1".into(),
        crate::app::agent::ScheduledTaskInfo {
            task_id: "loop-1".into(),
            prompt: "keep going".into(),
            human_schedule: "every 5m".into(),
            created_at: std::time::Instant::now(),
            next_fire_at: None,
            tag: "loop".into(),
            last_subagent_id: None,
        },
    );
    assert!(DashboardStopPlan::would_stop_anything(agent));
    agree(agent);
    agent.session.scheduled_tasks.clear();
    agent.session.session_id = None;
    agent
        .session
        .pending_prompts
        .push_back(crate::app::agent::QueuedPrompt::plain(
            1,
            "later",
            crate::app::agent::QueueEntryKind::Prompt,
        ));
    assert!(DashboardStopPlan::would_stop_anything(agent));
    agree(agent);
    agent.session.pending_prompts.clear();
    agent
        .session
        .bg_tasks
        .insert("bg-2".into(), super::make_bg_task("bg-2"));
    assert!(!DashboardStopPlan::would_stop_anything(agent));
    agree(agent);
}
