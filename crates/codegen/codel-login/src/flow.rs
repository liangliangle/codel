//! Credential resolution for an API-key-only build.
//!
//! Upstream's `flow` module ran interactive logins — browser OIDC, device code,
//! external auth-provider binaries, devbox migration — and refreshed session
//! tokens. This fork authenticates with an API key only, so all of that is
//! gone. What remains is: read the configured credential, or fail with a
//! message that says how to configure one. Nothing here prompts, opens a
//! browser, spawns a provider, or refreshes anything.

use std::sync::Arc;

use codel_shell_base::util::codel_home;

use crate::{AuthManager, CodelAuth, CodelComConfig};

/// The `AuthManager` a startup call site should read through.
fn build_startup_auth_manager(
    codel_com_config: &CodelComConfig,
    proxy_base_url: String,
) -> Arc<AuthManager> {
    Arc::new(AuthManager::new_with_proxy_base_url(
        &codel_home::codel_home(),
        codel_com_config.clone(),
        proxy_base_url,
    ))
}

/// The configured credential, or `None` when none is available.
///
/// An API key does not expire, so there is nothing to refresh: this is a read
/// of `auth.json` (and the in-memory copy) and nothing more.
pub async fn try_ensure_fresh_auth(
    codel_com_config: &CodelComConfig,
    proxy_base_url: String,
) -> Option<CodelAuth> {
    let auth_manager = build_startup_auth_manager(codel_com_config, proxy_base_url);
    match auth_manager.auth().await {
        Ok(auth) => Some(auth),
        Err(e) => {
            tracing::debug!(error = %e, "try_ensure_fresh_auth: no credentials configured");
            None
        }
    }
}

/// Readiness-path variant of [`try_ensure_fresh_auth`].
///
/// Identical here: upstream deferred a cold session mint past readiness, but an
/// API key is configured rather than minted, so there is nothing to defer.
pub async fn try_noninteractive_auth_no_mint(
    codel_com_config: &CodelComConfig,
    proxy_base_url: String,
) -> Option<CodelAuth> {
    try_ensure_fresh_auth(codel_com_config, proxy_base_url).await
}

/// Nothing to mint.
///
/// Upstream cold-minted a session through a non-interactive provider. An API
/// key is configured by the operator, so there is no provider to run and no
/// credential to persist.
pub async fn mint_session_noninteractive(_auth_manager: &Arc<AuthManager>) -> Option<CodelAuth> {
    None
}

/// The credential this process must use, or an error naming how to configure one.
///
/// `config_device_flow` and `reauth` are accepted for call-site compatibility;
/// neither can do anything without a login flow, and passing them is a
/// programming error the call sites are being migrated away from.
pub async fn ensure_authenticated(
    codel_com_config: &CodelComConfig,
    _config_device_flow: Option<bool>,
    proxy_base_url: String,
    _reauth: bool,
    message_prefix: Option<&str>,
) -> anyhow::Result<CodelAuth> {
    let auth_manager = build_startup_auth_manager(codel_com_config, proxy_base_url);
    match auth_manager.auth().await {
        Ok(auth) => Ok(auth),
        Err(e) => {
            let prefix = message_prefix.unwrap_or("Authentication required.");
            anyhow::bail!(
                "{prefix} Set the API key with the `codel/setApiKey` method, or declare \
                 `api_key` / `env_key` on a `[model.<id>]` entry in config.toml ({e})"
            )
        }
    }
}

/// Non-interactive variant of [`ensure_authenticated`].
///
/// With `has_noninteractive_auth` this returns whatever is configured and never
/// fails — upstream used it to skip the interactive prompt on the wire path.
/// Without it, a configured key is still required.
pub async fn ensure_authenticated_or_noninteractive(
    codel_com_config: &CodelComConfig,
    config_device_flow: Option<bool>,
    proxy_base_url: String,
    has_noninteractive_auth: bool,
    message_prefix: Option<&str>,
) -> anyhow::Result<Option<CodelAuth>> {
    if has_noninteractive_auth {
        Ok(try_ensure_fresh_auth(codel_com_config, proxy_base_url).await)
    } else {
        ensure_authenticated(
            codel_com_config,
            config_device_flow,
            proxy_base_url,
            false,
            message_prefix,
        )
        .await
        .map(Some)
    }
}
