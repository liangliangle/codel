#![allow(
    unused_imports,
    unused_variables,
    unused_mut,
    unreachable_code,
    dead_code
)]
//! Authentication subsystem for the codel shell crate family.
//!
//! Extracted from `codel-shell::auth`; the shell re-exports this crate as
//! `codel_shell::auth` so existing `crate::*` paths keep resolving.
//!
//! This fork authenticates with an API key and nothing else. Upstream's login
//! flows (browser OIDC, device code, external auth-provider binaries), the
//! session-token refresh machinery and the enterprise auth configuration have
//! been removed; what remains is credential storage, the `AuthManager`, and the
//! request-side credential plumbing.
#![deny(clippy::indexing_slicing)]
pub use codel_logging::unified_log;
pub mod api_key_probe;
pub mod attribution;
pub mod auth_method;
pub mod backend;
pub mod config;
pub mod credential_provider;
pub mod error;
pub mod flow;
pub mod codel_auth_credentials;
pub mod jwt;
pub mod manager;
pub mod meta;
pub mod model;
pub mod recovery;
pub mod side_call_bearer;
pub mod storage;
pub mod token_output;
pub mod token_type;
pub use api_key_probe::{
    DEFAULT_PROBE_TIMEOUT, first_party_env_key_allows_advertise, should_probe_first_party_env_key,
};
pub use config::CodelComConfig;
pub use flow::{
    ensure_authenticated, ensure_authenticated_or_noninteractive, mint_session_noninteractive,
    try_ensure_fresh_auth, try_noninteractive_auth_no_mint,
};
pub use jwt::{is_jwt_expired_or_near, parse_jwt_expiration, parse_jwt_subject};
pub use error::{AuthError, RefreshTokenError, RefreshTokenFailedReason};
pub use manager::AuthManager;
pub use manager::{AuthRemedy, CachedTokenState};
pub use meta::AuthMeta;
pub use model::{AuthMode, CodelAuth, lookup_auth};
pub use model::{TOKEN_TTL, UserInfo, default_coding_data_retention_opt_out, is_expired};
pub use side_call_bearer::{SharedAuthKeyProvider, shared_api_key_provider};
pub use storage::auth_json_path;
pub use storage::{clear_api_key, read_api_key, read_auth_json, store_api_key};
