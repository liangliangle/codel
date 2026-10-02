//! The ACP auth method this agent advertises.
//!
//! Upstream advertised up to four methods — `codel.api_key`, a cached session
//! token, an enterprise OIDC method and the default `codel.dev` browser login —
//! and a `[auth] preferred_method` pin chose between them. The fork
//! authenticates with an API key only, so the list is at most one entry and
//! there is nothing to pin.

use agent_client_protocol as acp;

use crate::agent::config::ModelEntry;

/// Shared, live handle to the agent's current ACP auth method id. `Arc` so a clone can cross the per-session-thread boundary at spawn.
/// The `ArcSwapOption` interior lets the agent's `authenticate` handler publish a new method without re-spawning sessions. Every running session's per-turn auth gate observes the new method on its next turn.
/// `None` until the first `authenticate`. Auth is process-global (one user, one `AuthManager`), so all sessions sharing one cell is correct.
pub(crate) type SharedAuthMethodId = std::sync::Arc<arc_swap::ArcSwapOption<acp::AuthMethodId>>;

/// Construct a [`SharedAuthMethodId`]. `None` is the pre-`authenticate` state.
pub(crate) fn new_shared_auth_method_id(initial: Option<acp::AuthMethodId>) -> SharedAuthMethodId {
    std::sync::Arc::new(arc_swap::ArcSwapOption::new(
        initial.map(std::sync::Arc::new),
    ))
}

// The first-party env-key primitives live in the low `codel-login` crate
// (auth needs them without pulling in shell's `ModelEntry`); re-exported here so
// `crate::agent::auth_method::{CODEL_API_KEY_ENV_VAR, ..}` call sites keep resolving.
pub use codel_login::auth_method::{
    LEGACY_CODEL_API_KEY_ENV_VAR, CODEL_API_KEY_ENV_VAR, has_codel_api_key_env, read_codel_api_key_env,
};

/// The ACP method id for the API-key method.
pub const CODEL_API_KEY_METHOD_ID: &str = "codel.api_key";

pub const AUTH_ERROR_API_KEY: &str = "Authentication failed. Set CODEL_API_KEY, or add api_key/env_key to a [model.<id>] entry in ~/.codel/config.toml.";

/// Error when the kill switch forbids API-key auth and nothing else can authenticate.
pub const AUTH_ERROR_API_KEY_DISABLED: &str = "API-key authentication is disabled for this deployment and no other method exists.";

/// Whether `codel.api_key` should be advertised (and pushed FIRST) when building the `auth_methods` list at `initialize()` time.
/// Regression: `codel.api_key` must stay first when only per-model credentials exist (no global `CODEL_API_KEY`).
/// Deferring it made BYOK users hit the login screen because the pager uses `auth_methods.first()` for startup metadata. Both inputs can change between calls, so the result is not cached. When true the method is never advertised, regardless of available credentials, so `CODEL_API_KEY` can't bypass a deployment's forced kill switch. Presence-only for the first-party env key (treats it as usable).
pub(crate) fn should_advertise_codel_api_key<'a, I>(disable_api_key_auth: bool, models: I) -> bool
where
    I: IntoIterator<Item = &'a ModelEntry>,
{
    should_advertise_codel_api_key_with_env_ok(disable_api_key_auth, models, true)
}

/// Single advertise policy for `codel.api_key`: the kill switch, BYOK, and the first-party env key.
/// The env key is gated by `first_party_env_ok` (probe result, or `true` for presence-only); BYOK still advertises without a probe.
pub(crate) fn should_advertise_codel_api_key_with_env_ok<'a, I>(
    disable_api_key_auth: bool,
    models: I,
    first_party_env_ok: bool,
) -> bool
where
    I: IntoIterator<Item = &'a ModelEntry>,
{
    if disable_api_key_auth {
        return false;
    }
    let has_byok = models.into_iter().any(ModelEntry::has_own_credentials);
    has_byok || (has_codel_api_key_env() && first_party_env_ok)
}

/// Output of [`build_auth_methods`].
pub struct BuiltAuthMethods {
    /// Auth methods in advertised order. At most one entry in this fork.
    pub methods: Vec<acp::AuthMethod>,
    /// The `auth_method_id` to install on the agent; `None` when the method is unavailable.
    pub default_auth_method_id: Option<acp::AuthMethodId>,
}

