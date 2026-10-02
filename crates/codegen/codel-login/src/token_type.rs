use crate::model::{AuthMode, CodelAuth};

/// What kind of bearer is loaded right now.
///
/// Upstream used this as the dispatch key for `auth()`,
/// `unauthorized_recovery()` and proactive refresh, with variants for OIDC
/// sessions, legacy web-login sessions and external auth binaries. This fork
/// carries API keys only, so there are two states: a key is loaded, or nothing
/// is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenType {
    /// Plain API key (no refresh possible).
    ApiKey,
    /// No credentials loaded.
    None,
}

impl TokenType {
    /// Classify the loaded credential (pure; no manager state).
    pub fn from_auth(auth: Option<&CodelAuth>) -> Self {
        match auth {
            None => Self::None,
            // `AuthMode` has a single variant, so any loaded credential is an API key.
            Some(_) => {
                let _ = AuthMode::ApiKey;
                Self::ApiKey
            }
        }
    }

    /// Always `false`: an API key cannot be refreshed.
    pub fn is_refreshable(self) -> bool {
        false
    }

    /// Converts to the telemetry enum for the `manual_auth` KPI; the mapping is stable.
    pub fn telemetry_kind(self) -> codel_logging::events::AuthTokenKind {
        use codel_logging::events::AuthTokenKind as K;
        match self {
            Self::ApiKey => K::ApiKey,
            Self::None => K::None,
        }
    }
}
