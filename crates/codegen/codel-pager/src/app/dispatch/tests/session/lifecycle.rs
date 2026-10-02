//! Tests for session create, exit, trust, startup actions, worktree creation, and cloud lifecycle.
use super::*;
fn expect_agent(app: &AppView, id: AgentId) -> &AgentView {
    let Some(agent) = app.agents.get(&id) else {
        panic!("expected agent {id:?}");
    };
    agent
}
use crate::app::dispatch::session::lifecycle::dispatch_accept_consent;
/// Simulate a release-stamped build so folder-trust is active (a local/dev build auto-trusts and persists nothing).
/// Mirrors this module's raw env idiom.
fn simulate_release_build() {
    unsafe { std::env::set_var(codel_version::TEST_VERSION_ENV, "0.0.0-sim") };
}
#[test]
fn voice_final_dropped_after_recording_session_cleared() {
    let mut app = test_app_with_agent();
    let id = AgentId(0);
    app.voice_state = VoiceState::Idle;
    crate::voice::handle_voice_event(
        &mut app,
        codel_voice::VoiceEvent::UtteranceFinal {
            text: "late".into(),
        },
    );
    assert_eq!(app.agents.get(&id).unwrap().prompt.text(), "");
}
#[test]
fn voice_auto_stops_when_leaving_recording_session() {
    let mut app = test_app_with_agent();
    let id = AgentId(0);
    let (tx, mut rx) = tokio::sync::mpsc::channel(8);
    app.voice_cmd_tx = Some(tx);
    app.voice_state = VoiceState::Recording {
        hold: false,
        target: VoiceTarget::Agent(id),
        partial: Partial::None,
        route: None,
    };
    app.active_view = ActiveView::Agent(id);
    app.enforce_voice_session_bound();
    assert!(app.voice_listening());
    assert!(rx.try_recv().is_err());
    app.active_view = ActiveView::AgentDashboard;
    app.enforce_voice_session_bound();
    assert!(!app.voice_listening(), "must stop when leaving the session");
    assert!(app.voice_interim().is_none());
    assert!(
        app.voice_recording_target().is_none(),
        "target dropped on leave"
    );
    assert!(matches!(
        rx.try_recv(),
        Ok(codel_voice::VoiceCommand::Abort)
    ));
}
#[test]
fn chip_submit_without_session_keeps_chips_and_does_not_send() {
    let mut app = test_app_with_agent();
    let id = AgentId(0);
    {
        let agent = app.agents.get_mut(&id).unwrap();
        agent.session.session_id = None;
        agent.apply_follow_ups("resp-1".into(), vec!["Summarize".into()]);
        assert!(agent.follow_ups.is_some(), "precondition: chips shown");
    }
    let effects = dispatch(Action::SubmitFollowUp("Summarize".into()), &mut app);
    assert!(
        !effects
            .iter()
            .any(|e| matches!(e, Effect::SendPrompt { .. })),
        "an unbound-session submit must emit no SendPrompt, got {effects:?}"
    );
    assert!(
        expect_agent(&app, id).follow_ups.is_some(),
        "an unbound-session submit must NOT clear the chips"
    );
}
#[test]
fn send_prompt_without_session_queues_but_no_effect() {
    let mut app = test_app_with_agent();
    let id = AgentId(0);
    app.agents.get_mut(&id).unwrap().session.session_id = None;
    let effects = dispatch(Action::SendPrompt("hello".into()), &mut app);
    assert!(effects.is_empty());
    assert_eq!(expect_agent(&app, id).session.queue_len(), 1);
    assert_eq!(
        expect_agent(&app, id)
            .session
            .pending_prompts
            .front()
            .map(|p| p.text.as_str()),
        Some("hello")
    );
}
#[test]
fn session_created_omits_cta_catalog_when_disabled() {
    let mut app = test_app_with_agent();
    assert!(!app.plugin_cta_enabled);
    let id = AgentId(0);
    app.agents.get_mut(&id).unwrap().session.session_id = None;
    let effects = dispatch(
        Action::TaskComplete(TaskResult::SessionCreated {
            agent_id: id,
            session_id: "new-session-123".into(),
            models: None,
            modes: None,
        }),
        &mut app,
    );
    assert!(
        !effects
            .iter()
            .any(|e| matches!(e, Effect::FetchPluginCtaCatalog { .. }))
    );
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::CheckMarketplaceUpdates { .. }))
    );
}
/// All System-block texts in an agent's scrollback, in order.
fn all_system_texts(app: &AppView, id: AgentId) -> Vec<String> {
    let sb = &expect_agent(app, id).scrollback;
    (0..sb.len())
        .filter_map(|i| match &sb.get(i).expect("index in range").block {
            RenderBlock::System(sys) => Some(sys.text.clone()),
            _ => None,
        })
        .collect()
}
#[test]
fn global_cancel_subagents_pref_skips_panel_without_session_override() {
    let mut app = test_app_with_agent();
    let id = AgentId(0);
    {
        let agent = app.agents.get_mut(&id).unwrap();
        agent.session.state = AgentState::TurnRunning;
        agent
            .subagent_sessions
            .insert("child-1".into(), make_test_subagent("child-1", "sa-1"));
    }
    app.current_ui.cancel_subagents_on_turn_cancel = Some("always_continue".into());
    let effects = dispatch(Action::CancelTurn, &mut app);
    assert!(expect_agent(&app, id).cancel_turn_view.is_none());
    assert!(matches!(
        effects.as_slice(),
        [Effect::CancelTurn {
            cancel_subagents: false,
            ..
        }]
    ));
}
#[test]
fn session_created_with_flag_emits_five_fetches_and_clears_flag() {
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
        Action::TaskComplete(TaskResult::SessionCreated {
            agent_id: id,
            session_id: acp::SessionId::new("s"),
            models: None,
            modes: None,
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
fn session_created_without_flag_emits_no_extension_fetches() {
    let mut app = test_app_with_agent();
    let id = AgentId(0);
    app.agents.get_mut(&id).unwrap().session.session_id = None;
    assert!(!expect_agent(&app, id).pending_extensions_fetch);
    let effects = dispatch(
        Action::TaskComplete(TaskResult::SessionCreated {
            agent_id: id,
            session_id: acp::SessionId::new("s"),
            models: None,
            modes: None,
        }),
        &mut app,
    );
    assert_eq!(count_extension_fetches(&effects), 0);
}
#[test]
fn session_failed_keeps_agent_clears_loading_and_toasts() {
    let mut app = test_app_with_agent();
    let id = AgentId(0);
    {
        let a = app.agents.get_mut(&id).unwrap();
        a.session.session_id = Some(acp::SessionId::new("existing"));
        a.pending_extensions_fetch = true;
        a.session_starting_since = Some(std::time::Instant::now());
    }
    let effects = dispatch(
        Action::TaskComplete(TaskResult::SessionFailed {
            agent_id: id,
            error: "No space left on device".to_string(),
            timed_out: false,
        }),
        &mut app,
    );
    assert!(effects.is_empty());
    let agent = &expect_agent(&app, id);
    assert!(!agent.pending_extensions_fetch);
    assert!(agent.session_starting_since.is_none());
    assert_eq!(
        agent.toast.as_ref().map(|(m, _)| m.as_str()),
        Some("Session creation failed: No space left on device"),
    );
}
#[test]
fn session_failed_names_step_only_on_timeout() {
    for (timed_out, error, expected) in [
        (
            true,
            "raw wire timeout text",
            "Couldn't start the session: it timed out while loading your plugins. It may still \
             finish in the background, so give it a moment before trying again.",
        ),
        (
            false,
            "No space left on device",
            "Session creation failed: No space left on device",
        ),
    ] {
        let mut app = test_app_with_agent();
        let id = AgentId(0);
        {
            let a = app.agents.get_mut(&id).unwrap();
            a.session.session_id = Some(acp::SessionId::new("existing"));
            a.session_starting_since = Some(std::time::Instant::now());
            a.session_new_phase = Some(codel_shell::agent::SessionSetupPhase::PluginRegistry);
        }
        dispatch(
            Action::TaskComplete(TaskResult::SessionFailed {
                agent_id: id,
                error: error.to_string(),
                timed_out,
            }),
            &mut app,
        );
        assert_eq!(
            Some(expected),
            expect_agent(&app, id)
                .toast
                .as_ref()
                .map(|(m, _)| m.as_str()),
        );
    }
}
#[test]
fn session_failed_orphan_returns_to_welcome_with_warning() {
    let mut app = test_app_with_agent();
    let id = AgentId(0);
    {
        let a = app.agents.get_mut(&id).unwrap();
        a.session.session_id = None;
        a.session.forked_from = None;
        a.session_starting_since = Some(std::time::Instant::now());
    }
    let effects = dispatch(
        Action::TaskComplete(TaskResult::SessionFailed {
            agent_id: id,
            error: "No space left on device".to_string(),
            timed_out: false,
        }),
        &mut app,
    );
    assert!(effects.is_empty());
    assert!(!app.agents.contains_key(&id));
    assert!(matches!(app.active_view, ActiveView::Welcome));
    assert!(
        app.startup_warnings
            .iter()
            .any(|w| { w.message == "Session creation failed: No space left on device" })
    );
}
#[test]
fn switch_model_without_session_sends_nothing_to_server() {
    let mut app = test_app_with_agent();
    let id = AgentId(0);
    app.agents.get_mut(&id).unwrap().session.session_id = None;
    let model_id = acp::ModelId::new(std::sync::Arc::from("codel-4.5"));
    let effects = dispatch(Action::SwitchModel(ModelChoice::new(model_id)), &mut app);
    assert!(
        !effects
            .iter()
            .any(|e| matches!(e, Effect::SwitchModel { .. }))
    );
    assert!(!expect_agent(&app, id).session.model_switch_pending);
}
#[test]
fn agent_type_mismatch_start_new_creates_session_with_model_id() {
    let mut app = test_app_with_agent();
    let model_id = acp::ModelId::new(std::sync::Arc::from("cursor-model"));
    let effects = dispatch(
        Action::AgentTypeMismatchAnswered {
            start_new: true,
            model_id: model_id.clone(),
            effort: None,
        },
        &mut app,
    );
    let create = effects
        .iter()
        .find(|e| matches!(e, Effect::CreateSession { .. }));
    assert!(create.is_some(), "expected CreateSession effect");
    match create.unwrap() {
        Effect::CreateSession { model_id: mid, .. } => {
            assert_eq!(
                mid.as_ref(),
                Some(&model_id),
                "CreateSession must carry the target model_id",
            );
        }
        _ => unreachable!(),
    }
    if let ActiveView::Agent(new_aid) = app.active_view {
        let agent = &expect_agent(&app, new_aid);
        assert!(
            agent.session.deferred_model_switch.is_none(),
            "no-effort mismatch must not set deferred_model_switch",
        );
    } else {
        panic!("expected active view to be an Agent");
    }
}
#[test]
fn exit_session_unregisters_active_session() {
    let mut app = test_app_with_agent();
    let effects = dispatch(Action::ExitSession, &mut app);
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::UnregisterActiveSession { .. })),
        "ExitSession must emit UnregisterActiveSession, got: {effects:?}"
    );
    assert!(matches!(app.active_view, ActiveView::Welcome));
}
/// Minimal has no welcome chrome; /exit must open an empty session like startup.
#[test]
fn exit_session_minimal_opens_new_session() {
    let mut app = test_app_with_agent();
    app.screen_mode = crate::app::ScreenMode::Minimal;
    let effects = dispatch(Action::ExitSession, &mut app);
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::UnregisterActiveSession { .. })),
        "ExitSession must emit UnregisterActiveSession, got: {effects:?}"
    );
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::CreateSession { .. })),
        "minimal /exit must dispatch NewSession, got {effects:?}"
    );
    let ActiveView::Agent(id) = app.active_view else {
        panic!(
            "minimal /exit must land on an agent, got {:?}",
            app.active_view
        );
    };
    assert_ne!(id, AgentId(0), "must not stay on the exited agent");
    assert_eq!(expect_agent(&app, id).active_pane, ActivePane::Prompt);
}
#[test]
fn slash_new_dispatches_new_session() {
    let mut app = test_app_with_agent();
    let effects = dispatch(Action::SendPrompt("/new".into()), &mut app);
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::CreateSession { .. }))
    );
}
#[test]
fn switch_model_deferred_when_no_session_id() {
    let mut app = test_app_with_agent();
    let id = AgentId(0);
    let model_id = acp::ModelId::new(std::sync::Arc::from("codel-4.5"));
    app.agents.get_mut(&id).unwrap().session.session_id = None;
    let effects = dispatch(
        Action::SwitchModel(ModelChoice::new(model_id.clone())),
        &mut app,
    );
    assert!(
        matches!(
            effects.as_slice(),
            [Effect::PersistPreferredModel { model_id: m, .. }] if m == &model_id
        ),
        "expected persist-only, got {effects:?}"
    );
    assert_eq!(
        expect_agent(&app, id).session.models.current,
        Some(model_id.clone())
    );
    assert_eq!(
        expect_agent(&app, id).session.deferred_model_switch,
        Some(crate::app::agent::DeferredModelSwitch {
            model_id,
            effort: None,
            prev_model_id: None,
        })
    );
    assert!(!expect_agent(&app, id).session.model_switch_pending);
}
#[test]
fn deferred_switch_threads_stash_prev_into_effect() {
    let mut app = test_app_with_agent();
    let id = AgentId(0);
    let model_a = acp::ModelId::new(std::sync::Arc::from("model-a"));
    let model_b = acp::ModelId::new(std::sync::Arc::from("model-b"));
    let agent = app.agents.get_mut(&id).unwrap();
    agent.session.session_id = None;
    agent.session.models.current = Some(model_a.clone());
    dispatch(
        Action::SwitchModel(ModelChoice::new(model_b.clone())),
        &mut app,
    );
    let effects = dispatch(
        Action::TaskComplete(TaskResult::SessionCreated {
            agent_id: id,
            session_id: "prev-session".into(),
            models: None,
            modes: None,
        }),
        &mut app,
    );
    assert!(effects.iter().any(|e| matches!(
        e,
        Effect::SwitchModel { choice, prev_model_id, .. }
            if choice.model_id == model_b && *prev_model_id == Some(model_a.clone())
    )));
}
#[test]
fn deferred_switch_prefers_authoritative_current_as_prev() {
    let mut app = test_app_with_agent();
    let id = AgentId(0);
    let model_b = acp::ModelId::new(std::sync::Arc::from("model-b"));
    let server_model = acp::ModelId::new(std::sync::Arc::from("server-model"));
    let agent = app.agents.get_mut(&id).unwrap();
    agent.session.session_id = None;
    agent.session.deferred_model_switch = Some(crate::app::agent::DeferredModelSwitch {
        model_id: model_b.clone(),
        effort: None,
        prev_model_id: None,
    });
    agent.session.models.current = Some(server_model.clone());
    let effects = dispatch(
        Action::TaskComplete(TaskResult::SessionCreated {
            agent_id: id,
            session_id: "auth-session".into(),
            models: None,
            modes: None,
        }),
        &mut app,
    );
    assert!(effects.iter().any(|e| matches!(
        e,
        Effect::SwitchModel { model_id, prev_model_id, .. }
            if *model_id == model_b && *prev_model_id == Some(server_model.clone())
    )));
}
#[test]
fn deferred_model_switch_applied_on_session_created() {
    let mut app = test_app_with_agent();
    let id = AgentId(0);
    let model_id = acp::ModelId::new(std::sync::Arc::from("codel-4.5"));
    let session_id: acp::SessionId = "new-session".into();
    app.agents.get_mut(&id).unwrap().session.session_id = None;
    app.agents
        .get_mut(&id)
        .unwrap()
        .session
        .deferred_model_switch = Some(crate::app::agent::DeferredModelSwitch {
        model_id: model_id.clone(),
        effort: None,
        prev_model_id: None,
    });
    let effects = dispatch(
        Action::TaskComplete(TaskResult::SessionCreated {
            agent_id: id,
            session_id: session_id.clone(),
            models: None,
            modes: None,
        }),
        &mut app,
    );
    assert!(
        expect_agent(&app, id)
            .session
            .deferred_model_switch
            .is_none()
    );
    assert!(expect_agent(&app, id).session.model_switch_pending);
    assert!(effects.iter().any(|e| matches!(
        e,
        Effect::SwitchModel {
            agent_id: a_id,
            session_id: s_id,
            model_id: m_id,
            .. } if *a_id == id && *s_id == session_id && *m_id == model_id
    )));
}
fn painted_notice(id: &str, version: i32) -> crate::app::consent::ConsentState {
    use crate::app::consent::{ConsentLegibility, ConsentNotice, ConsentSegment, ConsentState};
    ConsentState::Pending {
        notice: ConsentNotice {
            id: id.to_string(),
            version,
            title: "Updated terms".to_string(),
            segments: vec![ConsentSegment::Text("Review them.".to_string())],
            links: Vec::new(),
            accept_label: "Got it".to_string(),
        },
        legibility: ConsentLegibility::Painted,
        painted_at: Some(std::time::Instant::now()),
    }
}
#[test]
fn trust_gate_outcome_maps_every_case() {
    use crate::app::dispatch::session::lifecycle::{
        AgentLocation, TrustGateOutcome, trust_gate_outcome,
    };
    use codel_workspace::folder_trust::GrantResolution;
    let cases = [
        (
            GrantResolution::Trusted,
            AgentLocation::Embedded,
            TrustGateOutcome::Finish,
        ),
        (
            GrantResolution::Trusted,
            AgentLocation::Leader,
            TrustGateOutcome::Finish,
        ),
        (
            GrantResolution::SessionLocal,
            AgentLocation::Embedded,
            TrustGateOutcome::FinishSessionLocal,
        ),
        (
            GrantResolution::SessionLocal,
            AgentLocation::Leader,
            TrustGateOutcome::Quit,
        ),
        (
            GrantResolution::Unrecorded,
            AgentLocation::Embedded,
            TrustGateOutcome::Quit,
        ),
        (
            GrantResolution::Unrecorded,
            AgentLocation::Leader,
            TrustGateOutcome::Quit,
        ),
    ];
    for (resolution, agent, want) in cases {
        assert_eq!(want, trust_gate_outcome(resolution, agent));
    }
}
#[test]
fn dispatch_new_session_answered_with_persist_never_updates_mode_and_emits_effect() {
    let mut app = new_session_test_app();
    app.new_session_worktree_mode = crate::app::app_view::WorktreeMode::Ask;
    let effects = dispatch(
        Action::NewSessionAnswered {
            worktree: false,
            persist_mode: Some(crate::app::app_view::WorktreeMode::Never),
        },
        &mut app,
    );
    assert_eq!(
        app.new_session_worktree_mode,
        crate::app::app_view::WorktreeMode::Never
    );
    assert!(
        effects.iter().any(|e| matches!(
            e,
            Effect::PersistWorktreeMode {
                config_key: "new_session_worktree_mode",
                ..
            }
        )),
        "expected PersistWorktreeMode with config_key new_session_worktree_mode"
    );
}
#[test]
fn dispatch_new_session_has_empty_scrollback() {
    let mut app = test_app_with_agent();
    dispatch(Action::NewSession, &mut app);
    let new_id = AgentId(1);
    assert_eq!(expect_agent(&app, new_id).scrollback.len(), 0);
}
/// Dashboard attach follows the new session after `/new`.
#[test]
fn dispatch_new_session_repoints_dashboard_attached_agent() {
    use crate::views::dashboard::DashboardRowId;
    let mut app = test_app_with_agent();
    ensure_dashboard_state(&mut app);
    app.dashboard.as_mut().unwrap().attached_agent = Some(AgentId(0));
    app.active_view = ActiveView::Agent(AgentId(0));
    dispatch(Action::NewSession, &mut app);
    assert!(
        matches!(app.active_view, ActiveView::Agent(id) if id == AgentId(1)),
        "new session must switch active view to the new agent"
    );
    let d = app.dashboard.as_ref().unwrap();
    assert_eq!(
        d.attached_agent,
        Some(AgentId(1)),
        "attached_agent must re-point after /new so overlay back-out keeps working",
    );
    assert_eq!(
        d.selected,
        Some(DashboardRowId::TopLevel(AgentId(1))),
        "focus_row must move selection to the new agent row",
    );
}
/// Failed `/new` restores overlay attach to the survivor (not a dead placeholder).
#[test]
fn session_failed_orphan_restores_dashboard_attach_to_survivor() {
    use crate::views::dashboard::DashboardRowId;
    let mut app = test_app_with_agent();
    ensure_dashboard_state(&mut app);
    app.dashboard.as_mut().unwrap().attached_agent = Some(AgentId(0));
    app.active_view = ActiveView::Agent(AgentId(0));
    dispatch(Action::NewSession, &mut app);
    let fail_id = AgentId(1);
    assert!(
        matches!(app.active_view, ActiveView::Agent(id) if id == fail_id),
        "precondition: /new activated the orphan"
    );
    assert_eq!(
        app.dashboard.as_ref().unwrap().attached_agent,
        Some(fail_id),
        "precondition: attach followed /new onto the orphan"
    );
    assert!(
        expect_agent(&app, fail_id).session.session_id.is_none(),
        "precondition: create has not completed"
    );
    dispatch(
        Action::TaskComplete(TaskResult::SessionFailed {
            agent_id: fail_id,
            error: "No space left on device".to_string(),
            timed_out: false,
        }),
        &mut app,
    );
    assert!(!app.agents.contains_key(&fail_id));
    assert!(
        matches!(app.active_view, ActiveView::Agent(id) if id == AgentId(0)),
        "view recovery returns to the survivor"
    );
    let d = app.dashboard.as_ref().unwrap();
    assert_eq!(
        d.attached_agent,
        Some(AgentId(0)),
        "attach must follow back to the survivor so overlay back-out keeps working",
    );
    assert_eq!(
        d.selected,
        Some(DashboardRowId::TopLevel(AgentId(0))),
        "row focus must follow attach to the survivor",
    );
}
/// Last-session orphan failure clears overlay attach when returning to Welcome.
#[test]
fn session_failed_last_orphan_clears_dashboard_attach() {
    let mut app = test_app_with_agent();
    ensure_dashboard_state(&mut app);
    app.dashboard.as_mut().unwrap().attached_agent = Some(AgentId(0));
    app.active_view = ActiveView::Agent(AgentId(0));
    {
        let a = app.agents.get_mut(&AgentId(0)).unwrap();
        a.session.session_id = None;
        a.session.forked_from = None;
    }
    dispatch(
        Action::TaskComplete(TaskResult::SessionFailed {
            agent_id: AgentId(0),
            error: "No space left on device".to_string(),
            timed_out: false,
        }),
        &mut app,
    );
    assert!(app.agents.is_empty());
    assert!(matches!(app.active_view, ActiveView::Welcome));
    assert_eq!(
        app.dashboard.as_ref().unwrap().attached_agent,
        None,
        "Welcome recovery must clear overlay attach",
    );
}
/// `/new` must not invent dashboard attach when none was set.
#[test]
fn dispatch_new_session_without_dashboard_attach_leaves_attached_none() {
    let mut app = test_app_with_agent();
    ensure_dashboard_state(&mut app);
    assert!(app.dashboard.as_ref().unwrap().attached_agent.is_none());
    dispatch(Action::NewSession, &mut app);
    assert_eq!(
        app.dashboard.as_ref().unwrap().attached_agent,
        None,
        "/new must not enable overlay chrome when the prior session was not attached",
    );
}
#[test]
fn dispatch_new_session_keeps_stale_attach_on_other_agent() {
    let mut app = test_app_with_agent();
    insert_placeholder_agent(&mut app, AgentId(1));
    app.next_agent_id = 2;
    app.active_view = ActiveView::Agent(AgentId(0));
    ensure_dashboard_state(&mut app);
    app.dashboard.as_mut().unwrap().attached_agent = Some(AgentId(1));
    dispatch(Action::NewSession, &mut app);
    assert!(
        matches!(app.active_view, ActiveView::Agent(id) if id == AgentId(2)),
        "new session must switch active view to the new agent"
    );
    assert_eq!(
        app.dashboard.as_ref().unwrap().attached_agent,
        Some(AgentId(1)),
        "attach on a different agent must not be re-pointed to the new session",
    );
}
/// Worktree Always `/new` must re-point attach the same way as plain `/new`.
#[test]
fn dispatch_new_worktree_session_repoints_dashboard_attached_agent() {
    use crate::views::dashboard::DashboardRowId;
    let mut app = test_app_with_agent();
    app.cwd_has_git_ancestor = true;
    ensure_dashboard_state(&mut app);
    app.dashboard.as_mut().unwrap().attached_agent = Some(AgentId(0));
    app.active_view = ActiveView::Agent(AgentId(0));
    dispatch(
        Action::NewWorktreeSession {
            load_session_id: None,
            label: None,
            git_ref: None,
        },
        &mut app,
    );
    assert!(
        matches!(app.active_view, ActiveView::Agent(id) if id == AgentId(1)),
        "worktree /new must switch active view to the new agent"
    );
    let d = app.dashboard.as_ref().unwrap();
    assert_eq!(
        d.attached_agent,
        Some(AgentId(1)),
        "attached_agent must re-point after worktree /new so overlay back-out keeps working",
    );
    assert_eq!(
        d.selected,
        Some(DashboardRowId::TopLevel(AgentId(1))),
        "focus_row must move selection to the new agent row",
    );
}
#[test]
fn translate_local_submit_always_returns_persist_always_for_new_session() {
    use crate::views::question_view::{LocalQuestionKind, QuestionViewState};
    use codel_tools::implementations::codel_build::ask_user_question::{
        Question, QuestionOption,
    };
    let q = Question {
        question: "?".into(),
        options: (0..4)
            .map(|i| QuestionOption {
                label: format!("opt{i}"),
                description: String::new(),
                preview: None,
                id: None,
            })
            .collect(),
        multi_select: Some(false),
        id: None,
    };
    let mut state = QuestionViewState::new(
        "x".into(),
        vec![q],
        crate::views::prompt_widget::StashedPrompt::default(),
    )
    .with_local_kind(LocalQuestionKind::NewSession);
    let Some(slot) = state.selections.get_mut(0) else {
        panic!("expected a selection slot: {:?}", state.selections);
    };
    *slot = crate::views::question_view::QuestionSelection::Single(Some(2));
    let kind = state.local_kind.take().unwrap();
    let outcome = crate::app::agent_view::translate_local_submit_for_test(&state, kind, false);
    match outcome {
        crate::app::app_view::InputOutcome::Action(Action::NewSessionAnswered {
            worktree,
            persist_mode,
        }) => {
            assert!(worktree);
            assert_eq!(
                persist_mode,
                Some(crate::app::app_view::WorktreeMode::Always)
            );
        }
        other => panic!("expected NewSessionAnswered with persist Always, got {other:?}"),
    }
}
#[test]
fn translate_local_submit_never_returns_persist_never_for_new_session() {
    use crate::views::question_view::{LocalQuestionKind, QuestionViewState};
    use codel_tools::implementations::codel_build::ask_user_question::{
        Question, QuestionOption,
    };
    let q = Question {
        question: "?".into(),
        options: (0..4)
            .map(|i| QuestionOption {
                label: format!("opt{i}"),
                description: String::new(),
                preview: None,
                id: None,
            })
            .collect(),
        multi_select: Some(false),
        id: None,
    };
    let mut state = QuestionViewState::new(
        "x".into(),
        vec![q],
        crate::views::prompt_widget::StashedPrompt::default(),
    )
    .with_local_kind(LocalQuestionKind::NewSession);
    let Some(slot) = state.selections.get_mut(0) else {
        panic!("expected a selection slot: {:?}", state.selections);
    };
    *slot = crate::views::question_view::QuestionSelection::Single(Some(3));
    let kind = state.local_kind.take().unwrap();
    let outcome = crate::app::agent_view::translate_local_submit_for_test(&state, kind, false);
    match outcome {
        crate::app::app_view::InputOutcome::Action(Action::NewSessionAnswered {
            worktree,
            persist_mode,
        }) => {
            assert!(!worktree);
            assert_eq!(
                persist_mode,
                Some(crate::app::app_view::WorktreeMode::Never)
            );
        }
        other => panic!("expected NewSessionAnswered with persist Never, got {other:?}"),
    }
}
#[test]
fn delete_session_action_emits_delete_effect() {
    use crate::app::actions::AfterSessionDelete;
    let mut app = test_app_with_agent();
    open_session_picker_with(&mut app, vec![make_picker_entry("s1", "/repo")]);
    let effects = dispatch(
        Action::DeleteSession {
            source: "local".into(),
            session_id: "s1".into(),
            cwd: "/repo".into(),
        },
        &mut app,
    );
    assert!(matches!(
        effects.as_slice(),
        [Effect::DeleteSession {
            source,
            session_id,
            cwd,
            after: AfterSessionDelete::Stay,
        }] if source == "local" && session_id == "s1" && cwd == "/repo"
    ));
}
#[test]
fn delete_current_session_confirm_emits_effect() {
    use crate::app::actions::AfterSessionDelete;
    let mut app = test_app_with_agent();
    {
        let a = app.agents.get_mut(&AgentId(0)).unwrap();
        a.session.session_id = Some(acp::SessionId::new("sess-current"));
        a.session.cwd = std::path::PathBuf::from("/repo");
    }
    assert!(dispatch(Action::DeleteCurrentSession, &mut app).is_empty());
    assert!(matches!(
        expect_agent(&app, AgentId(0))
            .question_view
            .as_ref()
            .unwrap()
            .local_kind,
        Some(crate::views::question_view::LocalQuestionKind::DeleteCurrentSession)
    ));
    assert_eq!(
        expect_agent(&app, AgentId(0))
            .question_view
            .as_ref()
            .and_then(|qv| qv.questions.first())
            .and_then(|q| q.options.first())
            .map(|o| o.description.as_str()),
        Some("Remove history and return home")
    );
    assert!(
        dispatch(
            Action::DeleteCurrentSessionAnswered { confirmed: false },
            &mut app,
        )
        .is_empty()
    );
    let effects = dispatch(
        Action::DeleteCurrentSessionAnswered { confirmed: true },
        &mut app,
    );
    assert!(
        matches!(
            effects.first(),
            Some(Effect::CancelTurn {
                cancel_subagents: true,
                ..
            })
        ),
        "must cancel the turn/subagents before delete, got {effects:?}"
    );
    assert!(
        matches!(
            effects.last(),
            Some(Effect::DeleteSession {
                session_id,
                after: AfterSessionDelete::Welcome,
                ..
            }) if session_id == "sess-current"
        ),
        "got {effects:?}"
    );
}
/// Session delete must kill background tasks as `Teardown`; the wire default (`ClientUi`) would auto-wake.
#[test]
fn delete_current_session_kills_bg_tasks_as_teardown() {
    use codel_shell::extensions::task::TaskKillSource;
    let mut app = test_app_with_agent();
    {
        let a = app.agents.get_mut(&AgentId(0)).unwrap();
        a.session.session_id = Some(acp::SessionId::new("sess-del"));
        a.session.cwd = std::path::PathBuf::from("/repo");
        a.session
            .bg_tasks
            .insert("bg-del".into(), super::super::make_bg_task("bg-del"));
    }
    assert!(dispatch(Action::DeleteCurrentSession, &mut app).is_empty());
    let effects = dispatch(
        Action::DeleteCurrentSessionAnswered { confirmed: true },
        &mut app,
    );
    assert!(
        effects.iter().any(|e| matches!(
            e,
            Effect::KillBgTask {
                task_id,
                source: TaskKillSource::Teardown,
                ..
            } if task_id == "bg-del"
        )),
        "session delete must emit Teardown, got {effects:?}"
    );
    assert!(
        !effects.iter().any(|e| matches!(
            e,
            Effect::KillBgTask {
                source: TaskKillSource::ClientUi,
                ..
            }
        )),
        "session delete must not emit ClientUi, got {effects:?}"
    );
}
#[test]
fn delete_current_session_confirm_from_dashboard_emits_dashboard_after() {
    use crate::app::actions::AfterSessionDelete;
    let mut app = test_app_with_agent();
    {
        let a = app.agents.get_mut(&AgentId(0)).unwrap();
        a.session.session_id = Some(acp::SessionId::new("sess-dashboard"));
        a.session.cwd = std::path::PathBuf::from("/repo");
    }
    ensure_dashboard_state(&mut app);
    app.dashboard.as_mut().unwrap().attached_agent = Some(AgentId(0));
    assert!(dispatch(Action::DeleteCurrentSession, &mut app).is_empty());
    assert_eq!(
        expect_agent(&app, AgentId(0))
            .question_view
            .as_ref()
            .and_then(|qv| qv.questions.first())
            .and_then(|q| q.options.first())
            .map(|o| o.description.as_str()),
        Some("Remove history and return to the dashboard")
    );
    let effects = dispatch(
        Action::DeleteCurrentSessionAnswered { confirmed: true },
        &mut app,
    );
    assert!(
        matches!(
            effects.first(),
            Some(Effect::CancelTurn {
                cancel_subagents: true,
                ..
            })
        ),
        "must cancel the turn/subagents before delete, got {effects:?}"
    );
    assert!(
        matches!(
            effects.last(),
            Some(Effect::DeleteSession {
                session_id,
                after: AfterSessionDelete::Dashboard,
                ..
            }) if session_id == "sess-dashboard"
        ),
        "got {effects:?}"
    );
}
#[test]
fn delete_current_session_refuses_known_read_only_workspace_member() {
    let mut app = test_app_with_agent();
    app.workspace_dashboard_enabled = true;
    let temp = tempfile::tempdir().unwrap();
    let store =
        codel_dashboard_store::WorkspaceStore::open(&temp.path().join("workspace.db")).unwrap();
    app.workspace_membership.set_read_only_for_test(
        store,
        codel_dashboard_store::WorkspaceSnapshot {
            grouping: codel_dashboard_store::Grouping::State,
            members: vec![codel_dashboard_store::Member {
                session_id: codel_dashboard_store::SessionId::new("test-session").unwrap(),
                kind: codel_dashboard_store::MemberKind::Build,
                origin: codel_dashboard_store::MemberOrigin::Local,
                cwd: Some("/tmp".into()),
                title: Some("Read only".into()),
                model: None,
                last_turn_summary: None,
                is_worktree: false,
                last_change_unix_ms: 1,
                pin_rank: None,
                order_rank: None,
            }],
            data_version: 1,
        },
    );
    let effects = dispatch(
        Action::DeleteCurrentSessionAnswered { confirmed: true },
        &mut app,
    );
    assert!(effects.is_empty());
    assert!(app.agents.contains_key(&AgentId(0)));
    assert!(read_toast(&app).contains("workspace is read-only"));
}
/// Dashboard state can exist without overlay attach; the delete must still land on Welcome.
#[test]
fn delete_current_session_dashboard_state_without_attach_stays_welcome() {
    use crate::app::actions::AfterSessionDelete;
    let mut app = test_app_with_agent();
    {
        let a = app.agents.get_mut(&AgentId(0)).unwrap();
        a.session.session_id = Some(acp::SessionId::new("sess-no-attach"));
        a.session.cwd = std::path::PathBuf::from("/repo");
    }
    ensure_dashboard_state(&mut app);
    assert!(app.dashboard.as_ref().unwrap().attached_agent.is_none());
    assert!(dispatch(Action::DeleteCurrentSession, &mut app).is_empty());
    assert_eq!(
        expect_agent(&app, AgentId(0))
            .question_view
            .as_ref()
            .and_then(|qv| qv.questions.first())
            .and_then(|q| q.options.first())
            .map(|o| o.description.as_str()),
        Some("Remove history and return home")
    );
    let effects = dispatch(
        Action::DeleteCurrentSessionAnswered { confirmed: true },
        &mut app,
    );
    assert!(
        matches!(
            effects.last(),
            Some(Effect::DeleteSession {
                after: AfterSessionDelete::Welcome,
                ..
            })
        ),
        "got {effects:?}"
    );
}
/// Minimal has no welcome chrome; /delete must open an empty session like startup.
#[test]
fn delete_current_session_complete_minimal_opens_new_session() {
    use crate::app::actions::{AfterSessionDelete, TaskResult};
    let mut app = test_app_with_agent();
    app.screen_mode = crate::app::ScreenMode::Minimal;
    app.agents.get_mut(&AgentId(0)).unwrap().session.session_id =
        Some(acp::SessionId::new("sess-a"));
    let effects = dispatch_task_result(
        TaskResult::DeleteSessionComplete {
            source: "current".into(),
            session_id: "sess-a".into(),
            after: AfterSessionDelete::Welcome,
        },
        &mut app,
    );
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::UnregisterActiveSession { .. }))
    );
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::CreateSession { .. })),
        "minimal /delete must dispatch NewSession, got {effects:?}"
    );
    let ActiveView::Agent(id) = app.active_view else {
        panic!(
            "minimal /delete must land on an agent, got {:?}",
            app.active_view
        );
    };
    assert_ne!(id, AgentId(0), "must not stay on the deleted agent");
    assert!(!app.agents.contains_key(&AgentId(0)));
    assert_eq!(expect_agent(&app, id).active_pane, ActivePane::Prompt);
}
#[test]
fn delete_current_session_complete_returns_to_dashboard() {
    use crate::app::actions::{AfterSessionDelete, TaskResult};
    let mut app = test_app_with_agent();
    app.agents.get_mut(&AgentId(0)).unwrap().session.session_id =
        Some(acp::SessionId::new("sess-dash"));
    ensure_dashboard_state(&mut app);
    app.dashboard.as_mut().unwrap().attached_agent = Some(AgentId(0));
    app.active_view = ActiveView::Agent(AgentId(0));
    let effects = dispatch_task_result(
        TaskResult::DeleteSessionComplete {
            source: "current".into(),
            session_id: "sess-dash".into(),
            after: AfterSessionDelete::Dashboard,
        },
        &mut app,
    );
    assert!(matches!(app.active_view, ActiveView::AgentDashboard));
    assert!(app.agents.is_empty());
    assert!(app.dashboard.is_some());
    assert!(
        app.dashboard.as_ref().unwrap().attached_agent.is_none(),
        "delete complete must clear attach on the removed agent"
    );
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::UnregisterActiveSession { .. }))
    );
}
#[test]
fn entry_title_falls_back_to_short_session_id_when_no_prompt() {
    use crate::views::session_title::entry_title;
    let mut app = test_app_with_agent();
    if let Some(a) = app.agents.get_mut(&AgentId(0)) {
        a.session.session_id = Some("abcdef0123456".into());
    }
    let title = entry_title(expect_agent(&app, AgentId(0)));
    assert_eq!(title, "session abcdef01");
    assert!(crate::views::session_title::named_title(expect_agent(&app, AgentId(0))).is_none());
}
#[test]
fn bg_task_killed_no_op_for_unknown_session() {
    let mut app = two_agent_app_with_bg_task();
    let effects = dispatch(
        Action::TaskComplete(TaskResult::BgTaskKilled {
            session_id: "nonexistent".into(),
            task_id: "task-B-1".into(),
            outcome: Some(codel_tools::types::KillOutcome::AlreadyExited),
        }),
        &mut app,
    );
    assert!(effects.is_empty());
    assert!(
        expect_agent(&app, AgentId(1))
            .session
            .bg_tasks
            .get("task-B-1")
            .is_some_and(|t| t.pending_kill)
    );
}
/// Mutual exclusion: enabling always-approve via dispatch clears the per-session `auto_mode` display flag (yolo wins).
#[test]
fn set_yolo_on_clears_session_auto_mode() {
    use crate::app::actions::PermissionModeKind;
    let mut app = test_app_with_agent();
    dispatch(
        Action::SetPermissionMode(PermissionModeKind::Auto),
        &mut app,
    );
    assert!(
        expect_agent(&app, AgentId(0)).session.is_auto(),
        "precondition: active agent is in auto"
    );
    dispatch(Action::SetYoloMode(true), &mut app);
    assert!(expect_agent(&app, AgentId(0)).session.is_yolo());
    assert!(
        !expect_agent(&app, AgentId(0)).session.is_auto(),
        "enabling always-approve must clear the per-session auto flag (yolo wins)"
    );
}
/// Gate OFF and policy pin: Plan exit lands on Normal (ask) but MUST still push `SetSessionMode(Default)` so the agent leaves Plan.
/// Otherwise the session stays in Plan while the UI reads Normal.
#[test]
fn cycle_mode_plan_exit_under_gate_off_pin_emits_set_session_mode() {
    let mut app = test_app_with_agent();
    app.auto_mode_gate = false;
    app.yolo_policy_block = Some(POLICY_WARNING);
    app.agents.get_mut(&AgentId(0)).unwrap().plan_mode_pending = Some(true);
    let effects = dispatch(Action::CycleMode, &mut app);
    assert!(
        !expect_agent(&app, AgentId(0)).session.is_yolo(),
        "the pin must keep yolo off when exiting Plan"
    );
    assert_eq!(app.current_ui.permission_mode.as_deref(), Some("ask"));
    assert_eq!(
        expect_agent(&app, AgentId(0)).plan_mode_pending,
        Some(false)
    );
    assert_eq!(agent_toast(&app).as_deref(), Some(POLICY_WARNING));
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::SetSessionMode { .. })),
        "Plan exit under the pin must still send SetSessionMode(Default), got {effects:?}"
    );
    assert!(
        effects.iter().any(|e| matches!(
            e,
            Effect::PersistPermissionMode {
                canonical: "ask",
                ..
            }
        )),
        "expected PersistPermissionMode(ask) under pin, got {effects:?}"
    );
}
/// Pre-session cycle (no session id yet) under the pin: Plan to Auto does not stage yolo.
/// Auto is the next mode; the pin applies on the Auto to Always-Approve step.
#[test]
fn cycle_mode_pre_session_blocked_by_policy_pin() {
    let mut app = test_app_with_agent();
    app.yolo_policy_block = Some(POLICY_WARNING);
    {
        let agent = app.agents.get_mut(&AgentId(0)).unwrap();
        agent.session.session_id = None;
        agent.plan_mode_pending = Some(true);
        agent.deferred_session_mode = Some(codel_tools::types::SessionMode::Plan);
    }
    let _ = dispatch(Action::CycleMode, &mut app);
    let agent = &expect_agent(&app, AgentId(0));
    assert!(
        !agent.session.is_yolo(),
        "pre-session Plan→Auto must not stage yolo under the pin"
    );
    assert_eq!(app.current_ui.permission_mode.as_deref(), Some("auto"));
    let _ = dispatch(Action::CycleMode, &mut app);
    let agent = &expect_agent(&app, AgentId(0));
    assert!(
        !agent.session.is_yolo(),
        "pre-session cycle must not stage yolo under the pin"
    );
    assert_eq!(agent.plan_mode_pending, Some(false));
    assert_eq!(agent.deferred_session_mode, None);
    assert_eq!(agent_toast(&app).as_deref(), Some(POLICY_WARNING));
}
/// Pre-session cycle from Plan with STALE `yolo_mode = true` (e.g. client state restored from before the pin landed) resets to Normal.
/// Yolo is cleared too (matching the with-session catch-all for the same Plan+yolo input) so always-approve is not left active behind another banner.
#[test]
fn cycle_mode_pre_session_clears_stale_yolo_under_pin() {
    let mut app = test_app_with_agent();
    app.yolo_policy_block = Some(POLICY_WARNING);
    {
        let agent = app.agents.get_mut(&AgentId(0)).unwrap();
        agent.session.session_id = None;
        agent.plan_mode_pending = Some(true);
        agent.deferred_session_mode = Some(codel_tools::types::SessionMode::Plan);
        agent.session.yolo_mode = true;
    }
    let _ = dispatch(Action::CycleMode, &mut app);
    let agent = &expect_agent(&app, AgentId(0));
    assert!(
        !agent.session.is_yolo(),
        "stale pre-session yolo must be cleared so Normal is enforced, not just displayed"
    );
    assert!(!app.default_yolo, "clamp must clear the global mirror too");
    assert_eq!(agent.plan_mode_pending, Some(false));
    assert_eq!(agent.deferred_session_mode, None);
    assert_eq!(
        app.current_ui.permission_mode.as_deref(),
        Some("ask"),
        "Plan+yolo resets to Normal (matches the with-session catch-all), not always-approve"
    );
}
/// While session creation is in flight, Shift+Tab cycles the mode locally (optimistic pending plus a deferred ACP push) and never creates a session.
#[test]
fn dispatch_cycle_mode_pre_session_cycles_locally() {
    let mut app = test_app_with_agent();
    app.agents.get_mut(&AgentId(0)).unwrap().session.session_id = None;
    let effects = dispatch(Action::CycleMode, &mut app);
    let agent = &expect_agent(&app, AgentId(0));
    assert_eq!(
        agent.plan_mode_pending,
        Some(true),
        "pre-session Normal → Plan must set optimistic pending"
    );
    assert_eq!(
        agent.deferred_session_mode,
        Some(codel_tools::types::SessionMode::Plan),
        "Plan must be deferred to SessionCreated"
    );
    assert!(
        !effects
            .iter()
            .any(|e| matches!(e, Effect::CreateSession { .. })),
        "cycling a mode must not create a session, got {effects:?}"
    );
    let effects = dispatch(Action::CycleMode, &mut app);
    let agent = &expect_agent(&app, AgentId(0));
    assert!(!agent.session.is_yolo(), "Plan → Auto must not enable yolo");
    assert_eq!(app.current_ui.permission_mode.as_deref(), Some("auto"));
    assert_eq!(agent.plan_mode_pending, Some(false));
    assert!(agent.deferred_session_mode.is_none());
    assert!(
        !effects
            .iter()
            .any(|e| matches!(e, Effect::CreateSession { .. })),
        "still no CreateSession, got {effects:?}"
    );
    assert!(
        effects.iter().any(|e| matches!(
            e,
            Effect::PersistPermissionMode {
                canonical: "auto",
                session_id: None,
                ..
            }
        )),
        "pre-session Plan → Auto must persist the displayed mode, got {effects:?}"
    );
    let _ = dispatch(Action::CycleMode, &mut app);
    let agent = &expect_agent(&app, AgentId(0));
    assert!(agent.session.is_yolo(), "Auto → Always-Approve flips yolo");
    let _ = dispatch(Action::CycleMode, &mut app);
    let agent = &expect_agent(&app, AgentId(0));
    assert!(!agent.session.is_yolo());
    assert_eq!(agent.plan_mode_pending, Some(false));
}
/// No-session bail-out: when the active agent has no ACP session, the setter toasts "No active session" and returns empty effects.
/// Pins the safety contract that we never dispatch a mode change to a non-existent session.
#[test]
fn set_plan_mode_no_session_toasts_and_bails() {
    let mut app = test_app_with_agent();
    app.agents.get_mut(&AgentId(0)).unwrap().session.session_id = None;
    let effects = dispatch(
        Action::SetPlanMode(crate::app::actions::PlanModeKind::On),
        &mut app,
    );
    assert!(
        effects.is_empty(),
        "no-session bail must NOT emit Effect (no live session to set mode on)"
    );
    let toast = read_toast(&app);
    assert!(
        toast.contains("No active session"),
        "bail-out must surface the reason to the user: {toast}",
    );
    let agent = app.agents.get(&AgentId(0)).unwrap();
    assert!(agent.plan_mode_pending.is_none());
    assert!(!agent.plan_mode_active);
}
/// Non-idempotent ON transition: emits `Effect::SetSessionMode(plan)` and sets `plan_mode_pending`.
/// Complement to the idempotent-ON test above.
#[test]
fn set_plan_mode_on_from_off_emits_set_session_mode() {
    let mut app = test_app_with_agent();
    let effects = dispatch(
        Action::SetPlanMode(crate::app::actions::PlanModeKind::On),
        &mut app,
    );
    assert_eq!(effects.len(), 1);
    assert!(
        matches!(effects.first(), Some(Effect::SetSessionMode { mode_id, .. }) if &*mode_id.0 == "plan"),
        "expected SetSessionMode(plan), got: {effects:?}"
    );
    let agent = app.agents.get(&AgentId(0)).unwrap();
    assert_eq!(
        agent.plan_mode_pending,
        Some(true),
        "optimistic pending must be set to Some(true)"
    );
}
/// Top-level resolver round-trip via real AgentView.
#[test]
fn session_id_resolver_round_trip_top_level() {
    use crate::views::dashboard::{DashboardRowId, PersistedRowId, SessionIdResolver};
    let app = test_app_with_agent();
    let resolver = SessionIdResolver::from_agents(&app.agents);
    let pid = PersistedRowId::TopLevel {
        session_id: "test-session".into(),
    };
    let live = resolver.resolve(&pid).expect("must resolve");
    assert_eq!(live, DashboardRowId::TopLevel(AgentId(0)));
    let back = resolver.to_persisted(&live).expect("must reverse");
    assert_eq!(back, pid);
    let absent = PersistedRowId::TopLevel {
        session_id: "no-such-session".into(),
    };
    assert!(resolver.resolve(&absent).is_none());
}
#[cfg(feature = "local-workspace")]
mod welcome_workspace_mode {
    use super::*;
    use crate::app::session_startup::{
        LocalWorkspaceConfig, LocalWorkspaceMode, set_active_local_workspace,
    };
    use crate::views::welcome::WelcomeWorkspaceMode;
    #[test]
    #[serial_test::serial(CODEL_CHAT_LOCAL_WORKSPACE_ACK)]
    fn welcome_new_session_sets_own_override() {
        let _ack = codel_test_support::EnvGuard::set(
            crate::app::session_startup::CODEL_CHAT_LOCAL_WORKSPACE_ACK_ENV,
            "1",
        );
        set_active_local_workspace(None).unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let mut app = test_app();
        app.chat_mode = true;
        app.active_view = ActiveView::Welcome;
        app.cwd = tmp.path().to_path_buf();
        app.welcome_workspace_mode = WelcomeWorkspaceMode::LocalWorkspace;
        let _ = dispatch(Action::NewSession, &mut app);
        let override_cfg = app
            .welcome_session_local_workspace
            .clone()
            .flatten()
            .expect("welcome Local must set one-shot own override");
        assert_eq!(override_cfg.mode, LocalWorkspaceMode::Own);
        assert_eq!(override_cfg.cwd.as_deref(), Some(tmp.path()));
        set_active_local_workspace(None).unwrap();
    }
    #[test]
    fn new_session_ignores_history_bypass_for_indicator() {
        set_active_local_workspace(None).unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let mut app = test_app();
        app.chat_mode = true;
        app.active_view = ActiveView::Welcome;
        app.cwd = tmp.path().to_path_buf();
        app.cwd_has_git_ancestor = false;
        app.welcome_workspace_mode = WelcomeWorkspaceMode::Sandbox;
        app.welcome_history_load_as_build = true;
        let effects = dispatch(Action::NewSession, &mut app);
        assert!(
            effects
                .iter()
                .any(|e| matches!(e, Effect::CreateSession { .. })),
            "new session must create: {effects:?}"
        );
        assert!(
            app.welcome_history_load_as_build,
            "create must not consume history bypass (restore+load still owns it)"
        );
        let agent = app.agents.values().next().expect("new agent");
        assert!(agent.chat_kind);
        assert_eq!(
            agent.workspace_mode,
            WelcomeWorkspaceMode::Sandbox,
            "create must not stamp Local from leftover history bypass"
        );
        set_active_local_workspace(None).unwrap();
    }
    #[test]
    fn fork_from_welcome_with_local_selection_creates_placeholder() {
        set_active_local_workspace(None).unwrap();
        let mut app = test_app();
        app.chat_mode = true;
        app.active_view = ActiveView::Welcome;
        app.welcome_workspace_mode = WelcomeWorkspaceMode::LocalWorkspace;
        assert!(app.agents.is_empty());
        let effects = crate::app::dispatch::session::fork::dispatch_startup_fork_session(
            &mut app,
            "parent-1".into(),
            None,
            None,
        );
        assert!(
            !app.agents.is_empty(),
            "fork must still create a placeholder agent"
        );
        assert!(
            effects
                .iter()
                .any(|e| matches!(e, Effect::ForkSession { .. })),
            "fork effect expected: {effects:?}"
        );
        set_active_local_workspace(None).unwrap();
    }
    #[test]
    fn startup_lock_prevents_sandbox_from_clearing_cli_stamp() {
        let tmp = tempfile::tempdir().unwrap();
        set_active_local_workspace(Some(LocalWorkspaceConfig {
            mode: LocalWorkspaceMode::Attach,
            cwd: Some(tmp.path().to_path_buf()),
            server_id: Some("cli-srv".into()),
        }))
        .unwrap();
        let mut app = test_app();
        app.chat_mode = true;
        app.active_view = ActiveView::Welcome;
        app.local_workspace_startup_locked = true;
        app.welcome_workspace_mode = WelcomeWorkspaceMode::Sandbox;
        app.cwd = tmp.path().to_path_buf();
        let _ = dispatch(Action::NewSession, &mut app);
        let stamp = crate::app::session_startup::active_local_workspace()
            .unwrap()
            .expect("CLI stamp must remain");
        assert_eq!(stamp.mode, LocalWorkspaceMode::Attach);
        assert!(
            app.welcome_session_local_workspace.is_none(),
            "locked path must not set a one-shot override"
        );
        set_active_local_workspace(None).unwrap();
    }
    #[test]
    #[serial_test::serial(CODEL_CHAT_LOCAL_WORKSPACE_ACK)]
    fn confirm_ack_skips_reapply_and_sets_oneshot() {
        let _ack = codel_test_support::EnvGuard::unset(
            crate::app::session_startup::CODEL_CHAT_LOCAL_WORKSPACE_ACK_ENV,
        );
        let home = tempfile::tempdir().unwrap();
        let _home =
            codel_test_support::EnvGuard::set("CODEL_HOME", home.path().to_str().unwrap());
        set_active_local_workspace(None).unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let mut app = test_app();
        app.chat_mode = true;
        app.active_view = ActiveView::Welcome;
        app.cwd = tmp.path().to_path_buf();
        app.welcome_workspace_mode = WelcomeWorkspaceMode::LocalWorkspace;
        app.welcome_local_workspace_ack_pending = true;
        let effects = dispatch(Action::ConfirmWelcomeLocalWorkspaceAck, &mut app);
        assert!(
            !app.welcome_local_workspace_ack_pending,
            "confirm must clear pending"
        );
        assert!(
            app.welcome_session_local_workspace
                .clone()
                .flatten()
                .is_some(),
            "one-shot Own override must be set before CreateSession"
        );
        assert!(
            effects
                .iter()
                .any(|e| matches!(e, Effect::CreateSession { .. })),
            "confirm must create without re-entering AwaitAck: {effects:?}"
        );
        set_active_local_workspace(None).unwrap();
    }
    #[test]
    #[serial_test::serial(CODEL_CHAT_LOCAL_WORKSPACE_ACK)]
    fn welcome_local_worktree_always_keeps_oneshot_until_create() {
        let _ack = codel_test_support::EnvGuard::set(
            crate::app::session_startup::CODEL_CHAT_LOCAL_WORKSPACE_ACK_ENV,
            "1",
        );
        set_active_local_workspace(None).unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let mut app = test_app();
        app.chat_mode = true;
        app.active_view = ActiveView::Welcome;
        app.cwd = tmp.path().to_path_buf();
        app.cwd_has_git_ancestor = true;
        app.new_session_worktree_mode = crate::app::app_view::WorktreeMode::Always;
        app.welcome_workspace_mode = WelcomeWorkspaceMode::LocalWorkspace;
        let live = test_app_with_agent();
        let (id, agent) = live.agents.into_iter().next().unwrap();
        app.agents.insert(id, agent);
        let effects = dispatch(Action::NewSession, &mut app);
        assert!(
            effects
                .iter()
                .any(|e| matches!(e, Effect::CreateWorktreeSession { .. })),
            "Always worktree must emit CreateWorktreeSession: {effects:?}"
        );
        assert!(
            app.welcome_session_local_workspace
                .clone()
                .flatten()
                .is_some(),
            "one-shot must remain until process_effects consumes CreateWorktreeSession"
        );
        assert!(
            crate::app::session_startup::active_local_workspace()
                .unwrap()
                .is_some(),
            "welcome Local stamps process-wide Own (agents map treated as stale)"
        );
        set_active_local_workspace(None).unwrap();
    }
    #[test]
    #[serial_test::serial(CODEL_CHAT_LOCAL_WORKSPACE_ACK)]
    fn failed_worktree_create_clears_welcome_oneshot() {
        let _ack = codel_test_support::EnvGuard::set(
            crate::app::session_startup::CODEL_CHAT_LOCAL_WORKSPACE_ACK_ENV,
            "1",
        );
        set_active_local_workspace(None).unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let mut app = test_app();
        app.chat_mode = true;
        app.active_view = ActiveView::Welcome;
        app.cwd = tmp.path().to_path_buf();
        app.cwd_has_git_ancestor = false;
        app.welcome_workspace_mode = WelcomeWorkspaceMode::LocalWorkspace;
        app.welcome_history_load_as_build = true;
        let effects = dispatch(
            Action::NewWorktreeSession {
                load_session_id: None,
                label: None,
                git_ref: None,
            },
            &mut app,
        );
        assert!(effects.is_empty(), "expected hard-fail, got {effects:?}");
        assert!(
            app.welcome_session_local_workspace.is_none(),
            "failed worktree must drop one-shot so next create re-applies picker"
        );
        assert!(
            !app.welcome_history_load_as_build,
            "failed worktree must not leak history bypass"
        );
        set_active_local_workspace(None).unwrap();
    }
    #[test]
    #[serial_test::serial(CODEL_CHAT_LOCAL_WORKSPACE_ACK)]
    fn confirm_ack_honors_worktree_always() {
        let _ack = codel_test_support::EnvGuard::unset(
            crate::app::session_startup::CODEL_CHAT_LOCAL_WORKSPACE_ACK_ENV,
        );
        let home = tempfile::tempdir().unwrap();
        let _home =
            codel_test_support::EnvGuard::set("CODEL_HOME", home.path().to_str().unwrap());
        set_active_local_workspace(None).unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let mut app = test_app();
        app.chat_mode = true;
        app.active_view = ActiveView::Welcome;
        app.cwd = tmp.path().to_path_buf();
        app.cwd_has_git_ancestor = true;
        app.new_session_worktree_mode = crate::app::app_view::WorktreeMode::Always;
        app.welcome_workspace_mode = WelcomeWorkspaceMode::LocalWorkspace;
        app.welcome_local_workspace_ack_pending = true;
        let effects = dispatch(Action::ConfirmWelcomeLocalWorkspaceAck, &mut app);
        assert!(
            effects
                .iter()
                .any(|e| matches!(e, Effect::CreateWorktreeSession { .. })),
            "confirm must honor WorktreeMode::Always: {effects:?}"
        );
        assert!(
            app.welcome_session_local_workspace
                .clone()
                .flatten()
                .is_some()
        );
        set_active_local_workspace(None).unwrap();
    }
    #[test]
    fn welcome_fetch_session_list_filters_by_workspace_mode() {
        set_active_local_workspace(None).unwrap();
        let mut app = test_app();
        app.chat_mode = true;
        app.active_view = ActiveView::Welcome;
        app.welcome_workspace_mode = WelcomeWorkspaceMode::Sandbox;
        let effects = dispatch(Action::FetchSessionList, &mut app);
        match effects.as_slice() {
            [Effect::FetchSessionList { kind_filter, .. }] => {
                assert_eq!(
                    kind_filter.as_deref(),
                    Some(["chat".to_string()].as_slice())
                );
            }
            other => panic!("expected FetchSessionList, got {other:?}"),
        }
        app.welcome_workspace_mode = WelcomeWorkspaceMode::LocalWorkspace;
        let effects = dispatch(Action::FetchSessionList, &mut app);
        match effects.as_slice() {
            [Effect::FetchSessionList { kind_filter, .. }] => {
                assert_eq!(
                    kind_filter.as_deref(),
                    Some(["build".to_string()].as_slice())
                );
            }
            other => panic!("expected FetchSessionList, got {other:?}"),
        }
        set_active_local_workspace(None).unwrap();
    }
    #[test]
    fn pick_conversation_auto_switches_to_sandbox() {
        set_active_local_workspace(None).unwrap();
        let mut app = test_app();
        app.chat_mode = true;
        app.active_view = ActiveView::Welcome;
        app.welcome_workspace_mode = WelcomeWorkspaceMode::LocalWorkspace;
        app.session_picker_entries = Some(vec![crate::app::app_view::SessionPickerEntry {
            id: "conv-1".into(),
            summary: "hello".into(),
            updated_at: chrono::Utc::now(),
            created_at: chrono::Utc::now(),
            cwd: String::new(),
            hostname: None,
            source: "conversation".into(),
            model_id: None,
            num_messages: 1,
            last_active_at: None,
            branch: None,
            repo_name: String::new(),
            worktree_label: None,
            last_turn_summary: None,
            last_recap: None,
            session_kind: None,
            card_detail: None,
        }]);
        let effects = dispatch(Action::PickSession(0), &mut app);
        assert_eq!(app.welcome_workspace_mode, WelcomeWorkspaceMode::Sandbox);
        assert!(
            app.welcome_session_local_workspace.is_none(),
            "conversation pick must drop (not force-clear) local one-shot"
        );
        assert!(
            effects.iter().any(|e| matches!(
                e,
                Effect::LoadSession {
                    chat_kind: true,
                    ..
                }
            )),
            "conversation must load as chat: {effects:?}"
        );
        assert!(!app.welcome_history_load_as_build);
        set_active_local_workspace(None).unwrap();
    }
    #[test]
    fn pick_local_disk_auto_switches_to_local_and_bypasses_chat_refusal() {
        set_active_local_workspace(None).unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let mut app = test_app();
        app.chat_mode = true;
        app.active_view = ActiveView::Welcome;
        app.cwd = tmp.path().to_path_buf();
        app.welcome_workspace_mode = WelcomeWorkspaceMode::Sandbox;
        let sess_dir = super::super::super::plant_local_build_session(tmp.path(), "build-1");
        app.session_picker_entries = Some(vec![crate::app::app_view::SessionPickerEntry {
            id: "build-1".into(),
            summary: "local work".into(),
            updated_at: chrono::Utc::now(),
            created_at: chrono::Utc::now(),
            cwd: tmp.path().display().to_string(),
            hostname: None,
            source: "local".into(),
            model_id: None,
            num_messages: 1,
            last_active_at: None,
            branch: None,
            repo_name: String::new(),
            worktree_label: None,
            last_turn_summary: None,
            last_recap: None,
            session_kind: None,
            card_detail: None,
        }]);
        let effects = dispatch(Action::PickSession(0), &mut app);
        assert_eq!(
            app.welcome_workspace_mode,
            WelcomeWorkspaceMode::LocalWorkspace
        );
        assert!(app.chat_mode, "sticky --chat remains");
        assert!(
            effects.iter().any(|e| matches!(
                e,
                Effect::LoadSession {
                    chat_kind: false,
                    ..
                }
            )),
            "local-disk pick must load as build: {effects:?}"
        );
        assert!(
            app.welcome_history_load_as_build,
            "bypass stays until process_effects LoadSession"
        );
        let agent = app.agents.values().next().expect("placeholder agent");
        assert!(
            agent.chat_kind,
            "sticky --chat keeps agent.chat_kind for already-open focus matching"
        );
        assert_eq!(
            agent.workspace_mode,
            WelcomeWorkspaceMode::LocalWorkspace,
            "Local UX is the workspace_mode indicator"
        );
        let _ = std::fs::remove_dir_all(sess_dir);
        set_active_local_workspace(None).unwrap();
    }
    #[test]
    fn pick_local_disk_in_worktree_sets_history_bypass() {
        set_active_local_workspace(None).unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let mut app = test_app();
        app.chat_mode = true;
        app.active_view = ActiveView::Welcome;
        app.cwd = tmp.path().to_path_buf();
        app.cwd_has_git_ancestor = true;
        app.welcome_workspace_mode = WelcomeWorkspaceMode::Sandbox;
        app.session_picker_entries = Some(vec![crate::app::app_view::SessionPickerEntry {
            id: "build-wt".into(),
            summary: "local work".into(),
            updated_at: chrono::Utc::now(),
            created_at: chrono::Utc::now(),
            cwd: tmp.path().display().to_string(),
            hostname: None,
            source: "local".into(),
            model_id: None,
            num_messages: 1,
            last_active_at: None,
            branch: None,
            repo_name: String::new(),
            worktree_label: None,
            last_turn_summary: None,
            last_recap: None,
            session_kind: None,
            card_detail: None,
        }]);
        let _ = dispatch(Action::PickSessionInWorktree(0), &mut app);
        assert_eq!(
            app.welcome_workspace_mode,
            WelcomeWorkspaceMode::LocalWorkspace
        );
        assert!(
            app.welcome_history_load_as_build,
            "worktree pick of build row must set history bypass"
        );
        set_active_local_workspace(None).unwrap();
    }
    #[test]
    fn pick_in_worktree_resume_skips_local_ack() {
        set_active_local_workspace(None).unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let mut app = test_app();
        app.chat_mode = true;
        app.active_view = ActiveView::Welcome;
        app.cwd = tmp.path().to_path_buf();
        app.cwd_has_git_ancestor = true;
        app.welcome_workspace_mode = WelcomeWorkspaceMode::LocalWorkspace;
        app.session_picker_entries = Some(vec![crate::app::app_view::SessionPickerEntry {
            id: "build-wt".into(),
            summary: "local work".into(),
            updated_at: chrono::Utc::now(),
            created_at: chrono::Utc::now(),
            cwd: tmp.path().display().to_string(),
            hostname: None,
            source: "local".into(),
            model_id: None,
            num_messages: 1,
            last_active_at: None,
            branch: None,
            repo_name: String::new(),
            worktree_label: None,
            last_turn_summary: None,
            last_recap: None,
            session_kind: None,
            card_detail: None,
        }]);
        let effects = dispatch(Action::PickSessionInWorktree(0), &mut app);
        assert!(
            !app.welcome_local_workspace_ack_pending,
            "worktree resume must not block on Local ACK"
        );
        assert!(
            effects
                .iter()
                .any(|e| matches!(e, Effect::CreateWorktreeSession { .. })),
            "worktree resume must create worktree without ACK: {effects:?}"
        );
        set_active_local_workspace(None).unwrap();
    }
    #[test]
    #[serial_test::serial(CODEL_CHAT_LOCAL_WORKSPACE_ACK)]
    fn pick_in_worktree_no_git_clears_history_bypass() {
        let _ack = codel_test_support::EnvGuard::set(
            crate::app::session_startup::CODEL_CHAT_LOCAL_WORKSPACE_ACK_ENV,
            "1",
        );
        set_active_local_workspace(None).unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let mut app = test_app();
        app.chat_mode = true;
        app.active_view = ActiveView::Welcome;
        app.cwd = tmp.path().to_path_buf();
        app.cwd_has_git_ancestor = false;
        app.welcome_workspace_mode = WelcomeWorkspaceMode::Sandbox;
        app.session_picker_entries = Some(vec![crate::app::app_view::SessionPickerEntry {
            id: "build-wt".into(),
            summary: "local work".into(),
            updated_at: chrono::Utc::now(),
            created_at: chrono::Utc::now(),
            cwd: tmp.path().display().to_string(),
            hostname: None,
            source: "local".into(),
            model_id: None,
            num_messages: 1,
            last_active_at: None,
            branch: None,
            repo_name: String::new(),
            worktree_label: None,
            last_turn_summary: None,
            last_recap: None,
            session_kind: None,
            card_detail: None,
        }]);
        let effects = dispatch(Action::PickSessionInWorktree(0), &mut app);
        assert!(
            effects.is_empty(),
            "no-git worktree must hard-fail: {effects:?}"
        );
        assert!(
            !app.welcome_history_load_as_build,
            "no-git worktree fail must not leak history bypass"
        );
        set_active_local_workspace(None).unwrap();
    }
    #[test]
    fn cli_lock_still_sets_history_bypass_without_rewriting_mode() {
        let tmp = tempfile::tempdir().unwrap();
        set_active_local_workspace(Some(LocalWorkspaceConfig {
            mode: LocalWorkspaceMode::Attach,
            cwd: Some(tmp.path().to_path_buf()),
            server_id: Some("cli-srv".into()),
        }))
        .unwrap();
        let mut app = test_app();
        app.chat_mode = true;
        app.active_view = ActiveView::Welcome;
        app.cwd = tmp.path().to_path_buf();
        app.local_workspace_startup_locked = true;
        app.welcome_workspace_mode = WelcomeWorkspaceMode::Sandbox;
        let sess_dir = super::super::super::plant_local_build_session(tmp.path(), "build-lock");
        app.session_picker_entries = Some(vec![crate::app::app_view::SessionPickerEntry {
            id: "build-lock".into(),
            summary: "local work".into(),
            updated_at: chrono::Utc::now(),
            created_at: chrono::Utc::now(),
            cwd: tmp.path().display().to_string(),
            hostname: None,
            source: "local".into(),
            model_id: None,
            num_messages: 1,
            last_active_at: None,
            branch: None,
            repo_name: String::new(),
            worktree_label: None,
            last_turn_summary: None,
            last_recap: None,
            session_kind: None,
            card_detail: None,
        }]);
        let effects = dispatch(Action::PickSession(0), &mut app);
        assert_eq!(
            app.welcome_workspace_mode,
            WelcomeWorkspaceMode::Sandbox,
            "CLI lock must not rewrite welcome mode from Sandbox"
        );
        assert!(
            app.welcome_history_load_as_build,
            "CLI lock must still set local-disk load bypass"
        );
        assert!(
            effects.iter().any(|e| matches!(
                e,
                Effect::LoadSession {
                    chat_kind: false,
                    ..
                }
            )),
            "locked local-disk pick must still load: {effects:?}"
        );
        let _ = std::fs::remove_dir_all(sess_dir);
        set_active_local_workspace(None).unwrap();
    }
    #[test]
    fn failed_local_pick_clears_history_bypass() {
        set_active_local_workspace(None).unwrap();
        let mut app = test_app();
        app.chat_mode = true;
        app.active_view = ActiveView::Welcome;
        app.welcome_workspace_mode = WelcomeWorkspaceMode::Sandbox;
        app.session_picker_entries = Some(vec![crate::app::app_view::SessionPickerEntry {
            id: "missing-build".into(),
            summary: "gone".into(),
            updated_at: chrono::Utc::now(),
            created_at: chrono::Utc::now(),
            cwd: String::new(),
            hostname: None,
            source: "local".into(),
            model_id: None,
            num_messages: 1,
            last_active_at: None,
            branch: None,
            repo_name: String::new(),
            worktree_label: None,
            last_turn_summary: None,
            last_recap: None,
            session_kind: None,
            card_detail: None,
        }]);
        let effects = dispatch(Action::PickSession(0), &mut app);
        assert!(effects.is_empty());
        assert!(
            !app.welcome_history_load_as_build,
            "failed/no-op pick must not leak bypass"
        );
        set_active_local_workspace(None).unwrap();
    }
    #[test]
    fn deferred_history_bypass_survives_startup_gate() {
        set_active_local_workspace(None).unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let mut app = test_app();
        app.chat_mode = true;
        app.active_view = ActiveView::Welcome;
        app.trust_state = crate::app::app_view::TrustState::Pending {
            workspace: tmp.path().to_path_buf(),
        };
        app.welcome_history_load_as_build = true;
        let effects = dispatch(Action::LoadSession("sid".into(), None, false), &mut app);
        assert!(effects.is_empty());
        assert!(
            !app.welcome_history_load_as_build,
            "live flag moved onto deferred startup"
        );
        assert!(app.deferred_startup.history_load_as_build);
        let effects = finish_trust(&mut app);
        assert!(
            app.welcome_history_load_as_build,
            "drain must re-apply bypass before LoadSession"
        );
        assert!(
            effects.iter().any(|e| matches!(
                e,
                Effect::LoadSession {
                    chat_kind: false,
                    ..
                }
            )),
            "deferred drain must emit LoadSession: {effects:?}"
        );
        set_active_local_workspace(None).unwrap();
    }
    #[test]
    fn cli_lock_conversation_pick_does_not_rewrite_sandbox_mode() {
        let tmp = tempfile::tempdir().unwrap();
        set_active_local_workspace(Some(LocalWorkspaceConfig {
            mode: LocalWorkspaceMode::Attach,
            cwd: Some(tmp.path().to_path_buf()),
            server_id: Some("cli-srv".into()),
        }))
        .unwrap();
        let mut app = test_app();
        app.chat_mode = true;
        app.active_view = ActiveView::Welcome;
        app.local_workspace_startup_locked = true;
        app.welcome_workspace_mode = WelcomeWorkspaceMode::Sandbox;
        app.session_picker_entries = Some(vec![crate::app::app_view::SessionPickerEntry {
            id: "conv-lock".into(),
            summary: "hello".into(),
            updated_at: chrono::Utc::now(),
            created_at: chrono::Utc::now(),
            cwd: String::new(),
            hostname: None,
            source: "conversation".into(),
            model_id: None,
            num_messages: 1,
            last_active_at: None,
            branch: None,
            repo_name: String::new(),
            worktree_label: None,
            last_turn_summary: None,
            last_recap: None,
            session_kind: None,
            card_detail: None,
        }]);
        let effects = dispatch(Action::PickSession(0), &mut app);
        assert_eq!(
            app.welcome_workspace_mode,
            WelcomeWorkspaceMode::Sandbox,
            "CLI lock must not auto-switch welcome mode on conversation pick"
        );
        assert!(!app.welcome_history_load_as_build);
        assert!(
            effects.iter().any(|e| matches!(
                e,
                Effect::LoadSession {
                    chat_kind: true,
                    ..
                }
            )),
            "conversation must still load: {effects:?}"
        );
        let agent = app.agents.values().next().expect("agent");
        assert_eq!(
            agent.workspace_mode,
            WelcomeWorkspaceMode::Sandbox,
            "conversation without session-local intent → Sandbox (not Local·CLI)"
        );
        assert!(
            !agent.workspace_mode_cli_locked,
            "CLI lock must not badge conversation LoadSession"
        );
        set_active_local_workspace(None).unwrap();
    }
    #[test]
    fn session_restore_failed_clears_history_bypass() {
        set_active_local_workspace(None).unwrap();
        let mut app = test_app_with_agent();
        app.chat_mode = true;
        app.welcome_history_load_as_build = true;
        let id = AgentId(0);
        let effects = dispatch(
            Action::TaskComplete(TaskResult::SessionRestoreFailed {
                agent_id: id,
                error: "boom".into(),
            }),
            &mut app,
        );
        assert!(effects.is_empty());
        assert!(
            !app.welcome_history_load_as_build,
            "failed restore must not leak bypass into the next load"
        );
        set_active_local_workspace(None).unwrap();
    }
    #[test]
    fn restore_and_load_sets_local_workspace_indicator() {
        set_active_local_workspace(None).unwrap();
        let mut app = test_app();
        app.chat_mode = true;
        app.active_view = ActiveView::Welcome;
        app.welcome_workspace_mode = WelcomeWorkspaceMode::Sandbox;
        app.session_picker_entries = Some(vec![crate::app::app_view::SessionPickerEntry {
            id: "remote-1".into(),
            summary: "remote row".into(),
            updated_at: chrono::Utc::now(),
            created_at: chrono::Utc::now(),
            cwd: "/other".into(),
            hostname: None,
            source: "remote".into(),
            model_id: None,
            num_messages: 1,
            last_active_at: None,
            branch: None,
            repo_name: String::new(),
            worktree_label: None,
            last_turn_summary: None,
            last_recap: None,
            session_kind: None,
            card_detail: None,
        }]);
        let effects = dispatch(Action::PickSession(0), &mut app);
        assert!(
            effects
                .iter()
                .any(|e| matches!(e, Effect::RestoreAndLoadSession { .. })),
            "remote pick must restore: {effects:?}"
        );
        assert!(
            app.welcome_history_load_as_build,
            "bypass kept until follow-up LoadSession"
        );
        let agent = app.agents.values().next().expect("restore placeholder");
        assert_eq!(
            agent.workspace_mode,
            WelcomeWorkspaceMode::LocalWorkspace,
            "restore placeholder must show Local indicator"
        );
        set_active_local_workspace(None).unwrap();
    }
    #[test]
    fn history_build_bypass_only_applies_to_load_session_batch() {
        use crate::app::event_loop::welcome_history_build_bypass_applies;
        assert!(!welcome_history_build_bypass_applies(&[], true));
        assert!(!welcome_history_build_bypass_applies(
            &[Effect::FetchSessionList {
                host: crate::views::session_picker_surface::SessionPickerHost::Welcome,
                cwd_override: None,
                generation: 0,
                query: None,
                seq: 0,
                kind_filter: None,
                headless_policy: Default::default(),
            }],
            true
        ));
        assert!(welcome_history_build_bypass_applies(
            &[Effect::LoadSession {
                agent_id: AgentId(0),
                session_id: "s".into(),
                session_cwd: None,
                chat_kind: false,
            }],
            true
        ));
        assert!(welcome_history_build_bypass_applies(
            &[Effect::RestoreAndLoadSession {
                agent_id: AgentId(0),
                session_id: "s".into(),
                session_cwd: "/tmp".into(),
            }],
            true
        ));
        assert!(welcome_history_build_bypass_applies(
            &[Effect::CreateWorktreeSession {
                agent_id: AgentId(0),
                load_session_id: Some("s".into()),
                label: None,
                git_ref: None,
                model_id: None,
                permission_mode_override: None,
                preferred_session_id: None,
                minted_session_id: None,
                chat_kind: false,
            }],
            true
        ));
        assert!(
            crate::app::event_loop::welcome_history_build_bypass_consume(
                &[Effect::CreateWorktreeSession {
                    agent_id: AgentId(0),
                    load_session_id: Some("s".into()),
                    label: None,
                    git_ref: None,
                    model_id: None,
                    permission_mode_override: None,
                    preferred_session_id: None,
                    minted_session_id: None,
                    chat_kind: false,
                }],
                true
            ),
            "worktree-resume batch consumes bypass (single-batch create)"
        );
        assert!(
            !crate::app::event_loop::welcome_history_build_bypass_consume(
                &[Effect::RestoreAndLoadSession {
                    agent_id: AgentId(0),
                    session_id: "s".into(),
                    session_cwd: "/tmp".into(),
                }],
                true
            ),
            "restore-only batch keeps bypass for follow-up LoadSession"
        );
        assert!(
            crate::app::event_loop::welcome_history_build_bypass_consume(
                &[Effect::LoadSession {
                    agent_id: AgentId(0),
                    session_id: "s".into(),
                    session_cwd: None,
                    chat_kind: false,
                }],
                true
            )
        );
    }
    #[test]
    fn fetch_session_list_kind_filter_only_on_welcome_chat() {
        set_active_local_workspace(None).unwrap();
        let mut welcome = test_app();
        welcome.chat_mode = true;
        welcome.active_view = ActiveView::Welcome;
        welcome.welcome_workspace_mode = WelcomeWorkspaceMode::LocalWorkspace;
        match dispatch(Action::FetchSessionList, &mut welcome).as_slice() {
            [Effect::FetchSessionList { kind_filter, .. }] => {
                assert_eq!(
                    kind_filter.as_deref(),
                    Some(["build".to_string()].as_slice())
                );
            }
            other => panic!("{other:?}"),
        }
        let mut in_session = test_app_with_agent();
        in_session.chat_mode = true;
        match dispatch(Action::FetchSessionList, &mut in_session).as_slice() {
            [Effect::FetchSessionList { kind_filter, .. }] => {
                assert!(kind_filter.is_none())
            }
            other => panic!("{other:?}"),
        }
        set_active_local_workspace(None).unwrap();
    }
    #[test]
    fn in_session_new_does_not_clear_process_stamp() {
        let tmp = tempfile::tempdir().unwrap();
        set_active_local_workspace(Some(LocalWorkspaceConfig {
            mode: LocalWorkspaceMode::Own,
            cwd: Some(tmp.path().to_path_buf()),
            server_id: None,
        }))
        .unwrap();
        let mut app = test_app_with_agent();
        app.chat_mode = true;
        app.cwd = tmp.path().to_path_buf();
        app.welcome_workspace_mode = WelcomeWorkspaceMode::Sandbox;
        let _ = dispatch(Action::NewSession, &mut app);
        assert!(
            crate::app::session_startup::active_local_workspace()
                .unwrap()
                .is_some(),
            "in-session /new must not clear the process stamp"
        );
        set_active_local_workspace(None).unwrap();
    }
}
