//! GBT-6212: create the session when the TUI opens; the first interaction reveals it.
use super::*;
use crate::app::app_view::{InputOutcome, PasteProvenance};
use crate::app::dispatch::session::lifecycle::{
    handle_session_created, handle_session_failed, handle_worktree_session_failed,
    maybe_create_home_session,
};
use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
fn key_event(code: KeyCode, mods: KeyModifiers) -> Event {
    Event::Key(KeyEvent {
        code,
        modifiers: mods,
        kind: KeyEventKind::Press,
        state: crossterm::event::KeyEventState::NONE,
    })
}
/// Feed `ev` to Welcome, require `ActionThenForward(LeaveHome)`, and run it through the event loop's forward path.
fn leave_home_with(app: &mut AppView, ev: &Event) -> Vec<Effect> {
    let outcome = app.handle_input(ev);
    let InputOutcome::ActionThenForward(Action::LeaveHome) = outcome else {
        panic!("expected ActionThenForward(LeaveHome), got {outcome:?}");
    };
    crate::app::event_loop::dispatch_then_forward(
        Action::LeaveHome,
        ev,
        std::time::Instant::now(),
        PasteProvenance::Terminal,
        app,
    )
}
fn creates_session(effects: &[Effect]) -> bool {
    effects
        .iter()
        .any(|e| matches!(e, Effect::CreateSession { .. }))
}

/// The real Welcome-stays route: chat mode (no husk) + Local workspace without
/// an ACK opens the y/N prompt. The forwarded `y` must not confirm it.
#[cfg(feature = "local-workspace")]
#[test]
#[serial_test::serial(CODEL_CHAT_LOCAL_WORKSPACE_ACK)]
fn leave_home_into_local_workspace_ack_keeps_the_keystroke_as_a_draft() {
    let _ack = codel_test_support::EnvGuard::unset(
        crate::app::session_startup::CODEL_CHAT_LOCAL_WORKSPACE_ACK_ENV,
    );
    let home = tempfile::tempdir().unwrap();
    let _home = codel_test_support::EnvGuard::set("CODEL_HOME", home.path().to_str().unwrap());
    crate::app::session_startup::set_active_local_workspace(None).unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let mut app = test_app();
    app.chat_mode = true;
    app.cwd = tmp.path().to_path_buf();
    app.welcome_workspace_mode = crate::views::welcome::WelcomeWorkspaceMode::LocalWorkspace;
    assert!(maybe_create_home_session(&mut app).is_empty());
    let effects = leave_home_with(&mut app, &key_event(KeyCode::Char('y'), KeyModifiers::NONE));
    assert!(matches!(app.active_view, ActiveView::Welcome));
    assert!(app.welcome_local_workspace_ack_pending);
    assert!(!creates_session(&effects), "got {effects:?}");
    assert!(app.agents.is_empty());
    assert_eq!(app.welcome_prompt.text(), "y");
}

fn bind_home_session(app: &mut AppView) {
    let home = app.home_session_agent.expect("home session");
    let _ = handle_session_created(app, home, acp::SessionId::new("home-sid"), None, None);
}
