//! Hook inputs that do not depend on the session.
//! The caller handles session trust, the workspace, and plugin hooks.

use std::path::Path;
use std::path::PathBuf;

use codel_config::HookConfigLayer;
use codel_hooks::discovery::ClaudeImport;
use codel_hooks::trust::DisabledHooks;

use crate::permission::managed_policy::ManagedSettings;
use crate::permission::managed_policy::disabled_hooks_snapshot;

#[derive(Debug)]
pub struct ProcessHookInputs {
    codel_home: Option<PathBuf>,
    home: Option<PathBuf>,
    claude_import: ClaudeImport,
    config_layers: Vec<HookConfigLayer>,
}

impl ProcessHookInputs {
    /// The shell passes its cached `ClaudeImport`.
    /// The hook service reads a fresh one.
    pub fn read(claude_import: ClaudeImport) -> Self {
        Self {
            codel_home: codel_config::user_codel_home(),
            home: codel_dirs::home_dir(),
            claude_import,
            config_layers: codel_config::hook_config_layers(),
        }
    }

    /// [`Self::read`] plus the disabled-hooks file under the same Codel home.
    /// Returned on its own so a dispatcher consumes it once and a caller that only lists hooks never reads it.
    pub fn read_with_disabled(
        managed: &ManagedSettings,
        claude_import: ClaudeImport,
    ) -> (Self, DisabledHooks) {
        let inputs = Self::read(claude_import);
        let disabled = disabled_hooks_snapshot(managed, inputs.codel_home());
        (inputs, disabled)
    }

    pub fn codel_home(&self) -> Option<&Path> {
        self.codel_home.as_deref()
    }

    pub fn home(&self) -> Option<&Path> {
        self.home.as_deref()
    }

    pub fn claude_import(&self) -> ClaudeImport {
        self.claude_import
    }

    pub fn config_layers(&self) -> &[HookConfigLayer] {
        &self.config_layers
    }
}
