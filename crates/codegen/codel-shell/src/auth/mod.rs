mod auth_provider;
pub(crate) mod attribution;
mod config;
pub mod credential_provider;
pub mod error;
mod login_api;
mod jwt;
pub(crate) mod manager;
mod meta;
mod model;
pub(crate) mod recovery;
pub(crate) mod single_flight;
mod storage;
mod token_output;
pub(crate) mod token_type;

pub use auth_provider::{AuthProviderConfig, AuthProviderRef};
pub(crate) use auth_provider::{
    PROVIDER_TIMEOUT_CEILING_SECS, PROVIDER_TOKEN_EXPIRY_SKEW_SECS, ProviderRefreshOutcome,
};
#[cfg(test)]
pub(crate) use auth_provider::{test_backdate_provider_mint, test_counting_provider};
pub use config::{
    CodelComConfig, ForceLoginTeam, OAuth2ProviderConfig, OidcAuthConfig, PreferredAuthMethod,
    CODEL_OAUTH2_ISSUER, codel_oauth2_issuer, is_codel_oauth2_issuer,
};
pub(crate) use config::LEGACY_AUTH_SCOPE;
pub(crate) use login_api::{
    AuthChannels, run_auth_flow, run_auth_flow_with_stderr_bridge,
    try_ensure_session_noninteractive,
};
pub use login_api::{
    AuthUrlInfo, AuthUrlMode, LoginTransportOverride, LogoutResult, ensure_authenticated,
    ensure_authenticated_or_noninteractive, ensure_authenticated_with_override, perform_logout,
    run_cli_login, run_cli_logout, try_ensure_fresh_auth,
};
pub use error::{AuthError, RefreshTokenError, RefreshTokenFailedReason};
pub use jwt::{is_jwt_expired_or_near, parse_jwt_expiration};
pub use manager::{AuthManager, shared_api_key_provider};
pub use meta::{AuthMeta, GateInfo};
pub use model::{AuthMode, CodelAuth, lookup_auth};
pub(crate) use model::{
    TOKEN_TTL, UserInfo, default_coding_data_retention_opt_out, is_expired, token_suffix,
};
pub use storage::{
    clear_api_key, read_api_key, read_auth_json, read_token_by_scope, store_api_key,
};

/// Diagnostic uploader callback type (previously in refresh module).
pub type DiagnosticUploader = std::sync::Arc<
    dyn Fn(Vec<u8>, String, String) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>
        + Send
        + Sync,
>;
