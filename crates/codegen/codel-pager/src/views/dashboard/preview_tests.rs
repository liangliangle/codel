use super::*;
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use ratatui::buffer::Buffer;

use crate::actions::ActionRegistry;
use crate::app::actions::Action;
use crate::app::agent_view::test_fixtures::make_agent;
use crate::app::app_view::InputOutcome;
use crate::views::dashboard::WorkspaceRowInputs;
use crate::views::dashboard::peek::PeekFields;
use crate::views::dashboard::render::render_dashboard;


#[test]
fn disabling_preview_closes_hidden_question_and_routes_typing_to_dispatch() {
    let mut agents = IndexMap::from([(AgentId(0), make_agent())]);
    let mut state = DashboardState::new();
    let row = DashboardRowId::TopLevel(AgentId(0));
    state.focus_row(row.clone());
    state.set_peek(Some(PeekPanelState::new(
        row.clone(),
        PeekFields {
            label: "session".to_owned(),
            time_ago: String::new(),
            response_type: "Permission".to_owned(),
            last_user_message: None,
            question: Some("Allow command?".to_owned()),
            options: vec![("allow".to_owned(), "Allow".to_owned())],
            request_id: Some(1),
            reject_option: None,
        },
    )));

    let agent = agents.get_mut(&AgentId(0)).expect("agent");
    agent
        .scrollback
        .set_view_mode(crate::scrollback::state::ViewMode::SingleTurn);
    let original_viewport = agent.scrollback.capture_viewport_snapshot();
    state.begin_peek_viewport(row.clone(), &mut agents);

    state.peek_reply.set_text("hidden reply");
    state.peek_close_rect = Some(Rect::new(0, 0, 1, 1));
    state.peek_reply_rect = Some(Rect::new(0, 0, 20, 1));
    state.file_search_dropdown_items_area = Some(Rect::new(0, 1, 20, 4));
    state.deferred_peek_send =
        Some(crate::views::dashboard::state::DeferredPeekSend { row, attach: false });
    state.search_mode = true;

    state.set_preview_enabled(false, &mut agents);

    assert_eq!(
        original_viewport,
        agents
            .get(&AgentId(0))
            .expect("agent")
            .scrollback
            .capture_viewport_snapshot()
    );
    assert!(state.search_mode);
    assert!(state.peek.is_none());
    assert!(state.peek_viewport.is_none());
    assert!(state.peek_close_rect.is_none());
    assert!(state.peek_reply_rect.is_none());
    assert!(state.file_search_dropdown_items_area.is_none());
    assert!(state.deferred_peek_send.is_none());
    assert!(state.peek_reply.text().is_empty());

    state.search_mode = false;

    let outcome = state.handle_input(
        &Event::Key(KeyEvent::new(KeyCode::Char('1'), KeyModifiers::NONE)),
        &ActionRegistry::defaults(),
    );

    assert!(!matches!(
        outcome,
        InputOutcome::Action(Action::DashboardPermissionSelect { .. })
    ));
    assert_eq!("1", state.dispatch.text());
}
