//! Unauthorized (401) recovery state machine.
//!
//! When the server rejects a token, `UnauthorizedRecovery` walks through
//! a sequence of recovery steps before giving up:
//!
//! 1. **ReloadFromDisk** — re-read `auth.json` under a file lock; if the
//!    on-disk token differs from the rejected one, accept it (another
//!    process may have written a new key).
//! 2. **Done** — all recovery strategies exhausted (no refresh authority
//!    exists for API keys).

use std::sync::Arc;

use crate::auth::error::{AuthError, RefreshTokenError, RefreshTokenFailedReason};
use crate::auth::manager::AuthManager;
use crate::auth::model::CodelAuth;

/// Whether the relay should stop reconnecting on this recovery error.
pub(crate) fn relay_should_cancel(err: &AuthError) -> bool {
    matches!(
        err,
        AuthError::Refresh(RefreshTokenError::Permanent(e))
            if matches!(e.reason, RefreshTokenFailedReason::RefreshTokenRejected)
    ) || matches!(
        err,
        AuthError::ServerRejectedNoRecovery
            | AuthError::RecoveryExhausted
            | AuthError::TokenExpiredNoRefresh
            | AuthError::PinnedTeamMismatch { .. }
            | AuthError::ApiKeyAuthDisabled
    )
}

/// Where a 401 recovery was initiated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RecoverySource {
    Turn,
    Relay,
    Background,
}

/// Which recovery step to attempt next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RecoveryStep {
    ReloadFromDisk,
    Done,
}

/// State machine that walks through recovery strategies after a 401.
pub struct UnauthorizedRecovery {
    auth_manager: Arc<AuthManager>,
    rejected_token: String,
    step: RecoveryStep,
}

impl UnauthorizedRecovery {
    pub(crate) fn new(
        auth_manager: Arc<AuthManager>,
        rejected: Option<CodelAuth>,
        _source: RecoverySource,
    ) -> Self {
        let rejected_token = rejected.as_ref().map(|a| a.key.clone()).unwrap_or_default();
        Self {
            auth_manager,
            rejected_token,
            step: RecoveryStep::ReloadFromDisk,
        }
    }

    #[tracing::instrument(
        skip(self),
        fields(step = ?self.step, token_type = tracing::field::Empty),
    )]
    pub async fn next(&mut self) -> Result<CodelAuth, AuthError> {
        let span = tracing::Span::current();
        if !span.is_disabled() {
            span.record(
                "token_type",
                tracing::field::debug(self.auth_manager.token_type()),
            );
        }
        self.resolve_next().await
    }

    async fn resolve_next(&mut self) -> Result<CodelAuth, AuthError> {
        let auth = self.next_step_loop().await?;
        if let Some(e) = self.auth_manager.cached_token_policy_error(&auth) {
            self.auth_manager.reject_and_clear(&e);
            return Err(e);
        }
        Ok(auth)
    }

    async fn next_step_loop(&mut self) -> Result<CodelAuth, AuthError> {
        loop {
            match self.step {
                RecoveryStep::ReloadFromDisk => {
                    self.step = RecoveryStep::Done;
                    if let Some(auth) = self.try_reload_from_disk().await {
                        return Ok(auth);
                    }
                }
                RecoveryStep::Done => {
                    // No refresh authority for API keys.
                    return Err(AuthError::ServerRejectedNoRecovery);
                }
            }
        }
    }

    async fn try_reload_from_disk(&self) -> Option<CodelAuth> {
        let _lock = self
            .auth_manager
            .try_lock_auth_file_async(crate::auth::manager::AUTH_LOCK_TIMEOUT)
            .await;
        if _lock.is_none() {
            tracing::warn!("auth recovery: proceeding without file lock");
        }

        let Some(disk_auth) = self.auth_manager.read_disk_auth() else {
            return None;
        };
        if crate::auth::is_expired(&disk_auth) {
            tracing::debug!("auth recovery: disk token is expired, skipping");
            return None;
        }
        if self.is_different_token(&disk_auth) {
            tracing::info!("auth recovery: disk has a different token, accepting");
            self.auth_manager.hot_swap(disk_auth.clone());
            Some(disk_auth)
        } else {
            tracing::debug!("auth recovery: disk token is same as rejected, skipping");
            None
        }
    }

    fn is_different_token(&self, candidate: &CodelAuth) -> bool {
        candidate.key != self.rejected_token
    }
}
