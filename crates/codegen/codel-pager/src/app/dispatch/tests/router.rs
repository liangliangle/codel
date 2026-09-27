//! Tests for the action router, model switching, slash commands, and other cross-cutting dispatch behavior.
use super::*;
fn agent_ref(app: &AppView, id: AgentId) -> &AgentView {
    let Some(agent) = app.agents.get(&id) else {
        panic!("expected agent {id:?}");
    };
    agent
}
#[test]
fn external_prompt_editor_arms_typed_request_and_preserves_composer_modes() {
    use crate::app::agent_view::PromptInputMode;
    for mode in [
        PromptInputMode::Normal,
        PromptInputMode::Bash,
        PromptInputMode::Remember,
    ] {
        let mut app = test_app_with_agent();
        let id = AgentId(0);
        app.screen_mode = crate::app::ScreenMode::Minimal;
        let agent = app.agents.get_mut(&id).unwrap();
        agent
            .prompt
            .set_screen_mode(crate::app::ScreenMode::Minimal);
        agent.prompt_input_mode = mode;
        agent.prompt.set_text("draft with\nnewlines");
        let effects = dispatch(Action::EditPromptExternal, &mut app);
        assert!(effects.is_empty());
        let request = app.pending_editor.take().expect("editor request");
        match request {
            crate::app::external_editor::PendingEditorRequest::PromptDraft {
                agent_id,
                original_text,
            } => {
                assert_eq!(agent_id, id);
                assert_eq!(original_text, "draft with\nnewlines");
            }
            other => panic!("expected prompt draft request, got {other:?}"),
        }
        assert_eq!(agent_ref(&app, id).prompt_input_mode, mode);
        assert_eq!(agent_ref(&app, id).prompt.text(), "draft with\nnewlines");
    }
}
#[test]
fn external_prompt_editor_arms_in_fullscreen_and_refuses_owned_input() {
    let mut app = test_app_with_agent();
    let id = AgentId(0);
    app.agents.get_mut(&id).unwrap().prompt.set_text("draft");
    let _ = dispatch(Action::EditPromptExternal, &mut app);
    assert!(
        matches!(
            app.pending_editor.take(),
            Some(crate::app::external_editor::PendingEditorRequest::PromptDraft { .. })
        ),
        "full TUI arms the request without requiring prompt-pane focus"
    );
    app.screen_mode = crate::app::ScreenMode::Minimal;
    app.agents.get_mut(&id).unwrap().active_pane = ActivePane::Scrollback;
    let _ = dispatch(Action::EditPromptExternal, &mut app);
    assert!(
        matches!(
            app.pending_editor,
            Some(crate::app::external_editor::PendingEditorRequest::PromptDraft { .. })
        ),
        "the composer stays the editing surface with scrollback focused"
    );
    app.pending_editor = None;
    app.agents.get_mut(&id).unwrap().cancel_turn_view =
        Some(crate::views::modal::CancelTurnViewState {
            active_idx: 0,
            running_count: 1,
        });
    let _ = dispatch(Action::EditPromptExternal, &mut app);
    assert!(app.pending_editor.is_none(), "modal owner must refuse");
    assert_eq!(agent_ref(&app, id).prompt.text(), "draft");
    app.agents.get_mut(&id).unwrap().cancel_turn_view = None;
    app.agents.get_mut(&id).unwrap().prompt_mode = PromptMode::EditingQueued {
        id: 1,
        original: "queued".to_owned(),
        server_id: None,
        kind: crate::app::agent::QueueEntryKind::Prompt,
    };
    let _ = dispatch(Action::EditPromptExternal, &mut app);
    assert!(app.pending_editor.is_none(), "queue edit must refuse");
    assert_eq!(agent_ref(&app, id).prompt.text(), "draft");
    app.agents.get_mut(&id).unwrap().prompt_mode = PromptMode::Normal;
    app.agents.get_mut(&id).unwrap().prompt.set_text("/");
    let models = agent_ref(&app, id).session.models.clone();
    app.agents
        .get_mut(&id)
        .unwrap()
        .prompt
        .refresh_slash(&models);
    assert!(agent_ref(&app, id).prompt.any_dropdown_open());
    let _ = dispatch(Action::EditPromptExternal, &mut app);
    assert!(app.pending_editor.is_none(), "dropdown owner must refuse");
}
#[test]
fn external_prompt_editor_refuses_elements_with_visible_message() {
    let mut app = test_app_with_agent();
    let id = AgentId(0);
    app.screen_mode = crate::app::ScreenMode::Minimal;
    app.agents
        .get_mut(&id)
        .unwrap()
        .prompt
        .set_screen_mode(crate::app::ScreenMode::Minimal);
    let agent = app.agents.get_mut(&id).unwrap();
    let pasted = "one\ntwo\nthree\nfour";
    let _ = agent.prompt.handle_paste(pasted);
    assert!(!agent.prompt.textarea.elements().is_empty());
    let _ = dispatch(Action::EditPromptExternal, &mut app);
    assert!(app.pending_editor.is_none());
    assert_eq!(agent_ref(&app, id).prompt.text(), pasted);
    assert!(!agent_ref(&app, id).prompt.textarea.elements().is_empty());
    assert!(
        agent_ref(&app, id)
            .scrollback
            .iter_entries()
            .any(|(_, entry)| entry.block.searchable_text().as_deref()
                == Some(crate::app::external_editor::ATTACHMENT_MESSAGE))
    );
    let agent = app.agents.get_mut(&id).unwrap();
    agent.prompt.set_text("");
    agent.prompt.textarea.insert_element(
        "@src/main.rs",
        crate::views::prompt_widget::KIND_FILE_REF,
        None,
    );
    let file_ref_text = agent.prompt.text().to_owned();
    let _ = dispatch(Action::EditPromptExternal, &mut app);
    assert!(app.pending_editor.is_none());
    assert_eq!(agent_ref(&app, id).prompt.text(), file_ref_text);
    assert!(!agent_ref(&app, id).prompt.textarea.elements().is_empty());
    let agent = app.agents.get_mut(&id).unwrap();
    agent.prompt.set_text("");
    let image = crate::prompt_images::PastedImage {
        element_id: codel_ratatui_textarea::ElementId::from_raw(0),
        display_number: 0,
        mime_type: "image/png".to_owned(),
        dimensions: Some((8, 8)),
        byte_len: 1,
        encoded_bytes: Some(vec![0].into()),
        source_path: None,
        staged_temp_path: None,
        session_image_path: None,
        preview: crate::prompt_images::PromptImagePreview::default(),
    };
    agent.prompt.insert_image(image).unwrap();
    let image_text = agent.prompt.text().to_owned();
    let _ = dispatch(Action::EditPromptExternal, &mut app);
    assert!(app.pending_editor.is_none());
    assert_eq!(agent_ref(&app, id).prompt.text(), image_text);
    assert_eq!(agent_ref(&app, id).prompt.images.len(), 1);
}
#[test]
fn external_prompt_editor_refuses_voice_and_pending_paste_with_visible_messages() {
    use crate::app::agent_view::AgentDeferredSend;
    use crate::app::app_view::{VoiceState, VoiceTarget};
    for voice_state in [
        VoiceState::ColdStart {
            hold: false,
            target: VoiceTarget::Agent(AgentId(0)),
        },
        VoiceState::Recording {
            hold: false,
            target: VoiceTarget::Agent(AgentId(0)),
            interim: Some("partial".to_owned()),
        },
        VoiceState::Stopping {
            target: VoiceTarget::Agent(AgentId(0)),
            interim: Some("partial".to_owned()),
        },
    ] {
        let mut app = test_app_with_agent();
        let id = AgentId(0);
        app.screen_mode = crate::app::ScreenMode::Minimal;
        app.voice_state = voice_state;
        app.agents.get_mut(&id).unwrap().prompt.set_text("draft");
        let _ = dispatch(Action::EditPromptExternal, &mut app);
        assert!(app.pending_editor.is_none());
        assert_eq!(agent_ref(&app, id).prompt.text(), "draft");
        assert!(
            agent_ref(&app, id)
                .scrollback
                .iter_entries()
                .any(|(_, entry)| entry.block.searchable_text().as_deref()
                    == Some(crate::app::external_editor::VOICE_MESSAGE))
        );
    }
    for (probes, deferred_send) in [
        (1, None),
        (1, Some(AgentDeferredSend::SendPrompt)),
        (0, Some(AgentDeferredSend::SendPrompt)),
    ] {
        let mut app = test_app_with_agent();
        let id = AgentId(0);
        app.screen_mode = crate::app::ScreenMode::Minimal;
        let agent = app.agents.get_mut(&id).unwrap();
        agent.prompt.set_text("draft");
        agent.paste_probe_in_flight = probes;
        agent.deferred_send = deferred_send;
        let _ = dispatch(Action::EditPromptExternal, &mut app);
        assert!(app.pending_editor.is_none());
        assert_eq!(agent_ref(&app, id).prompt.text(), "draft");
        assert_eq!(agent_ref(&app, id).paste_probe_in_flight, probes);
        assert_eq!(agent_ref(&app, id).deferred_send, deferred_send);
        assert!(
            agent_ref(&app, id)
                .scrollback
                .iter_entries()
                .any(|(_, entry)| entry.block.searchable_text().as_deref()
                    == Some(crate::app::external_editor::PASTE_MESSAGE))
        );
    }
}
#[test]
fn deferred_paste_completion_after_refused_editor_does_not_implicitly_send_without_stash() {
    let mut app = test_app_with_agent();
    let id = AgentId(0);
    app.screen_mode = crate::app::ScreenMode::Minimal;
    let agent = app.agents.get_mut(&id).unwrap();
    agent.prompt.set_text("draft");
    let draft_len = agent.prompt.text().len();
    agent.prompt.set_cursor(draft_len);
    agent.paste_probe_in_flight = 1;
    let _ = dispatch(Action::EditPromptExternal, &mut app);
    assert!(app.pending_editor.is_none());
    let effects = dispatch(
        Action::TaskComplete(TaskResult::ClipboardAttachmentProbed {
            ctx: crate::app::actions::ClipboardPasteContext {
                target: crate::app::actions::ClipboardPasteTarget::AgentPrompt {
                    agent_id: id,
                    images_dir: None,
                },
                source: crate::app::actions::ClipboardPasteSource::ClipboardKey {
                    text: crate::app::actions::ClipboardTextRead::Success(Some(
                        "pasted".to_owned(),
                    )),
                    tip_showing: false,
                },
            },
            image: crate::app::actions::ProbedAttachment::NoRaster,
            file_urls: None,
        }),
        &mut app,
    );
    assert!(effects.is_empty(), "no deferred submit was armed");
    assert_eq!(agent_ref(&app, id).prompt.text(), "draftpasted");
    assert!(agent_ref(&app, id).session.pending_prompts.is_empty());
}
#[test]
fn external_prompt_editor_result_replaces_or_clears_without_sending() {
    let mut app = test_app_with_agent();
    let id = AgentId(0);
    app.agents.get_mut(&id).unwrap().session.state = AgentState::TurnRunning;
    app.agents.get_mut(&id).unwrap().prompt.set_text("original");
    crate::app::external_editor::apply_prompt_text(&mut app, id, "edited\n".to_owned());
    assert_eq!(agent_ref(&app, id).prompt.text(), "edited\n");
    assert!(agent_ref(&app, id).session.state.is_turn_running());
    assert!(agent_ref(&app, id).session.pending_prompts.is_empty());
    crate::app::external_editor::apply_prompt_text(&mut app, id, String::new());
    assert_eq!(agent_ref(&app, id).prompt.text(), "");
    assert!(agent_ref(&app, id).session.state.is_turn_running());
}
#[test]
fn editor_failure_targets_original_agent_and_vanished_agent_is_safe() {
    let mut app = test_app_with_agent();
    let id = AgentId(0);
    app.agents.get_mut(&id).unwrap().prompt.set_text("original");
    crate::app::external_editor::report_prompt_failure(&mut app, id, "editor failed");
    assert_eq!(agent_ref(&app, id).prompt.text(), "original");
    assert!(
        agent_ref(&app, id)
            .scrollback
            .iter_entries()
            .any(|(_, entry)| entry.block.searchable_text().as_deref() == Some("editor failed"))
    );
    app.agents.shift_remove(&id);
    crate::app::external_editor::apply_prompt_text(&mut app, id, "ignored".to_owned());
    crate::app::external_editor::report_prompt_failure(&mut app, id, "ignored");
    assert!(app.agents.is_empty());
}
#[test]
fn config_editor_action_still_uses_typed_request() {
    let mut app = test_app_with_agent();
    let path = std::path::PathBuf::from("/tmp/agent-config.md");
    let _ = dispatch(
        Action::SuspendForEditor {
            path: path.clone(),
            refresh_agents_modal: Some(crate::views::agents_modal::AgentsTab::Agents),
        },
        &mut app,
    );
    assert!(matches!(
        app.pending_editor,
        Some(crate::app::external_editor::PendingEditorRequest::ConfigFile {
            path: ref queued,
            refresh_agents_modal: Some(crate::views::agents_modal::AgentsTab::Agents),
        }) if queued == &path
    ));
}
fn seed_foreign_resume_hint(
    app: &mut AppView,
    tool: codel_foreign_sessions::ForeignSessionTool,
) {
    app.foreign_session_compat = codel_foreign_sessions::EnabledForeignSessionSources {
        claude: true,
        codex: true,
        cursor: true,
    };
    let Effect::CanonicalizeForeignResumeCwd {
        requested_cwd,
        launch_token,
    } = app.begin_foreign_resume_detection().unwrap()
    else {
        panic!("expected canonicalization effect");
    };
    let canonical_cwd = dunce::canonicalize(&requested_cwd).unwrap();
    assert!(app.accept_foreign_resume_canonical_cwd(
        launch_token,
        &requested_cwd,
        Some(canonical_cwd.clone()),
    ));
    app.apply_foreign_resume_detection(
        launch_token,
        &canonical_cwd,
        Some(codel_foreign_sessions::RecentForeignSession {
            tool,
            native_id: "native-id".into(),
            age: std::time::Duration::from_secs(60),
        }),
    );
}
/// Sending feedback is a submit: it retires the active ephemeral tip.
#[test]
fn send_feedback_clears_active_ephemeral_tip() {
    let mut app = test_app_with_agent();
    let id = AgentId(0);
    let agent = app.agents.get_mut(&id).unwrap();
    let _ = agent.ephemeral_tip.show(
        crate::tips::EphemeralTip::new("t", ratatui::text::Line::from("hint")),
        &mut std::collections::HashMap::new(),
    );
    assert!(agent.ephemeral_tip.is_active());
    let _ = dispatch(
        Action::SendFeedback {
            text: "it broke".into(),
            images: Default::default(),
            trace: None,
        },
        &mut app,
    );
    assert!(
        !app.agents.get(&id).unwrap().ephemeral_tip.is_active(),
        "feedback submit must clear the tip"
    );
}
/// Sending a remember note is a submit: it retires the active ephemeral tip.
#[test]
fn send_remember_note_clears_active_ephemeral_tip() {
    let mut app = test_app_with_agent();
    let id = AgentId(0);
    let agent = app.agents.get_mut(&id).unwrap();
    let _ = agent.ephemeral_tip.show(
        crate::tips::EphemeralTip::new("t", ratatui::text::Line::from("hint")),
        &mut std::collections::HashMap::new(),
    );
    assert!(agent.ephemeral_tip.is_active());
    let _ = dispatch(Action::SendRememberNote("remember this".into()), &mut app);
    assert!(
        !app.agents.get(&id).unwrap().ephemeral_tip.is_active(),
        "remember-note submit must clear the tip"
    );
}
#[test]
fn follow_up_chip_does_not_execute_slash_command() {
    let mut app = test_app_with_agent();
    let id = AgentId(0);
    assert!(!agent_ref(&app, id).session.is_yolo());
    let effects = dispatch(Action::SubmitFollowUp("/always-approve".into()), &mut app);
    assert!(
        !agent_ref(&app, id).session.is_yolo(),
        "a /always-approve chip must NOT flip YOLO mode"
    );
    assert!(
        matches!(effects.as_slice(), [Effect::SendPrompt { text, .. }] if text == "/always-approve"),
        "chip text must be submitted literally, got {effects:?}"
    );
}
#[test]
fn follow_up_chip_does_not_execute_exit_alias() {
    let mut app = test_app_with_agent();
    let effects = dispatch(Action::SubmitFollowUp("quit".into()), &mut app);
    assert!(
        matches!(effects.as_slice(), [Effect::SendPrompt { text, .. }] if text == "quit"),
        "bare 'quit' chip must be a literal prompt, got {effects:?}"
    );
}
#[test]
fn chip_submit_while_running_clears_follow_up_chips() {
    let mut app = test_app_with_agent();
    let id = AgentId(0);
    {
        let agent = app.agents.get_mut(&id).unwrap();
        agent.apply_follow_ups("resp-1".into(), vec!["Summarize".into()]);
        assert!(agent.follow_ups.is_some(), "precondition: chips shown");
        agent.session.state = AgentState::TurnRunning;
    }
    let effects = dispatch(Action::SubmitFollowUp("Summarize".into()), &mut app);
    assert!(
        matches!(effects.as_slice(), [Effect::SendPrompt { text, .. }] if text == "Summarize"),
        "chip must immediate-send while running, got {effects:?}"
    );
    assert_eq!(agent_ref(&app, id).session.queue_len(), 0);
    assert!(
        agent_ref(&app, id).follow_ups.is_none(),
        "immediate-send chip path must clear chips"
    );
}
#[test]
fn chip_submit_while_reconnect_pending_keeps_chips_and_does_not_send() {
    let mut app = test_app_with_agent();
    let id = AgentId(0);
    app.agents
        .get_mut(&id)
        .unwrap()
        .apply_follow_ups("resp-1".into(), vec!["Summarize".into()]);
    app.reconnect_pending = true;
    let effects = dispatch(Action::SubmitFollowUp("Summarize".into()), &mut app);
    assert!(
        effects.is_empty(),
        "a reconnect-pending submit must emit no effect, got {effects:?}"
    );
    assert_eq!(agent_ref(&app, id).session.queue_len(), 0);
    assert!(
        agent_ref(&app, id).follow_ups.is_some(),
        "reconnect-pending submit must NOT clear the chips"
    );
    app.reconnect_pending = false;
    app.agents.get_mut(&id).unwrap().session.state = AgentState::TurnRunning;
    let effects2 = dispatch(Action::SubmitFollowUp("Summarize".into()), &mut app);
    assert!(
        matches!(effects2.as_slice(), [Effect::SendPrompt { text, .. }] if text == "Summarize"),
        "after reconnect clears, the chip must submit, got {effects2:?}"
    );
    assert!(
        agent_ref(&app, id).follow_ups.is_none(),
        "a proceeding submit must clear the chips"
    );
}
#[test]
fn mark_turn_finished_clears_start_and_stamps_active() {
    let mut app = test_app_with_agent();
    let id = AgentId(0);
    let agent = app.agents.get_mut(&id).unwrap();
    agent.turn_started_at = Some(std::time::Instant::now());
    agent.last_active_at = None;
    agent.mark_turn_finished(crate::app::cancel_latency::TurnEnd::Completed);
    assert!(
        agent.turn_started_at.is_none(),
        "turn_started_at must be cleared"
    );
    assert!(
        agent.last_active_at.is_some(),
        "last_active_at must be stamped"
    );
}
fn critical_announcement(id: &str) -> codel_announcements::RemoteAnnouncement {
    codel_announcements::RemoteAnnouncement {
        id: Some(id.into()),
        title: Some(format!("{id} title")),
        message: Some(format!("{id} message")),
        severity: Some("critical".into()),
        ..Default::default()
    }
}
fn promo_announcement(id: &str) -> codel_announcements::RemoteAnnouncement {
    codel_announcements::RemoteAnnouncement {
        id: Some(id.into()),
        message: Some(format!("{id} message")),
        severity: Some("promo".into()),
        cta: Some(codel_announcements::AnnouncementCta {
            label: Some("Go".into()),
            url: Some(format!("https://codel/{id}")),
            caption: None,
        }),
        ..Default::default()
    }
}
/// Id of the item the banner slot currently selects (None = banner closed).
fn shown_banner_id(app: &AppView) -> Option<String> {
    crate::views::announcements::first_session_announcement(
        &app.active_announcements,
        &app.hidden_announcement_ids,
    )
    .and_then(|a| a.id.clone())
}
/// `AnnouncementsOpenCta(surface)` re-resolves through the slot gate and opens the promo url from every surface.
/// The opens are observed through the file named by `CODEL_TEST_OPEN_URL_FILE`.
/// A critical owning the slot, or no usable cta, makes it a silent no-op, so a stale prior-frame click cannot open the promo url.
#[serial_test::serial(CODEL_TEST_OPEN_URL_FILE)]
#[test]
fn announcements_open_cta_opens_promo_and_noops_under_critical() {
    use codel_logging::events::AnnouncementCtaSurface;
    let url_file = std::env::temp_dir().join(format!("codel-cta-open-{}.txt", std::process::id()));
    let _ = std::fs::remove_file(&url_file);
    unsafe { std::env::set_var("CODEL_TEST_OPEN_URL_FILE", &url_file) };
    let opened = || std::fs::read_to_string(&url_file).unwrap_or_default();
    let mut app = test_app_with_agent();
    app.active_announcements = vec![promo_announcement("promo-open")];
    for surface in [
        AnnouncementCtaSurface::Banner,
        AnnouncementCtaSurface::Welcome,
        AnnouncementCtaSurface::Header,
        AnnouncementCtaSurface::Dashboard,
        AnnouncementCtaSurface::Keyboard,
    ] {
        let _ = std::fs::write(&url_file, "");
        let effects = dispatch(Action::AnnouncementsOpenCta(surface), &mut app);
        assert!(effects.is_empty(), "open is a side effect, not an Effect");
        assert!(
            opened().lines().any(|l| l == "https://codel/promo-open"),
            "surface {surface:?} must open the promo url; got {:?}",
            opened()
        );
    }
    let _ = std::fs::write(&url_file, "");
    app.active_announcements = vec![
        critical_announcement("crit-a"),
        promo_announcement("promo-open"),
    ];
    let _ = dispatch(
        Action::AnnouncementsOpenCta(AnnouncementCtaSurface::Keyboard),
        &mut app,
    );
    assert!(
        opened().trim().is_empty(),
        "a critical slot owner must make the open a no-op; got {:?}",
        opened()
    );
    let _ = std::fs::write(&url_file, "");
    app.active_announcements = vec![];
    let _ = dispatch(
        Action::AnnouncementsOpenCta(AnnouncementCtaSurface::Banner),
        &mut app,
    );
    assert!(opened().trim().is_empty(), "no cta → no open");
    unsafe { std::env::remove_var("CODEL_TEST_OPEN_URL_FILE") };
    let _ = std::fs::remove_file(&url_file);
}
/// `AnnouncementCtaShown` latches once per (announcement, surface) pair.
/// The first frame with an armed CTA rect emits, later frames don't, and a NEW announcement id re-emits on the same surfaces.
#[test]
fn cta_impressions_latch_once_per_surface_and_reemit_for_new_id() {
    use crate::app::app_view::ActiveView;
    use codel_logging::events::AnnouncementCtaSurface;
    let mut app = test_app_with_agent();
    let id = AgentId(0);
    app.active_view = ActiveView::Agent(id);
    app.active_announcements = vec![promo_announcement("promo-a")];
    let rect = Some(ratatui::layout::Rect::new(0, 0, 4, 1));
    {
        let agent = app.agents.get_mut(&id).unwrap();
        agent.hit_announcement_cta.set(rect);
        agent.hit_upgrade_cta.set(rect);
    }
    app.log_announcement_cta_impressions();
    let logged = &app.announcement_cta_impressions_logged;
    assert_eq!(logged.len(), 2);
    assert!(logged.contains(&("promo-a".to_string(), AnnouncementCtaSurface::Banner)));
    assert!(logged.contains(&("promo-a".to_string(), AnnouncementCtaSurface::Header)));
    app.log_announcement_cta_impressions();
    assert_eq!(app.announcement_cta_impressions_logged.len(), 2);
    app.active_announcements = vec![promo_announcement("promo-b")];
    app.log_announcement_cta_impressions();
    let logged = &app.announcement_cta_impressions_logged;
    assert_eq!(logged.len(), 4);
    assert!(logged.contains(&("promo-b".to_string(), AnnouncementCtaSurface::Banner)));
    assert!(logged.contains(&("promo-b".to_string(), AnnouncementCtaSurface::Header)));
}
/// No impression is logged without a painted button under a promo slot owner.
/// A critical preempting the slot, a hidden promo, and cleared (unpainted) rects all emit nothing; this is the same gate the click dispatch resolves.
#[test]
fn cta_impressions_respect_slot_gate_and_paint() {
    use crate::app::app_view::ActiveView;
    let mut app = test_app_with_agent();
    let id = AgentId(0);
    app.active_view = ActiveView::Agent(id);
    let rect = Some(ratatui::layout::Rect::new(0, 0, 4, 1));
    {
        let agent = app.agents.get_mut(&id).unwrap();
        agent.hit_announcement_cta.set(rect);
        agent.hit_upgrade_cta.set(rect);
    }
    app.active_announcements = vec![critical_announcement("crit"), promo_announcement("p")];
    app.log_announcement_cta_impressions();
    assert!(app.announcement_cta_impressions_logged.is_empty());
    app.active_announcements = vec![promo_announcement("p")];
    app.hidden_announcement_ids = ["p".to_string()].into_iter().collect();
    app.log_announcement_cta_impressions();
    assert!(app.announcement_cta_impressions_logged.is_empty());
    app.hidden_announcement_ids.clear();
    {
        let agent = app.agents.get_mut(&id).unwrap();
        agent.hit_announcement_cta.clear();
        agent.hit_upgrade_cta.clear();
    }
    app.log_announcement_cta_impressions();
    assert!(app.announcement_cta_impressions_logged.is_empty());
    {
        let agent = app.agents.get_mut(&id).unwrap();
        agent.active_subagent = Some("child-sid".into());
        agent.hit_announcement_cta.clear();
        agent.hit_upgrade_cta.clear();
    }
    app.active_announcements = vec![promo_announcement("p2")];
    app.log_announcement_cta_impressions();
    assert!(app.announcement_cta_impressions_logged.is_empty());
}
/// Frame occluders (the goal-detail class) leave rects armed and block clicks at dispatch time.
/// Impressions follow the same rule as OSC 8 links: an occluded CTA is not counted until an overlay-free frame shows it clean.
#[test]
fn cta_impressions_suppressed_while_rect_occluded() {
    use crate::app::app_view::ActiveView;
    use codel_logging::events::AnnouncementCtaSurface;
    let mut app = test_app_with_agent();
    let id = AgentId(0);
    app.active_view = ActiveView::Agent(id);
    app.active_announcements = vec![promo_announcement("p")];
    let rect = ratatui::layout::Rect::new(0, 0, 4, 1);
    let overlay = ratatui::layout::Rect::new(0, 0, 80, 1);
    {
        let agent = app.agents.get_mut(&id).unwrap();
        agent.hit_announcement_cta.set(Some(rect));
        agent.hit_upgrade_cta.set(Some(rect));
        agent.frame_occluder_rects.push(overlay);
    }
    app.log_announcement_cta_impressions();
    assert!(app.announcement_cta_impressions_logged.is_empty());
    {
        let agent = app.agents.get_mut(&id).unwrap();
        agent.frame_occluder_rects.clear();
    }
    app.log_announcement_cta_impressions();
    let logged = &app.announcement_cta_impressions_logged;
    assert_eq!(logged.len(), 2);
    assert!(logged.contains(&("p".to_string(), AnnouncementCtaSurface::Banner)));
    assert!(logged.contains(&("p".to_string(), AnnouncementCtaSurface::Header)));
    app.log_announcement_cta_impressions();
    assert_eq!(app.announcement_cta_impressions_logged.len(), 2);
}
#[test]
fn dispatch_send_prompt_announcements_via_registry() {
    let mut app = test_app_with_agent();
    let agent_id = AgentId(0);
    switch_to_agent(&mut app, agent_id, SwitchCause::New);
    app.active_announcements = vec![critical_announcement("crit-a")];
    let effects = dispatch(Action::SendPrompt("/announcements hide".into()), &mut app);
    assert!(
            effects.iter().any(
                |e| matches!(e, Effect::PersistAnnouncementsHidden { hidden_ids } if hidden_ids.contains("crit-a"))
            ),
            "expected persist effect carrying the hidden id, got {effects:?}"
        );
    assert!(app.hidden_announcement_ids.contains("crit-a"));
    assert_eq!(shown_banner_id(&app), None, "hidden critical closes banner");
    assert!(agent_ref(&app, agent_id).prompt.text().is_empty());
    let initial_scrollback_len = agent_ref(&app, agent_id).scrollback.len();
    let initial_queue_len = agent_ref(&app, agent_id).session.queue_len();
    let effects = dispatch(Action::SendPrompt("/announcements foo".into()), &mut app);
    assert!(effects.is_empty(), "expected no effects, got {effects:?}");
    assert_eq!(
        agent_ref(&app, agent_id).session.queue_len(),
        initial_queue_len
    );
    assert_eq!(
        agent_ref(&app, agent_id).scrollback.len(),
        initial_scrollback_len + 1,
        "expected usage message in scrollback"
    );
}
#[test]
fn switch_model_dispatch_produces_effect_and_sets_pending() {
    let mut app = test_app_with_agent();
    let id = AgentId(0);
    let model_id = acp::ModelId::new(std::sync::Arc::from("codel-4.5"));
    assert!(!agent_ref(&app, id).session.model_switch_pending);
    let effects = dispatch(
        Action::SwitchModel {
            model_id: model_id.clone(),
            effort: None,
        },
        &mut app,
    );
    assert_eq!(effects.len(), 1);
    assert!(
        matches!(effects.first(), Some(Effect::SwitchModel { model_id: mid, .. }) if mid == &model_id)
    );
    assert!(agent_ref(&app, id).session.model_switch_pending);
    assert!(agent_ref(&app, id).session.state.is_idle());
}
#[test]
fn switch_model_allowed_when_agent_chat_kind() {
    let mut app = test_app_with_agent();
    let id = AgentId(0);
    app.agents.get_mut(&id).unwrap().chat_kind = true;
    let model_id = acp::ModelId::new(std::sync::Arc::from("auto"));
    let effects = dispatch(
        Action::SwitchModel {
            model_id: model_id.clone(),
            effort: None,
        },
        &mut app,
    );
    assert_eq!(effects.len(), 1);
    assert!(
        matches!(effects.first(), Some(Effect::SwitchModel { model_id: mid, .. }) if mid == &model_id)
    );
    assert!(agent_ref(&app, id).session.model_switch_pending);
}
#[test]
fn switch_model_allowed_when_app_chat_mode() {
    let mut app = test_app_with_agent();
    let id = AgentId(0);
    app.chat_mode = true;
    let model_id = acp::ModelId::new(std::sync::Arc::from("auto"));
    let effects = dispatch(
        Action::SwitchModel {
            model_id: model_id.clone(),
            effort: None,
        },
        &mut app,
    );
    assert_eq!(effects.len(), 1);
    assert!(
        matches!(effects.first(), Some(Effect::SwitchModel { model_id: mid, .. }) if mid == &model_id)
    );
    assert!(agent_ref(&app, id).session.model_switch_pending);
}
#[test]
fn agent_type_mismatch_cancel_is_noop() {
    let mut app = test_app_with_agent();
    let id = AgentId(0);
    let model_id = acp::ModelId::new(std::sync::Arc::from("cursor-model"));
    let agent_count_before = app.agents.len();
    let effects = dispatch(
        Action::AgentTypeMismatchAnswered {
            start_new: false,
            model_id,
            effort: None,
        },
        &mut app,
    );
    assert!(effects.is_empty());
    assert_eq!(app.agents.len(), agent_count_before);
    assert!(matches!(app.active_view, ActiveView::Agent(a) if a == id));
}
#[test]
fn agent_type_mismatch_with_effort_stashes_deferred_switch() {
    use codel_shell::sampling::types::ReasoningEffort;
    let mut app = test_app_with_agent();
    let model_id = acp::ModelId::new(std::sync::Arc::from("cursor-reasoning"));
    let effort = Some(ReasoningEffort::High);
    let effects = dispatch(
        Action::AgentTypeMismatchAnswered {
            start_new: true,
            model_id: model_id.clone(),
            effort,
        },
        &mut app,
    );
    let create = effects
        .iter()
        .find(|e| matches!(e, Effect::CreateSession { .. }));
    assert!(create.is_some(), "expected CreateSession effect");
    match create.unwrap() {
        Effect::CreateSession { model_id: mid, .. } => {
            assert_eq!(mid.as_ref(), Some(&model_id));
        }
        _ => unreachable!(),
    }
    if let ActiveView::Agent(new_aid) = app.active_view {
        let agent = agent_ref(&app, new_aid);
        assert_eq!(
            agent.session.deferred_model_switch,
            Some(crate::app::agent::DeferredModelSwitch {
                model_id,
                effort,
                prev_model_id: None,
            }),
            "effort override must be stashed for the shell via deferred_model_switch",
        );
    } else {
        panic!("expected active view to be an Agent");
    }
}
#[test]
fn test_helper_agent_uses_generation_zero() {
    let app = test_app_with_agent();
    let id = AgentId(0);
    assert!(agent_ref(&app, id).session.available_commands.is_empty());
    assert_eq!(agent_ref(&app, id).session.available_commands_generation, 0);
    assert!(!agent_ref(&app, id).session.model_switch_pending);
}
#[test]
fn slash_exit_dispatches_quit() {
    let mut app = test_app_with_agent();
    let effects = dispatch(Action::SendPrompt("/exit".into()), &mut app);
    assert!(
        effects.last().is_some_and(|e| matches!(e, Effect::Quit)),
        "expected Quit as last effect, got: {effects:?}"
    );
}
#[test]
fn slash_quit_alias_dispatches_quit() {
    let mut app = test_app_with_agent();
    let effects = dispatch(Action::SendPrompt("/quit".into()), &mut app);
    assert!(effects.last().is_some_and(|e| matches!(e, Effect::Quit)));
}
#[test]
fn slash_new_does_not_cancel_running_turn() {
    let mut app = test_app_with_agent();
    let id = AgentId(0);
    app.agents.get_mut(&id).unwrap().session.state = AgentState::TurnRunning;
    let effects = dispatch(Action::SendPrompt("/new".into()), &mut app);
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::CreateSession { .. })),
        "expected CreateSession, got: {effects:?}",
    );
    assert!(
        !effects
            .iter()
            .any(|e| matches!(e, Effect::CancelTurn { .. }))
    );
    assert!(
        agent_ref(&app, id).session.state.is_turn_running(),
        "old agent's turn must remain running"
    );
}
#[test]
fn slash_new_uses_active_agent_cwd() {
    let mut app = test_app_with_agent();
    let id = AgentId(0);
    let agent_cwd = PathBuf::from("/custom/agent/cwd");
    app.agents.get_mut(&id).unwrap().session.cwd = agent_cwd.clone();
    let effects = dispatch(Action::SendPrompt("/new".into()), &mut app);
    let create = effects
        .iter()
        .find(|e| matches!(e, Effect::CreateSession { .. }));
    assert!(create.is_some(), "expected CreateSession effect");
    match create.unwrap() {
        Effect::CreateSession { cwd, .. } => assert_eq!(cwd, &agent_cwd),
        _ => unreachable!(),
    }
    let new_id = AgentId(1);
    assert!(!agent_ref(&app, new_id).session.is_worktree);
}
#[test]
fn slash_model_invalid_arg_produces_scrollback_error() {
    let mut app = test_app_with_agent();
    let id = AgentId(0);
    let initial_scrollback = agent_ref(&app, id).scrollback.len();
    let effects = dispatch(Action::SendPrompt("/model nonexistent".into()), &mut app);
    assert!(effects.is_empty(), "error should not produce effects");
    assert_eq!(agent_ref(&app, id).scrollback.len(), initial_scrollback + 1);
    assert!(agent_ref(&app, id).prompt.text().is_empty());
}
#[test]
fn slash_model_no_args_produces_scrollback_error() {
    let mut app = test_app_with_agent();
    let id = AgentId(0);
    let initial_scrollback = agent_ref(&app, id).scrollback.len();
    let effects = dispatch(Action::SendPrompt("/model".into()), &mut app);
    assert!(effects.is_empty());
    assert_eq!(agent_ref(&app, id).scrollback.len(), initial_scrollback + 1);
}
#[test]
fn slash_hooks_opens_modal() {
    let mut app = test_app_with_agent();
    app.appearance.disable_plugins = false;
    let id = AgentId(0);
    let effects = dispatch(Action::SendPrompt("/hooks".into()), &mut app);
    assert!(agent_ref(&app, id).extensions_modal.is_some());
    assert_eq!(effects.len(), 6);
}
#[test]
fn deferred_switch_overwritten_by_second_switch() {
    let mut app = test_app_with_agent();
    let id = AgentId(0);
    let model_a = acp::ModelId::new(std::sync::Arc::from("model-a"));
    let model_b = acp::ModelId::new(std::sync::Arc::from("model-b"));
    app.agents.get_mut(&id).unwrap().session.session_id = None;
    dispatch(
        Action::SwitchModel {
            model_id: model_a.clone(),
            effort: None,
        },
        &mut app,
    );
    dispatch(
        Action::SwitchModel {
            model_id: model_b.clone(),
            effort: None,
        },
        &mut app,
    );
    assert_eq!(
        agent_ref(&app, id).session.deferred_model_switch,
        Some(crate::app::agent::DeferredModelSwitch {
            model_id: model_b.clone(),
            effort: None,
            prev_model_id: Some(model_a),
        })
    );
}
#[test]
fn pick_over_cli_seed_keeps_display_as_rollback_target() {
    let mut app = test_app_with_agent();
    let id = AgentId(0);
    let displayed = acp::ModelId::new(std::sync::Arc::from("displayed-model"));
    let cli_model = acp::ModelId::new(std::sync::Arc::from("cli-model"));
    let picked = acp::ModelId::new(std::sync::Arc::from("picked-model"));
    let agent = app.agents.get_mut(&id).unwrap();
    agent.session.session_id = None;
    agent.session.models.current = Some(displayed.clone());
    agent.session.deferred_model_switch = Some(crate::app::agent::DeferredModelSwitch {
        model_id: cli_model,
        effort: None,
        prev_model_id: None,
    });
    dispatch(
        Action::SwitchModel {
            model_id: picked.clone(),
            effort: None,
        },
        &mut app,
    );
    assert_eq!(
        agent_ref(&app, id).session.deferred_model_switch,
        Some(crate::app::agent::DeferredModelSwitch {
            model_id: picked,
            effort: None,
            prev_model_id: Some(displayed),
        })
    );
}
#[test]
fn deferred_switch_updates_display_and_persists() {
    let mut app = test_app_with_agent();
    let id = AgentId(0);
    let model_id = acp::ModelId::new(std::sync::Arc::from("model-b"));
    app.agents.get_mut(&id).unwrap().session.session_id = None;
    let effects = dispatch(
        Action::SwitchModel {
            model_id: model_id.clone(),
            effort: None,
        },
        &mut app,
    );
    let agent = agent_ref(&app, id);
    assert_eq!(
        agent.session.models.current,
        Some(model_id.clone()),
        "pre-session pick must update the displayed model immediately"
    );
    assert_eq!(
        agent.session.deferred_model_switch,
        Some(crate::app::agent::DeferredModelSwitch {
            model_id: model_id.clone(),
            effort: None,
            prev_model_id: None,
        }),
        "switch must still round-trip once the session exists"
    );
    assert!(
        !agent.session.model_switch_pending,
        "nothing is in flight yet — the queue must not be blocked"
    );
    assert!(
        matches!(
            effects.as_slice(),
            [Effect::PersistPreferredModel { model_id: m, .. }] if m == &model_id
        ),
        "expected a single PersistPreferredModel effect, got {effects:?}"
    );
    let effects = dispatch(
        Action::SwitchModel {
            model_id: model_id.clone(),
            effort: None,
        },
        &mut app,
    );
    assert!(
        effects.is_empty(),
        "unchanged pre-session pick must not re-persist, got {effects:?}"
    );
}
/// Under sticky `--chat`, a local disk row loaded through the history bypass stays Build for rename.
/// Without the bypass flag this same disk row is refused (see `chat_mode_refuses_local_build_disk_load`).
#[cfg(feature = "local-workspace")]
#[test]
fn load_sticky_chat_history_bypass_rename_kind_is_build() {
    let cwd = PathBuf::from(format!("/tmp/chat-mode-hist-bypass-{}", std::process::id()));
    let session_id = format!("local-build-disk-{}", std::process::id());
    let sess_dir = plant_local_build_session(&cwd, &session_id);
    let mut app = test_app();
    app.cwd = cwd;
    app.chat_mode = true;
    app.welcome_history_load_as_build = true;
    let effects = dispatch(
        Action::LoadSession(session_id.clone(), None, false),
        &mut app,
    );
    let _ = std::fs::remove_dir_all(&sess_dir);
    assert!(
        matches!(
            effects.as_slice(),
            [Effect::LoadSession {
                session_id: sid,
                chat_kind: false,
                ..
            }] if sid == &session_id
        ),
        "history-bypass must load the local disk row, got {effects:?}"
    );
    let agent = app.agents.values().next().expect("agent");
    assert!(
        agent.chat_kind,
        "sticky --chat still sets the UI chat_kind bit"
    );
    assert!(
        !agent.conversation_entry,
        "history-bypass local build must not open as chat"
    );
    assert_eq!(
        agent.rename_kind(),
        codel_shell::session::unified_list::SessionKind::Build
    );
    let rename = dispatch(
        Action::RenameSession {
            title: "local title".into(),
        },
        &mut app,
    );
    assert!(
        matches!(
            rename.as_slice(),
            [Effect::RenameSession { kind, title, .. }]
                if *kind == codel_shell::session::unified_list::SessionKind::Build
                    && title == "local title"
        ),
        "history-bypass rename must send kind=build, got {rename:?}"
    );
}
/// End-to-end regression test for the "always re-asks" requirement.
/// dispatch(Action::Fork) -> modal MUST re-open.
/// (a) "no persistence in dispatch_fork": whether the modal opens is decided only by the absence of `args.worktree_override`; and (b) "submit_question_answers clears question_view": open_fork_question refuses while a question is already on screen.
#[test]
fn dispatch_fork_no_flag_always_reopens_modal_after_previous_answer() {
    use crate::views::question_view::QuestionSelection;
    let mut app = fork_test_app();
    app.fork_worktree_mode = crate::app::app_view::WorktreeMode::Ask;
    let effects = dispatch(Action::Fork(fork_args(None, None)), &mut app);
    assert!(effects.is_empty(), "round 1: no effects until answered");
    let qv1 = agent_ref(&app, AgentId(0))
        .question_view
        .as_ref()
        .expect("round 1: modal opened");
    assert_eq!(
        qv1.questions.first().map(|q| q.options.len()),
        Some(4),
        "round 1: modal offers exactly 4 options (Yes/No/Always/Never)"
    );
    {
        let agent = app.agents.get_mut(&AgentId(0)).unwrap();
        let qv = agent.question_view.as_mut().expect("modal still present");
        let Some(slot) = qv.selections.first_mut() else {
            panic!("expected first question selection");
        };
        *slot = QuestionSelection::Single(Some(0));
    }
    let outcome = app
        .agents
        .get_mut(&AgentId(0))
        .unwrap()
        .submit_question_answers_for_test(false);
    let inner = match outcome {
        crate::app::app_view::InputOutcome::Action(a) => a,
        other => panic!("expected InputOutcome::Action, got {other:?}"),
    };
    assert!(
        matches!(inner, Action::ForkAnswered { worktree: true, .. }),
        "submit must produce ForkAnswered with worktree=true, got {inner:?}"
    );
    assert!(
        agent_ref(&app, AgentId(0)).question_view.is_none(),
        "submit must clear question_view on the parent agent"
    );
    let effects = dispatch(inner, &mut app);
    assert!(matches!(
        effects.as_slice(),
        [Effect::CreateWorktreeSession { .. }]
    ));
    switch_to_agent(&mut app, AgentId(0), SwitchCause::Picker);
    let effects = dispatch(Action::Fork(fork_args(None, None)), &mut app);
    assert!(effects.is_empty(), "round 2: no effects until answered");
    let qv2 = agent_ref(&app, AgentId(0))
        .question_view
        .as_ref()
        .expect("round 2: modal must re-open (choice never persisted)");
    assert_eq!(
        qv2.questions.first().map(|q| q.options.len()),
        Some(4),
        "round 2: modal still offers exactly 4 options (Yes/No/Always/Never)"
    );
}
#[test]
fn translate_local_submit_skipped_returns_changed_with_no_action() {
    use crate::views::question_view::{LocalQuestionKind, QuestionViewState};
    use codel_tools::implementations::codel_build::ask_user_question::{
        Question, QuestionOption,
    };
    let q = Question {
        question: "?".into(),
        options: vec![QuestionOption {
            label: "x".into(),
            description: "x".into(),
            preview: None,
            id: None,
        }],
        multi_select: Some(false),
        id: None,
    };
    let state = QuestionViewState::new(
        "x".into(),
        vec![q],
        crate::views::prompt_widget::StashedPrompt::default(),
    );
    let kind = LocalQuestionKind::Fork {
        directive: Some("dropped".into()),
    };
    let outcome = crate::app::agent_view::translate_local_submit_for_test(&state, kind, true);
    assert!(matches!(
        outcome,
        crate::app::app_view::InputOutcome::Changed
    ));
}
#[test]
fn translate_local_submit_no_selection_returns_changed_no_action() {
    use crate::views::question_view::{LocalQuestionKind, QuestionViewState};
    use codel_tools::implementations::codel_build::ask_user_question::{
        Question, QuestionOption,
    };
    let q = Question {
        question: "?".into(),
        options: (0..2)
            .map(|_| QuestionOption {
                label: "opt".into(),
                description: String::new(),
                preview: None,
                id: None,
            })
            .collect(),
        multi_select: Some(false),
        id: None,
    };
    let state = QuestionViewState::new(
        "x".into(),
        vec![q],
        crate::views::prompt_widget::StashedPrompt::default(),
    );
    let kind = LocalQuestionKind::Fork { directive: None };
    let outcome = crate::app::agent_view::translate_local_submit_for_test(&state, kind, false);
    assert!(matches!(
        outcome,
        crate::app::app_view::InputOutcome::Changed
    ));
}
#[test]
fn translate_local_submit_out_of_range_index_returns_changed_no_action() {
    use crate::views::question_view::{LocalQuestionKind, QuestionViewState};
    use codel_tools::implementations::codel_build::ask_user_question::{
        Question, QuestionOption,
    };
    let q = Question {
        question: "?".into(),
        options: (0..2)
            .map(|_| QuestionOption {
                label: "opt".into(),
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
    );
    let Some(slot) = state.selections.first_mut() else {
        panic!("expected first question selection");
    };
    *slot = crate::views::question_view::QuestionSelection::Single(Some(99));
    let kind = LocalQuestionKind::Fork { directive: None };
    let outcome = crate::app::agent_view::translate_local_submit_for_test(&state, kind, false);
    assert!(matches!(
        outcome,
        crate::app::app_view::InputOutcome::Changed
    ));
}
#[test]
fn handle_ask_user_question_does_not_push_system_block_when_displaced_acp_modal() {
    use crate::views::question_view::QuestionViewState;
    use codel_tools::implementations::codel_build::ask_user_question::{
        Question, QuestionOption,
    };
    let mut app = fork_test_app();
    let id = AgentId(0);
    let stashed = app.agents.get_mut(&id).unwrap().prompt.stash();
    let q = Question {
        question: "first ACP question?".into(),
        options: vec![QuestionOption {
            label: "ok".into(),
            description: "ok".into(),
            preview: None,
            id: None,
        }],
        multi_select: Some(false),
        id: None,
    };
    app.agents.get_mut(&id).unwrap().question_view =
        Some(QuestionViewState::new("first-acp".into(), vec![q], stashed));
    let scrollback_len_before = agent_ref(&app, id).scrollback.len();
    let (args, _rx) = make_ask_user_question_args("second-acp");
    let handled = crate::app::acp_handler::handle_ask_user_question(args, &mut app);
    assert!(handled);
    let qv = agent_ref(&app, id).question_view.as_ref().unwrap();
    assert_eq!(qv.tool_call_id, "second-acp");
    assert_eq!(
        agent_ref(&app, id).scrollback.len(),
        scrollback_len_before,
        "no system block when displaced modal was an ACP question, not a local one"
    );
}
#[test]
fn close_active_agent_no_parent_switches_to_first_surviving_peer() {
    let mut app = three_agent_app();
    switch_to_agent(&mut app, AgentId(1), SwitchCause::Picker);
    dispatch_sessions_confirm_close(&mut app, AgentId(1));
    assert!(matches!(app.active_view, ActiveView::Agent(id) if id == AgentId(0)));
    assert!(!app.agents.contains_key(&AgentId(1)));
}
#[test]
fn close_active_agent_with_dead_parent_falls_back_to_surviving_peer() {
    let mut app = three_agent_app();
    set_forked_from(&mut app, AgentId(2), AgentId(99));
    switch_to_agent(&mut app, AgentId(2), SwitchCause::Picker);
    dispatch_sessions_confirm_close(&mut app, AgentId(2));
    assert!(matches!(app.active_view, ActiveView::Agent(id) if id == AgentId(0)));
}
#[test]
fn entry_title_uses_display_name_when_set() {
    use crate::views::session_title::entry_title;
    let mut app = test_app_with_agent();
    if let Some(a) = app.agents.get_mut(&AgentId(0)) {
        a.display_name = Some("custom title".into());
    }
    let title = entry_title(agent_ref(&app, AgentId(0)));
    assert_eq!(title, "custom title");
}
#[test]
fn find_agent_by_session_id_returns_none_for_unknown() {
    let mut app = test_app_with_agent();
    assert!(find_agent_by_session_id(&mut app.agents, "nonexistent").is_none());
}
#[test]
fn find_agent_by_session_id_returns_none_when_session_id_is_none() {
    let mut app = test_app_with_agent();
    app.agents.get_mut(&AgentId(0)).unwrap().session.session_id = None;
    assert!(find_agent_by_session_id(&mut app.agents, "test-session").is_none());
}
#[test]
fn find_agent_by_session_id_finds_inactive_agent() {
    let mut app = two_agent_app_with_bg_task();
    let found = find_agent_by_session_id(&mut app.agents, "sess-B");
    assert!(found.is_some());
    assert_eq!(
        found.unwrap().session.session_id,
        Some(acp::SessionId::new("sess-B"))
    );
}
/// Verifies that the dispatcher routes each Action to its own setter (catches a copy-paste registration bug where two setters were swapped).
/// The original 5-setting matrix shrank to 2 after the user-feedback drop of `session_picker_grouped` / `load_envrc` / `use_leader`.
#[test]
fn pr13_each_setter_writes_to_its_own_mirror() {
    let mut app = test_app_with_agent();
    assert_eq!(app.show_tips, None);
    assert_eq!(app.auto_update, None);
    let _ = dispatch(Action::SetShowTips(false), &mut app);
    assert_eq!(app.show_tips, Some(false));
    assert_eq!(app.auto_update, None);
    let _ = dispatch(Action::SetAutoUpdate(false), &mut app);
    assert_eq!(app.auto_update, Some(false));
    assert_eq!(app.show_tips, Some(false));
}
/// The PAGER registry default must agree with `PagerLocalSnapshot::default()` and with `AgentView::new`'s runtime initializer.
/// `defaults_match_pager_state` in `registry::tests` covers the snapshot leg; the registry test alone can't see `AgentView::new`'s constant.
#[test]
fn pager_registry_default_matches_agent_view_new_initializer() {
    use crate::settings::{SettingKind, SettingOwner, SettingsRegistry};
    let app = test_app_with_agent();
    let agent = app
        .agents
        .get(&AgentId(0))
        .expect("test_app_with_agent must create AgentId(0)");
    let reg = SettingsRegistry::defaults();
    for meta in reg.all() {
        if meta.owner != SettingOwner::Pager {
            continue;
        }
        match (meta.key, &meta.kind) {
            ("multiline_mode", SettingKind::Bool { default }) => {
                assert_eq!(
                    *default, agent.multiline_mode,
                    "registry default for `multiline_mode` ({default}) drifts from \
                         AgentView::new's initializer ({}). Update one to match the \
                         other — the registry is the contract surface.",
                    agent.multiline_mode,
                );
            }
            ("plan_mode", SettingKind::Enum { default, .. }) => {
                let effective = agent.plan_mode_pending.unwrap_or(agent.plan_mode_active);
                let expected = if effective { "on" } else { "off" };
                assert_eq!(
                    *default, expected,
                    "registry default for `plan_mode` (`{default}`) drifts from \
                         AgentView::new's initializer (effective={effective} → \
                         expected `{expected}`). Update one to match the other — \
                         the registry is the contract surface.",
                );
            }
            ("respect_manual_folds", SettingKind::Bool { default }) => {
                let live = agent
                    .scrollback
                    .appearance()
                    .scrollback
                    .scroll
                    .respect_manual_folds;
                assert_eq!(
                    *default, live,
                    "registry default for `respect_manual_folds` ({default}) drifts \
                         from the agent's default appearance config ({live}). Update one \
                         to match the other — ScrollConfig::default() is the source of \
                         truth.",
                );
            }
            _ => {
                panic!(
                    "PAGER setting `{}` has no arm in \
                     pager_registry_default_matches_agent_view_new_initializer — \
                     add an arm that pins the registry default against the runtime \
                     initializer in `AgentView::new` (or, for future fields, the \
                     equivalent runtime construction site).",
                    meta.key,
                )
            }
        }
    }
}
/// Picking the regular "Yes, proceed" option (NOT enable-always-approve) must behave as before: no PersistPermissionMode effect, no YOLO flip.
/// Pins that the always-approve code path is gated strictly on the id check.
#[test]
fn regular_allow_once_does_not_trigger_always_approve_persist() {
    use std::sync::Arc;
    let mut app = test_app_with_agent();
    let _response_rx = enqueue_permission_with_enable_always_approve(&mut app);
    let effects = dispatch(
        Action::PermissionSelect(acp::PermissionOptionId::new(Arc::from("opt-allow-once"))),
        &mut app,
    );
    assert!(
        !effects
            .iter()
            .any(|e| matches!(e, Effect::PersistPermissionMode { .. })),
        "picking the regular AllowOnce option must NOT emit PersistPermissionMode — \
             the always-approve mode is opt-in via the dedicated option only",
    );
    assert!(
        !agent_ref(&app, AgentId(0)).session.is_yolo(),
        "session.yolo_mode must remain OFF when the regular AllowOnce option is picked",
    );
    assert!(
        !app.default_yolo,
        "app.default_yolo must remain OFF when the regular AllowOnce option is picked",
    );
}
#[test]
fn show_tasks_empty_commits_empty_message() {
    let mut app = test_app_with_agent();
    let before = agent_scrollback_len(&app);
    let effects = dispatch(Action::ShowTasks, &mut app);
    assert!(effects.is_empty(), "got: {effects:?}");
    assert_eq!(agent_scrollback_len(&app), before + 1);
    assert_eq!(
        last_system_text(&app, AgentId(0)),
        "No background tasks, workflows, or subagents."
    );
}
#[test]
fn show_tasks_lists_a_scheduled_task() {
    use crate::app::agent::ScheduledTaskInfo;
    let mut app = test_app_with_agent();
    {
        let agent = app.agents.get_mut(&AgentId(0)).unwrap();
        agent.session.scheduled_tasks.insert(
            "t1".to_string(),
            ScheduledTaskInfo {
                task_id: "t1".to_string(),
                prompt: "check CI status".to_string(),
                human_schedule: "every 5m".to_string(),
                created_at: std::time::Instant::now(),
                next_fire_at: None,
                tag: "loop".to_string(),
                last_subagent_id: None,
            },
        );
    }
    let effects = dispatch(Action::ShowTasks, &mut app);
    assert!(effects.is_empty(), "got: {effects:?}");
    let text = last_system_text(&app, AgentId(0));
    assert!(text.contains("Task (1):"), "got: {text:?}");
    assert!(
        text.contains("loop · every 5m · check CI status"),
        "got: {text:?}"
    );
    assert!(text.contains("scheduled"), "got: {text:?}");
}
#[test]
fn classify_top_level_branches() {
    use crate::views::dashboard::{RowState, classify_top_level};
    let mut app = test_app_with_agent();
    let agent = app.agents.get(&AgentId(0)).unwrap();
    assert_eq!(classify_top_level(agent), RowState::Idle);
    let agent = app.agents.get_mut(&AgentId(0)).unwrap();
    agent.session.state = AgentState::TurnRunning;
    assert_eq!(classify_top_level(agent), RowState::Working);
    agent.session.state = AgentState::Idle;
    agent.session.loading_replay = true;
    assert_eq!(classify_top_level(agent), RowState::Working);
    agent.session.loading_replay = false;
}
/// For Idle rows, `last_change_at` is the frozen `last_active_at` anchor.
/// Building the row twice in rapid succession against a fixed `last_active_at` yields nearly-identical `elapsed()` values (within tolerance).
#[test]
fn build_rows_idle_anchor_is_frozen_last_active_at() {
    use crate::views::dashboard::build_rows_with_roster;
    let mut app = test_app_with_agent();
    mark_agent_nonempty(&mut app, AgentId(0));
    let agent = app.agents.get_mut(&AgentId(0)).unwrap();
    let anchor = std::time::Instant::now() - std::time::Duration::from_secs(300);
    agent.last_active_at = Some(anchor);
    agent.turn_started_at = None;
    agent.session.state = crate::app::agent::AgentState::Idle;
    let rows1 = build_rows_with_roster(
        &app.agents,
        &std::collections::BTreeSet::new(),
        &[],
        crate::views::dashboard::Grouping::State,
        &crate::views::dashboard::Filter::None,
        None,
        &[],
    );
    let Some(row1) = rows1.first() else {
        panic!("expected an idle dashboard row: {rows1:?}");
    };
    assert_eq!(row1.state, crate::views::dashboard::RowState::Idle);
    let elapsed1 = row1.last_change_at.elapsed().unwrap_or_default();
    assert!(
        elapsed1 >= std::time::Duration::from_secs(299),
        "expected >= 299s, got {elapsed1:?}",
    );
    let rows2 = build_rows_with_roster(
        &app.agents,
        &std::collections::BTreeSet::new(),
        &[],
        crate::views::dashboard::Grouping::State,
        &crate::views::dashboard::Filter::None,
        None,
        &[],
    );
    let Some(row2) = rows2.first() else {
        panic!("expected an idle dashboard row: {rows2:?}");
    };
    let elapsed2 = row2.last_change_at.elapsed().unwrap_or_default();
    assert!(
        elapsed2 >= std::time::Duration::from_secs(299),
        "idle anchor must stay frozen across rebuilds, got {elapsed2:?}",
    );
}
/// Working rows anchor at `turn_started_at`.
/// The age column then shows the LIVE elapsed time within the current turn rather than the time since the previous turn ended.
#[test]
fn build_rows_working_anchor_is_turn_started_at() {
    use crate::views::dashboard::build_rows_with_roster;
    let mut app = test_app_with_agent();
    let agent = app.agents.get_mut(&AgentId(0)).unwrap();
    let turn_start = std::time::Instant::now() - std::time::Duration::from_secs(5);
    let stale = std::time::Instant::now() - std::time::Duration::from_secs(600);
    agent.turn_started_at = Some(turn_start);
    agent.last_active_at = Some(stale);
    agent.session.state = crate::app::agent::AgentState::TurnRunning;
    let rows = build_rows_with_roster(
        &app.agents,
        &std::collections::BTreeSet::new(),
        &[],
        crate::views::dashboard::Grouping::State,
        &crate::views::dashboard::Filter::None,
        None,
        &[],
    );
    let Some(row) = rows.first() else {
        panic!("expected a working dashboard row: {rows:?}");
    };
    assert_eq!(row.state, crate::views::dashboard::RowState::Working);
    let elapsed = row.last_change_at.elapsed().unwrap_or_default();
    assert!(
        elapsed >= std::time::Duration::from_secs(4)
            && elapsed < std::time::Duration::from_secs(30),
        "expected ~5s (turn_started_at anchor), got {elapsed:?}",
    );
}
/// Defensive test for the fallback path when both `turn_started_at` and `last_active_at` are `None`.
/// The row's `last_change_at` projects the *frozen* process-wide `fallback_epoch`.
/// Two consecutive builds therefore yield stable values (within sampling jitter) rather than re-anchoring at `now` and showing "0s" every frame.
#[test]
fn build_rows_fallback_anchor_is_frozen_when_last_active_at_is_none() {
    use crate::views::dashboard::build_rows_with_roster;
    let mut app = test_app_with_agent();
    mark_agent_nonempty(&mut app, AgentId(0));
    let agent = app.agents.get_mut(&AgentId(0)).unwrap();
    agent.last_active_at = None;
    agent.turn_started_at = None;
    agent.session.state = crate::app::agent::AgentState::Idle;
    let rows1 = build_rows_with_roster(
        &app.agents,
        &std::collections::BTreeSet::new(),
        &[],
        crate::views::dashboard::Grouping::State,
        &crate::views::dashboard::Filter::None,
        None,
        &[],
    );
    let rows2 = build_rows_with_roster(
        &app.agents,
        &std::collections::BTreeSet::new(),
        &[],
        crate::views::dashboard::Grouping::State,
        &crate::views::dashboard::Filter::None,
        None,
        &[],
    );
    let (Some(r1), Some(r2)) = (rows1.first(), rows2.first()) else {
        panic!("expected dashboard rows: {rows1:?} {rows2:?}");
    };
    let (t1, t2) = (r1.last_change_at, r2.last_change_at);
    let drift = t1.duration_since(t2).unwrap_or_else(|e| e.duration());
    assert!(
        drift < std::time::Duration::from_secs(1),
        "fallback anchor must be frozen across rebuilds, drifted {drift:?}",
    );
}
/// While the turn is IDLE the scrollback scan gives the peek header label the TYPE of the most recent agent block (Response / Edit / Thought / …).
/// The most recent block wins; a fresh user prompt is a turn boundary with no agent response after it yet, so the label is "Idle".
/// (The RUNNING case follows live turn activity; see the `extract_response_type_*` tests.)
#[serial_test::serial(CODEL_AGENT_DASHBOARD)]
#[test]
fn peek_label_reflects_last_response_type() {
    use crate::scrollback::block::RenderBlock;
    use crate::views::dashboard::peek::extract_last_response_type;
    let mut app = test_app_with_agent();
    let agent = app.agents.get_mut(&AgentId(0)).unwrap();
    agent
        .scrollback
        .push_block(RenderBlock::agent_message("hi"));
    assert_eq!(extract_last_response_type(agent), "Response");
    agent
        .scrollback
        .push_block(RenderBlock::tool_call("edit", "src/x.rs", true));
    assert_eq!(extract_last_response_type(agent), "Edit");
    agent.scrollback.push_block(RenderBlock::thinking("hmm"));
    assert_eq!(extract_last_response_type(agent), "Thought");
    agent
        .scrollback
        .push_block(RenderBlock::user_prompt("do it"));
    assert_eq!(extract_last_response_type(agent), "Idle");
}
/// agent.question_view.is_some() classifies as NeedsInput.
#[test]
fn classify_top_level_question_view_some_is_needs_input() {
    use crate::views::dashboard::{RowState, classify_top_level};
    use crate::views::question_view::QuestionViewState;
    let mut app = test_app_with_agent();
    let agent = app.agents.get_mut(&AgentId(0)).unwrap();
    agent.question_view = Some(QuestionViewState::new(
        "tc-1".to_string(),
        Vec::new(),
        crate::views::prompt_widget::StashedPrompt::default(),
    ));
    assert_eq!(classify_top_level(agent), RowState::NeedsInput);
}
/// ANSI escapes in `display_name` are stripped at row build time.
#[test]
fn top_level_label_strips_control_characters() {
    use crate::views::dashboard::build_rows_with_roster;
    let mut app = test_app_with_agent();
    let agent = app.agents.get_mut(&AgentId(0)).unwrap();
    agent.display_name = Some("a\x1b[31mevil\x1b[0m".to_string());
    let rows = build_rows_with_roster(
        &app.agents,
        &std::collections::BTreeSet::new(),
        &[],
        crate::views::dashboard::Grouping::State,
        &crate::views::dashboard::Filter::None,
        None,
        &[],
    );
    let top = rows.first().expect("top row");
    assert!(
        !top.label.contains('\x1b'),
        "label must not retain \\x1b: {:?}",
        top.label
    );
    assert!(top.label.contains("evil"));
}
fn mouse_event(
    kind: crossterm::event::MouseEventKind,
    col: u16,
    row: u16,
) -> crossterm::event::MouseEvent {
    crossterm::event::MouseEvent {
        kind,
        column: col,
        row,
        modifiers: crossterm::event::KeyModifiers::NONE,
    }
}
/// A single left-click on a row selects it and attaches the conversation immediately.
#[serial_test::serial(CODEL_AGENT_DASHBOARD)]
#[test]
fn mouse_left_click_attaches_immediately() {
    use crossterm::event::{Event, MouseButton, MouseEventKind};
    let mut app = test_app_with_agent();
    open_dashboard(&mut app);
    let d = app.dashboard.as_mut().unwrap();
    let id = crate::views::dashboard::DashboardRowId::TopLevel(AgentId(0));
    d.row_rects
        .push((id.clone(), ratatui::layout::Rect::new(0, 5, 80, 1)));
    d.selected = None;
    let outcome = d.handle_input(
        &Event::Mouse(mouse_event(MouseEventKind::Down(MouseButton::Left), 10, 5)),
        &crate::actions::ActionRegistry::defaults(),
    );
    match outcome {
        crate::app::app_view::InputOutcome::Action(
            crate::app::actions::Action::DashboardAttach(actual),
        ) => {
            assert_eq!(actual, id, "single click must attach the clicked row");
        }
        other => panic!("expected DashboardAttach on single click, got {other:?}"),
    }
    assert_eq!(d.selected, Some(id));
}
/// Every left-click attaches, including rapid repeated clicks.
/// The previous design used a 500ms window to distinguish single (select) from double (attach) click.
/// Now every click attaches, so the user's mental model "click = open" always holds.
#[serial_test::serial(CODEL_AGENT_DASHBOARD)]
#[test]
fn mouse_repeated_click_keeps_attaching() {
    use crossterm::event::{Event, MouseButton, MouseEventKind};
    let mut app = test_app_with_agent();
    open_dashboard(&mut app);
    let d = app.dashboard.as_mut().unwrap();
    let id = crate::views::dashboard::DashboardRowId::TopLevel(AgentId(0));
    d.row_rects
        .push((id.clone(), ratatui::layout::Rect::new(0, 5, 80, 1)));
    let reg = crate::actions::ActionRegistry::defaults();
    let outcome1 = d.handle_input(
        &Event::Mouse(mouse_event(MouseEventKind::Down(MouseButton::Left), 10, 5)),
        &reg,
    );
    match outcome1 {
        crate::app::app_view::InputOutcome::Action(
            crate::app::actions::Action::DashboardAttach(actual),
        ) => assert_eq!(actual, id),
        other => panic!("expected DashboardAttach on first click, got {other:?}"),
    }
    let outcome2 = d.handle_input(
        &Event::Mouse(mouse_event(MouseEventKind::Down(MouseButton::Left), 10, 5)),
        &reg,
    );
    match outcome2 {
        crate::app::app_view::InputOutcome::Action(
            crate::app::actions::Action::DashboardAttach(actual),
        ) => assert_eq!(actual, id),
        other => panic!("expected DashboardAttach on second click, got {other:?}"),
    }
}
/// Clicks after the previous 500ms double-click window also attach.
/// (The previous test asserted single-click behaviour for clicks more than 500ms apart; now every click attaches.)
#[serial_test::serial(CODEL_AGENT_DASHBOARD)]
#[test]
fn mouse_click_after_long_pause_still_attaches() {
    use crossterm::event::{Event, MouseButton, MouseEventKind};
    use std::time::{Duration, Instant};
    let mut app = test_app_with_agent();
    open_dashboard(&mut app);
    let d = app.dashboard.as_mut().unwrap();
    let id = crate::views::dashboard::DashboardRowId::TopLevel(AgentId(0));
    d.row_rects
        .push((id.clone(), ratatui::layout::Rect::new(0, 5, 80, 1)));
    let reg = crate::actions::ActionRegistry::defaults();
    let _ = d.handle_input(
        &Event::Mouse(mouse_event(MouseEventKind::Down(MouseButton::Left), 10, 5)),
        &reg,
    );
    if let Some((_, t)) = d.last_click.as_mut() {
        *t = Instant::now() - Duration::from_millis(600);
    }
    let outcome = d.handle_input(
        &Event::Mouse(mouse_event(MouseEventKind::Down(MouseButton::Left), 10, 5)),
        &reg,
    );
    match outcome {
        crate::app::app_view::InputOutcome::Action(
            crate::app::actions::Action::DashboardAttach(actual),
        ) => assert_eq!(actual, id),
        other => panic!("expected DashboardAttach, got {other:?}"),
    }
}
/// Click on the peek close-button rect closes the peek.
#[serial_test::serial(CODEL_AGENT_DASHBOARD)]
#[test]
fn mouse_click_on_peek_close_rect_clears_peek() {
    use crossterm::event::{Event, MouseButton, MouseEventKind};
    let mut app = test_app_with_agent();
    open_dashboard(&mut app);
    let d = app.dashboard.as_mut().unwrap();
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
    d.peek_close_rect = Some(ratatui::layout::Rect::new(70, 8, 3, 1));
    let reg = crate::actions::ActionRegistry::defaults();
    let outcome = d.handle_input(
        &Event::Mouse(mouse_event(MouseEventKind::Down(MouseButton::Left), 71, 8)),
        &reg,
    );
    assert!(matches!(
        outcome,
        crate::app::app_view::InputOutcome::Changed
    ));
    assert!(d.peek.is_none());
    assert!(d.peek_close_rect.is_none());
}
/// Same refusal at the content-hit worktree entry point.
#[test]
fn pick_content_session_in_worktree_refuses_conversation_row() {
    let mut app = test_app_with_agent();
    open_session_picker_with(&mut app, vec![make_conversation_entry("conv-wt-2")]);
    let effects = dispatch(
        Action::PickContentSessionInWorktree {
            session_id: "conv-wt-2".into(),
            cwd: String::new(),
        },
        &mut app,
    );
    assert!(effects.is_empty(), "no worktree effects, got {effects:?}");
    assert!(
        !app.deferred_startup.pending_chat,
        "refusal must not set the one-shot chat bit"
    );
    assert!(read_toast(&app).contains("worktree"));
}
/// Delete acts on local disk and registry; conversation rows have neither.
#[test]
fn delete_session_refuses_conversation_row() {
    let mut app = test_app_with_agent();
    open_session_picker_with(&mut app, vec![make_conversation_entry("conv-del-1")]);
    let effects = dispatch(
        Action::DeleteSession {
            source: "conversation".into(),
            session_id: "conv-del-1".into(),
            cwd: String::new(),
        },
        &mut app,
    );
    assert!(
        effects.is_empty(),
        "no DeleteSession effect for a conversation row, got {effects:?}"
    );
    assert!(read_toast(&app).contains("isn't supported"));
}
#[test]
fn delete_session_refuses_known_read_only_workspace_member() {
    let mut app = test_app_with_agent();
    open_session_picker_with(&mut app, vec![make_picker_entry("read-only-delete", "/r")]);
    app.workspace_dashboard_enabled = true;
    let temp = tempfile::tempdir().unwrap();
    let store =
        codel_dashboard_store::WorkspaceStore::open(&temp.path().join("workspace.db")).unwrap();
    app.workspace_membership.set_read_only_for_test(
        store,
        codel_dashboard_store::WorkspaceSnapshot {
            grouping: codel_dashboard_store::Grouping::State,
            members: vec![codel_dashboard_store::Member {
                session_id: codel_dashboard_store::SessionId::new("read-only-delete").unwrap(),
                kind: codel_dashboard_store::MemberKind::Build,
                origin: codel_dashboard_store::MemberOrigin::Local,
                cwd: Some("/r".into()),
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
        Action::DeleteSession {
            source: "local".into(),
            session_id: "read-only-delete".into(),
            cwd: "/r".into(),
        },
        &mut app,
    );
    assert!(effects.is_empty());
    assert!(read_toast(&app).contains("workspace is read-only"));
}
/// Expanding a conversation card must not read `chat_history.jsonl` (it doesn't exist); the row still toggles open.
#[test]
fn expand_conversation_card_skips_detail_load() {
    use crate::views::modal::ActiveModal;
    let mut app = test_app_with_agent();
    open_session_picker_with(&mut app, vec![make_conversation_entry("conv-exp-1")]);
    let effects = dispatch(
        Action::ExpandSessionCard {
            source: "conversation".into(),
            session_id: "conv-exp-1".into(),
        },
        &mut app,
    );
    assert!(
        effects.is_empty(),
        "no LoadCardDetail for a conversation row, got {effects:?}"
    );
    let agent = get_active_agent(&app).expect("active agent");
    let Some(ActiveModal::SessionPicker { state, .. }) = agent.active_modal.as_ref() else {
        panic!("expected SessionPicker modal");
    };
    assert!(state.expanded.contains(&0), "row still toggles open");
}
/// Canary: Build rows still lazy-load card detail on expand.
#[test]
fn expand_build_card_still_loads_detail() {
    let mut app = test_app_with_agent();
    open_session_picker_with(&mut app, vec![make_picker_entry("local-exp-1", "/r")]);
    let effects = dispatch(
        Action::ExpandSessionCard {
            source: "local".into(),
            session_id: "local-exp-1".into(),
        },
        &mut app,
    );
    assert!(
        matches!(effects.as_slice(), [Effect::LoadCardDetail { .. }]),
        "expected LoadCardDetail, got {effects:?}"
    );
}
fn system_texts(app: &AppView, id: AgentId) -> Vec<String> {
    agent_ref(app, id)
        .scrollback
        .iter_entries()
        .filter_map(|(_, e)| match &e.block {
            crate::scrollback::block::RenderBlock::System(s) => Some(s.text.clone()),
            _ => None,
        })
        .collect()
}
/// `/debug` bare: one system line reporting every toggle's state.
#[test]
fn show_debug_status_emits_toggle_states_to_transcript() {
    let mut app = test_app_with_agent();
    let id = AgentId(0);
    if app.scroll_state.scroll_log_active() {
        let _ = app.scroll_state.toggle_scroll_log();
    }
    let _ = dispatch(Action::ToggleFpsHud, &mut app);
    let _ = dispatch(Action::ShowDebugStatus, &mut app);
    let texts = system_texts(&app, id);
    let status = texts
        .iter()
        .find(|t| t.starts_with("debug toggles:"))
        .unwrap_or_else(|| panic!("status line missing, got {texts:?}"));
    assert!(status.contains("fps on"), "got: {status}");
    assert!(status.contains("log off"), "got: {status}");
    assert!(
        status.contains("/debug"),
        "must point at the command: {status}"
    );
}
/// `/debug log`: the toggle flips the recorder and echoes where it writes.
#[test]
fn toggle_scroll_log_flips_recorder_and_reports_path() {
    let mut app = test_app_with_agent();
    let id = AgentId(0);
    if app.scroll_state.scroll_log_active() {
        let _ = app.scroll_state.toggle_scroll_log();
    }
    let _ = dispatch(Action::ToggleScrollLog, &mut app);
    assert!(app.scroll_state.scroll_log_active());
    let _ = dispatch(Action::ToggleScrollLog, &mut app);
    assert!(!app.scroll_state.scroll_log_active());
    let texts = system_texts(&app, id);
    assert!(
        texts
            .iter()
            .any(|t| t.starts_with("scroll log: recording to")),
        "enable must echo the log path, got {texts:?}"
    );
    assert!(
        texts.iter().any(|t| t == "scroll log: off"),
        "disable must be confirmed, got {texts:?}"
    );
}
#[serial_test::serial(CODEL_TEST_OPEN_URL_FILE)]
#[test]
fn open_managed_connectors_starts_wait_when_modal_open() {
    use crate::views::extensions_modal::{ExtensionsModalState, ExtensionsTab};
    let url_file = std::env::temp_dir().join(format!(
        "codel-managed-connectors-open-{}.txt",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&url_file);
    unsafe { std::env::set_var("CODEL_TEST_OPEN_URL_FILE", &url_file) };
    let mut app = test_app_with_agent();
    let id = AgentId(0);
    app.agents.get_mut(&id).unwrap().extensions_modal =
        Some(ExtensionsModalState::new(ExtensionsTab::McpServers));
    let effects = dispatch(Action::OpenManagedConnectors, &mut app);
    assert!(effects.is_empty());
    assert!(
        agent_ref(&app, id)
            .extensions_modal
            .as_ref()
            .is_some_and(|modal| modal.is_managed_connectors_wait())
    );
    let recorded = std::fs::read_to_string(&url_file).unwrap_or_default();
    assert!(
        recorded
            .lines()
            .any(|line| line == crate::views::mcps_modal::managed_connectors_url(None)),
        "opener seam must record the connectors URL; got {recorded:?}"
    );
    unsafe { std::env::remove_var("CODEL_TEST_OPEN_URL_FILE") };
    let _ = std::fs::remove_file(&url_file);
}
#[test]
fn refresh_mcp_list_clears_managed_connectors_wait() {
    use crate::views::extensions_modal::{ExtensionsModalState, ExtensionsTab, TabDataState};
    let mut app = test_app_with_agent();
    let id = AgentId(0);
    {
        let agent = app.agents.get_mut(&id).unwrap();
        let mut modal = ExtensionsModalState::new(ExtensionsTab::McpServers);
        modal.begin_managed_connectors_wait();
        agent.extensions_modal = Some(modal);
    }
    let effects = dispatch(Action::RefreshMcpList, &mut app);
    let modal = agent_ref(&app, id)
        .extensions_modal
        .as_ref()
        .expect("modal stays open");
    assert!(!modal.is_managed_connectors_wait());
    assert!(matches!(modal.mcps_data, TabDataState::Loading));
    assert!(matches!(
        effects.as_slice(),
        [Effect::FetchMcpsList { cache: false, .. }]
    ));
}
