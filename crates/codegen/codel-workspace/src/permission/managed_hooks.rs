use std::path::Path;

use codel_permission_rules::managed_policy::ManagedSettings;

/// The disabled-hooks file under `codel_home` plus the `allow_managed_hooks_only` pin of `managed`.
pub fn disabled_hooks_snapshot(
    managed: &ManagedSettings,
    codel_home: Option<&Path>,
) -> codel_hooks::trust::DisabledHooks {
    codel_hooks::trust::DisabledHooks::load(codel_home, managed.non_managed_hooks.is_disabled())
}
