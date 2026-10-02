//! What it takes to get back to a usable credential.
//!
//! Upstream classified "a silent session refresh will fix this" against "the
//! operator's auth provider must mint one" against "the user must log in". With
//! API keys there is nothing to refresh and no provider to run, so a credential
//! the server refuses stays refused until a new key is configured.

use super::AuthManager;

/// The way back to a usable credential, as of right now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthRemedy {
    /// A later attempt can still succeed — a transient network or server-side failure.
    SelfHealing,
    /// The credential is unusable; a new API key must be configured.
    ApiKeyRequired,
}

impl AuthRemedy {
    pub fn is_self_healing(&self) -> bool {
        matches!(self, Self::SelfHealing)
    }

    /// `error_type` for a turn that died on this credential.
    pub fn turn_error_type(&self) -> &'static str {
        match self {
            Self::SelfHealing => "auth_transient",
            Self::ApiKeyRequired => "auth",
        }
    }

    /// The same remedy, for a turn that has already spent its automatic retries.
    /// [`Self::SelfHealing`] cannot survive that: its whole message is "retry in
    /// a few seconds", exactly what just failed several times over.
    pub fn after_retries_exhausted(self) -> Self {
        match self {
            Self::SelfHealing => Self::ApiKeyRequired,
            other => other,
        }
    }

    /// What to tell the user beyond the failure itself.
    pub fn advice(&self) -> Option<String> {
        match self {
            Self::SelfHealing => Some(
                "Authentication is temporarily unavailable (often a network blip right \
                 after wake). Retry in a few seconds."
                    .to_owned(),
            ),
            Self::ApiKeyRequired => None,
        }
    }
}

impl AuthManager {
    /// Classify how the caller can get back to a usable credential.
    pub fn auth_remedy(&self) -> AuthRemedy {
        if self.requires_manual_reauth() {
            AuthRemedy::ApiKeyRequired
        } else {
            AuthRemedy::SelfHealing
        }
    }
}
