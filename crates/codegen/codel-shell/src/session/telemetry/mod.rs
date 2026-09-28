//! Session-level telemetry helpers: product analytics and harness snapshots.

mod active_agent_message;
mod permission;
mod read_profile;
mod tool_call;

pub(crate) use read_profile::model_origin;
#[cfg(not(feature = "test-support"))]
pub(crate) use tool_call::tool_execution_span;
pub(crate) use tool_call::{
    CompletedTool, PreparedToolFacts, ToolCallProjection, ToolExecutionInput, coarse_span_outcome,
    completed_event, record_tool_execution, requested_model_snapshot, tool_identity,
};
#[cfg(feature = "test-support")]
pub use tool_call::{complete_projected_call, grep_output, tool_execution_span};

pub(crate) use active_agent_message::*;
pub(crate) use permission::*;

use codel_tools::implementations::skills::types::SkillScope;

/// `duration_ms`, `tool_count`, and `error_type` are status-specific; pass `None` when not applicable.
pub(crate) fn emit_mcp_connection_span(
    status: &str,
    server_name: &str,
    transport_type: &str,
    server_scope: &str,
    duration_ms: Option<i64>,
    tool_count: Option<i64>,
    error_type: Option<&str>,
) {
    let span = tracing::info_span!(
        "mcp.server_connection",
        status,
        server_name,
        transport_type,
        server_scope,
        duration_ms = tracing::field::Empty,
        tool_count = tracing::field::Empty,
        error_type = tracing::field::Empty,
    );
    if let Some(d) = duration_ms {
        span.record("duration_ms", d);
    }
    if let Some(t) = tool_count {
        span.record("tool_count", t);
    }
    if let Some(e) = error_type {
        span.record("error_type", e);
    }
    span.in_scope(|| {});
}

/// Plugin id is `plugin_source`; SkillScope on a plugin skill is install location.
pub(crate) fn skill_source(scope: SkillScope, plugin_name: Option<&str>) -> &'static str {
    if plugin_name.is_some() {
        return "plugin";
    }
    match scope {
        SkillScope::Local => "local",
        SkillScope::Repo => "repo",
        SkillScope::User => "user",
        SkillScope::Server => "server",
        SkillScope::Bundled => "bundled",
        SkillScope::Plugin => "plugin",
    }
}

/// Canonicalizes both paths; one that cannot be canonicalized (synthetic paths like `chat-product://`) matches only when identical.
pub(crate) fn is_same_skill_file(
    skill_path: &std::path::Path,
    read_path: &std::path::Path,
) -> bool {
    if skill_path == read_path {
        return true;
    }
    match (
        dunce::canonicalize(skill_path),
        dunce::canonicalize(read_path),
    ) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

pub(crate) fn format_hook_name(spec: &codel_hooks::config::HookSpec) -> String {
    let scope = spec.name.split(':').next().unwrap_or("unknown");
    match spec.configured_matcher.as_deref() {
        Some(m) if !m.is_empty() => format!("{scope}:{}:{}", spec.event, m.to_lowercase()),
        _ => format!("{scope}:{}", spec.event),
    }
}

/// Provenance for telemetry, mapped from the shared [`hook_origin`] classifier so this and `/hooks` inspect can't diverge.
fn format_hook_source(spec: &codel_hooks::config::HookSpec) -> &'static str {
    use codel_hooks::config::HookOrigin as O;
    match codel_hooks::config::hook_origin(spec) {
        O::SystemManaged | O::Managed => "managedConfig",
        O::Requirements => "requirementsConfig",
        O::UserConfig => "userConfig",
        O::UserFile => "userSettings",
        O::ProjectFile => "projectSettings",
        O::Plugin => "pluginHook",
        O::Agent => "agentHook",
        O::Unknown => "unknown",
    }
}

/// Per-hook inventory recorded as a `hook.registered` span at session start.
pub(crate) struct HookRegInfo {
    pub name: String,
    pub event: String,
    pub hook_type: String,
    pub source: &'static str,
}

impl HookRegInfo {
    pub(crate) fn from_spec(spec: &codel_hooks::config::HookSpec) -> Self {
        Self {
            name: format_hook_name(spec),
            event: spec.event.to_string(),
            hook_type: spec.handler_type.as_ref().to_string(),
            source: format_hook_source(spec),
        }
    }
}

#[derive(Debug)]
#[cfg(test)]
mod is_same_skill_file_tests {
    use super::is_same_skill_file;
    use std::path::Path;

    #[test]
    fn matches_identical_paths() {
        assert!(is_same_skill_file(
            Path::new("/home/u/.codel/skills/review/SKILL.md"),
            Path::new("/home/u/.codel/skills/review/SKILL.md")
        ));
    }

    #[test]
    fn rejects_a_different_skill() {
        assert!(!is_same_skill_file(
            Path::new("/home/u/.codel/skills/review/SKILL.md"),
            Path::new("/home/u/.codel/skills/design/SKILL.md")
        ));
    }

    #[cfg(unix)]
    #[test]
    fn matches_through_a_symlink() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real");
        std::fs::create_dir(&real).unwrap();
        let skill = real.join("SKILL.md");
        std::fs::write(&skill, "---\nname: x\n---\n").unwrap();
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();

        assert!(is_same_skill_file(&skill, &link.join("SKILL.md")));
    }

    #[test]
    fn synthetic_product_path_does_not_match_a_real_file() {
        let dir = tempfile::tempdir().unwrap();
        let skill = dir.path().join("SKILL.md");
        std::fs::write(&skill, "body").unwrap();

        assert!(!is_same_skill_file(
            Path::new("chat-product://commit"),
            &skill
        ));
    }
}

#[cfg(test)]
mod skill_source_tests {
    use super::skill_source;
    use codel_tools::implementations::skills::types::SkillScope;

    #[test]
    fn plugin_name_overrides_install_location_scope() {
        assert_eq!("plugin", skill_source(SkillScope::User, Some("acme")));
        assert_eq!("bundled", skill_source(SkillScope::Bundled, None));
        assert_eq!("plugin", skill_source(SkillScope::Plugin, None));
    }
}
