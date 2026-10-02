//! Tests for session lifecycle, loading, pickers, modals, forking, and trust.

use super::*;

mod foreign;
mod fork;
mod lifecycle;
mod load;
mod modal;
mod optimistic_home;
mod take_deferred;

fn content_hit(id: &str) -> codel_shell::extensions::session_search::SearchSessionHit {
    codel_shell::extensions::session_search::SearchSessionHit {
        session_id: id.into(),
        summary: id.into(),
        cwd: "/repo".into(),
        updated_at: chrono::Utc::now().to_rfc3339(),
        snippet: Some("native transcript match".into()),
        score: 1.0,
        matched_fields: vec![],
    }
}

fn count_extension_fetches(effects: &[Effect]) -> usize {
    effects
        .iter()
        .filter(|e| {
            matches!(
                e,
                Effect::FetchHooksList { .. }
                    | Effect::FetchPluginsList { .. }
                    | Effect::FetchMarketplaceList { .. }
                    | Effect::FetchMcpsList { .. }
                    | Effect::FetchSkillsList { .. }
            )
        })
        .count()
}

/// Build a single-agent app for the `/new` dispatcher tests.
/// Sets `current_branch` to `Some("main")` so the agent appears to be inside a git repo (mirrors `fork_test_app`).
fn new_session_test_app() -> AppView {
    let mut app = test_app_with_agent();
    app.agents.get_mut(&AgentId(0)).unwrap().current_branch = Some("main".into());
    app.cwd_has_git_ancestor = true;
    app
}
