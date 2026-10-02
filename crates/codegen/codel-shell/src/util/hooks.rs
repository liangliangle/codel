use std::path::Path;

use codel_hooks::discovery::DiscoveryOptions;
use codel_hooks::error::HookError;
use codel_hooks::trust::DisabledHooks;
use codel_hooks::trust::Trust;
use codel_workspace::hook_inputs::ProcessHookInputs;
use codel_workspace::permission::resolution::managed_settings;

pub(crate) fn process_hook_inputs() -> ProcessHookInputs {
    ProcessHookInputs::read(crate::claude_import::import_marker())
}

/// For a session that dispatches the hooks it discovers.
pub(crate) fn session_hook_inputs() -> (ProcessHookInputs, DisabledHooks) {
    ProcessHookInputs::read_with_disabled(managed_settings(), crate::claude_import::import_marker())
}

/// Every session startup and mid-session reload loads hooks through this function.
/// This is the one place that chooses which hook sources to load.
pub(crate) fn discover_hooks(
    inputs: &ProcessHookInputs,
    git_root: Option<&Path>,
    compat: &codel_tools::types::compat::CompatConfig,
    trust: Trust,
) -> (codel_hooks::discovery::HookRegistry, Vec<HookError>) {
    codel_hooks::discovery::assemble_hooks(
        inputs.config_layers(),
        DiscoveryOptions {
            git_root,
            codel_home: inputs.codel_home(),
            home: inputs.home(),
            compat: compat.hooks(),
            claude_import: inputs.claude_import(),
            trust,
        },
    )
}
