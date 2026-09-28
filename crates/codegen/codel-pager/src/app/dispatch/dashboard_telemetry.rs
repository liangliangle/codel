use crate::app::app_view::AppView;
use crate::views::dashboard::DashboardRowId;
use codel_logging::events::{
    DashboardAgentAttached, DashboardAgentLaunched, DashboardClosed, DashboardOpened,
};
use codel_logging::session_ctx::log_event;

pub(super) fn log_dashboard_opened(app: &AppView) {
    let subagents: usize = app.agents.values().map(|a| a.subagent_sessions.len()).sum();
}

pub(super) fn log_dashboard_closed(app: &AppView) {}

pub(super) fn log_dashboard_attached(id: &DashboardRowId) {
    let kind = match id {
        DashboardRowId::TopLevel(_) => "top_level",
        DashboardRowId::Roster { .. } => "roster",
        DashboardRowId::Workspace { .. } => "workspace",
    };
}

pub(super) fn log_dashboard_launched(source: &'static str) {}
