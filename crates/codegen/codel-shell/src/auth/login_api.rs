//! Stub for the removed interactive auth flow.
//! The global `CODEL_API_KEY` env var has been removed; every model must
//! carry its own `api_key`/`env_key` in the model config. These stubs no
//! longer resolve a process-global key.

use std::sync::Arc;

use crate::auth::error::AuthError;
use crate::auth::manager::AuthManager;
use crate::auth::model::CodelAuth;

/// Transport override for login (stub — only `None` is meaningful).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LoginTransportOverride {
    #[default]
    None,
    ForceLoopback,
}

impl LoginTransportOverride {
    pub fn from_flags(_use_oauth: bool, _force_loopback: bool) -> Self {
        Self::None
    }
}

/// Channels for interactive auth URL/code exchange (stub).
pub struct AuthChannels {
    pub url_tx: Option<tokio::sync::oneshot::Sender<String>>,
    pub code_rx: tokio::sync::mpsc::Receiver<String>,
}

/// Auth URL info (stub).
#[derive(Debug, Clone)]
pub struct AuthUrlInfo {
    pub url: String,
    pub mode: AuthUrlMode,
}

/// Auth URL mode (stub).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthUrlMode {
    Loopback,
    DeviceCode,
}

/// Logout result.
#[derive(Debug, Clone)]
pub struct LogoutResult {
    pub was_logged_in: bool,
    pub email: Option<String>,
    pub api_key_still_set: bool,
}

/// Run the auth flow. Returns a valid token from the manager if available,
/// otherwise errors directing the user to configure per-model credentials.
pub async fn run_auth_flow(
    auth_manager: &Arc<AuthManager>,
    _ctx: &crate::auth::config::CodelComConfig,
    _reauth: bool,
    _url_tx: Option<tokio::sync::oneshot::Sender<String>>,
    _code_rx: Option<tokio::sync::mpsc::Receiver<String>>,
    _force_interactive: Option<bool>,
    _override: LoginTransportOverride,
) -> anyhow::Result<(Arc<CodelAuth>, bool)> {
    // Try to get a valid token from the manager (reads disk).
    match auth_manager.auth().await {
        Ok(auth) => Ok((Arc::new(auth), false)),
        Err(_) => {
            anyhow::bail!(
                "No API key found. Configure api_key or env_key in your model config (~/.codel/config.toml)."
            )
        }
    }
}

/// Run auth flow with stderr bridge (same as run_auth_flow for API-key-only).
pub async fn run_auth_flow_with_stderr_bridge(
    auth_manager: &Arc<AuthManager>,
    ctx: &crate::auth::config::CodelComConfig,
    _channels: AuthChannels,
    reauth: bool,
    force_interactive: Option<bool>,
    login_override: LoginTransportOverride,
) -> anyhow::Result<(Arc<CodelAuth>, bool)> {
    run_auth_flow(
        auth_manager,
        ctx,
        reauth,
        None,
        None,
        force_interactive,
        login_override,
    )
    .await
}

/// Perform logout: clear credentials from memory and disk.
pub fn perform_logout(
    auth_manager: &AuthManager,
    scope: Option<&str>,
) -> Result<LogoutResult, AuthError> {
    let was_logged_in = auth_manager.current_or_expired().is_some();
    let email = auth_manager.current_or_expired().and_then(|a| a.email);

    if let Some(scope) = scope {
        let _ = auth_manager.remove_scope(scope);
    } else {
        let _ = auth_manager.clear();
    }

    // The global `CODEL_API_KEY` env var has been removed; there is no
    // process-global key that could remain set after logout.
    let api_key_still_set = false;

    Ok(LogoutResult {
        was_logged_in,
        email,
        api_key_still_set,
    })
}

/// Ensure authenticated (stub — no global API key; errors directing the user
/// to configure per-model credentials).
pub async fn ensure_authenticated(
    _ctx: &crate::auth::config::CodelComConfig,
    _has_noninteractive_auth: bool,
    _prompt: Option<&str>,
) -> Result<CodelAuth, AuthError> {
    Err(AuthError::NotLoggedIn)
}

/// Ensure authenticated or noninteractive (stub).
/// Returns `Ok(None)` since there is no process-global API key.
pub async fn ensure_authenticated_or_noninteractive(
    _ctx: &crate::auth::config::CodelComConfig,
    _has_noninteractive_auth: bool,
    _prompt: Option<&str>,
) -> Result<Option<CodelAuth>, AuthError> {
    Ok(None)
}

/// Ensure authenticated with override (stub).
pub async fn ensure_authenticated_with_override(
    _ctx: &crate::auth::config::CodelComConfig,
    _override: LoginTransportOverride,
) -> Result<CodelAuth, AuthError> {
    Err(AuthError::NotLoggedIn)
}

/// Try to ensure fresh auth noninteractively. Returns None since there is no
/// process-global API key.
pub async fn try_ensure_fresh_auth(
    _ctx: &crate::auth::config::CodelComConfig,
) -> Option<CodelAuth> {
    None
}

/// Try to ensure session noninteractively (stub — always None for API-key-only).
pub async fn try_ensure_session_noninteractive(
    _auth_manager: &Arc<AuthManager>,
) -> Option<CodelAuth> {
    None
}

/// Run CLI login (stub — returns error since interactive login is removed).
pub async fn run_cli_login(
    _auth_manager: &Arc<AuthManager>,
    _ctx: &crate::auth::config::CodelComConfig,
) -> anyhow::Result<()> {
    anyhow::bail!("Interactive login is not supported. Configure api_key or env_key in your model config (~/.codel/config.toml).")
}

/// Run CLI logout.
pub async fn run_cli_logout(
    auth_manager: &AuthManager,
    scope: Option<&str>,
) -> Result<LogoutResult, AuthError> {
    perform_logout(auth_manager, scope)
}