/// Build the advertised auth-method list.
///
/// Exactly one method exists, so the list is either `[codel.api_key]` or empty
/// when the caller could not establish that a key is available.
pub fn build_auth_methods(has_external_api_key: bool) -> BuiltAuthMethods {
    if !has_external_api_key {
        codel_logging::unified_log::warn(
            "auth: no API-key credentials available; advertising no auth method",
            None,
            None,
        );
        return BuiltAuthMethods {
            methods: Vec::new(),
            default_auth_method_id: None,
        };
    }
    BuiltAuthMethods {
        methods: vec![codel_api_key_auth_method()],
        default_auth_method_id: Some(acp::AuthMethodId::new(CODEL_API_KEY_METHOD_ID)),
    }
}

fn codel_api_key_auth_method() -> acp::AuthMethod {
    acp::AuthMethod::Agent(
        acp::AuthMethodAgent::new(
            acp::AuthMethodId::new(CODEL_API_KEY_METHOD_ID),
            "codel.api_key".to_string(),
        )
        .description(Some(format!(
            "{CODEL_API_KEY_ENV_VAR} or api_key/env_key in config.toml"
        ))),
    )
}

/// ACP auth-method classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthMethodKind {
    CodelApiKey,
    Unknown,
}

impl AuthMethodKind {
    pub fn from_id(id: &acp::AuthMethodId) -> Self {
        match id.0.as_ref() {
            CODEL_API_KEY_METHOD_ID => Self::CodelApiKey,
            _ => Self::Unknown,
        }
    }

    pub fn is_api_key(self) -> bool {
        matches!(self, Self::CodelApiKey)
    }

    /// Always `false`: the fork has no session-based auth method.
    pub fn is_session_based(self) -> bool {
        false
    }
}

/// Whether a session bearer should be attached for a request.
///
/// Upstream gated this on a session-based auth method, a non-BYOK model, and a
/// first-party endpoint. No advertised method carries a session credential, so
/// the gate is always closed.
pub(crate) fn session_token_auth_gate(
    _is_session_based: bool,
    _model_byok: ModelByok,
    _endpoint_is_first_party: bool,
) -> bool {
    false
}

/// Always `false`: no advertised method carries a session credential.
pub(crate) fn is_session_based_method(_method_id: &acp::AuthMethodId) -> bool {
    false
}

/// Per-model BYOK status: whether the selected model carries its own `[model.*]` `api_key`/`env_key`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, strum::AsRefStr, strum::IntoStaticStr)]
#[strum(serialize_all = "snake_case")]
pub(crate) enum ModelByok {
    /// Model has its own per-model key (not refreshable).
    Byok,
    /// Model has no per-model key (session auth governs).
    NotByok,
    /// Config couldn't be loaded/parsed; BYOK status indeterminate.
    Unknown,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn advertises_the_api_key_method_when_available() {
        let built = build_auth_methods(true);
        assert_eq!(built.methods.len(), 1);
        assert_eq!(
            built.default_auth_method_id.as_ref().map(|id| id.0.to_string()),
            Some(CODEL_API_KEY_METHOD_ID.to_owned()),
        );
        assert!(AuthMethodKind::from_id(&acp::AuthMethodId::new(CODEL_API_KEY_METHOD_ID)).is_api_key());
    }

    #[test]
    fn advertises_nothing_without_a_key() {
        let built = build_auth_methods(false);
        assert!(built.methods.is_empty());
        assert!(built.default_auth_method_id.is_none());
    }

    #[test]
    fn no_method_is_session_based() {
        assert!(!is_session_based_method(&acp::AuthMethodId::new(CODEL_API_KEY_METHOD_ID)));
        assert!(!AuthMethodKind::from_id(&acp::AuthMethodId::new("cached_token")).is_session_based());
    }

    #[test]
    fn unknown_ids_classify_as_unknown() {
        assert_eq!(
            AuthMethodKind::from_id(&acp::AuthMethodId::new("oidc")),
            AuthMethodKind::Unknown
        );
    }
}
