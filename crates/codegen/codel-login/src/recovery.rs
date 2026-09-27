//! Unauthorized (401) recovery state machine.
//!
//! When the server rejects a token, `UnauthorizedRecovery` walks through a sequence of recovery steps before giving up:
//!
//! 1. **ReloadFromDisk**: re-read `auth.json` under a file lock.
//!    If the on-disk token differs from the rejected one, accept it (another process may have refreshed).
//! 2. **RefreshFromAuthority**: run the appropriate refresh chain (OIDC token refresh, external binary, etc.) based on `TokenType`.
//!    Skipped when the live token was minted moments ago (fresh-mint guard).
//! 3. **Done**: all recovery strategies exhausted.
use crate::error::{AuthError, RefreshTokenError, RefreshTokenFailedReason};
use crate::manager::AuthManager;
use crate::model::CodelAuth;
use crate::token_type::TokenType;
use std::sync::Arc;
use codel_logging::events::{AuthTokenKind, ManualAuth, ManualAuthReason, ManualAuthSurface};
/// `manual_auth` KPI reason for a terminal `AuthError`, or `None` when it doesn't force a manual re-login.
/// Lives here (not on `AuthError`) so the error model stays telemetry-free.
pub fn manual_auth_reason(err: &AuthError) -> Option<ManualAuthReason> {
    use ManualAuthReason as R;
    Some(match err {
        AuthError::Refresh(RefreshTokenError::Permanent(e)) => match e.reason {
            RefreshTokenFailedReason::RefreshTokenRejected => R::RefreshTokenRejected,
            RefreshTokenFailedReason::ProviderInteractiveRequired => R::ProviderInteractiveRequired,
            RefreshTokenFailedReason::ClientRejected | RefreshTokenFailedReason::Other => {
                return None;
            }
        },
        AuthError::ServerRejectedNoRecovery => R::NoRefreshAuthority,
        AuthError::RecoveryExhausted => R::RecoveryExhausted,
        AuthError::TokenExpiredNoRefresh => R::TokenExpiredNoRefresh,
        AuthError::PinnedTeamMismatch { .. } => R::WrongTeam,
        AuthError::ApiKeyAuthDisabled
        | AuthError::Refresh(RefreshTokenError::Transient(_))
        | AuthError::NotLoggedIn => {
            return None;
        }
    })
}
/// Whether the relay should stop reconnecting on this recovery error.
/// Exhaustive and independent of `manual_auth_reason`, which buckets errors for the `manual_auth` KPI: a telemetry reclassification must not change how long a relay lives. The two differ in both directions: `ApiKeyAuthDisabled` cancels but is outside the KPI, and a non-sticky permanent verdict (`ProviderInteractiveRequired`) counts toward the KPI but does not cancel. Non-sticky verdicts age out via `PERMANENT_FAILURE_TTL`, and the relay runs on a child cancellation token, so cancelling on one would leave a headless leader alive but unreachable for its whole lifetime.
pub fn relay_should_cancel(err: &AuthError) -> bool {
    match err {
        AuthError::NotLoggedIn | AuthError::Refresh(RefreshTokenError::Transient(_)) => false,
        AuthError::Refresh(RefreshTokenError::Permanent(e)) => e.reason.is_sticky(),
        AuthError::TokenExpiredNoRefresh
        | AuthError::ServerRejectedNoRecovery
        | AuthError::RecoveryExhausted
        | AuthError::PinnedTeamMismatch { .. }
        | AuthError::ApiKeyAuthDisabled => true,
    }
}
/// Fresh-mint guard window (±) for `ServerRejected` refreshes ([`UnauthorizedRecovery::fresh_mint_guard`]).
/// 120s outlasts in-flight requests sent with a previous key plus validation lag (observed stale 401s land ~20s after mint). `current()`'s 300s early-invalidation buffer keeps any guard-returned token wire-valid.
/// A genuinely-dead fresh token waits at most this long to re-mint; the symmetric bound caps that delay when the clock stepped back.
const FRESH_MINT_GUARD_SECS: i64 = 120;
/// Where a 401 recovery was initiated; drives the `manual_auth` KPI.
/// Required at every call site so suppressing the KPI is explicit, not default.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoverySource {
    /// A chat/inference turn; surfaces the `ReAuthRequired` banner.
    Turn,
    /// The relay / leader connection handshake.
    Relay,
    /// Uploads, telemetry, tool calls. Never emits the KPI.
    Background,
}
impl RecoverySource {
    fn trigger(self) -> Option<ManualAuthSurface> {
        match self {
            RecoverySource::Turn => Some(ManualAuthSurface::Turn),
            RecoverySource::Relay => Some(ManualAuthSurface::Relay),
            RecoverySource::Background => None,
        }
    }
}
/// Identity of the rejected credential for `manual_auth`.
/// Captured from the rejected bearer (not live `inner`) so attribution is correct even after a `WrongTeam`/cleared-credential failure.
pub struct RejectedAuth {
    /// `user_id`, when known (empty ids collapse to `None`).
    principal: Option<String>,
    token_kind: AuthTokenKind,
    /// Full rejected bearer; the debounce key.
    /// Uses the whole token (not a suffix) so it matches the credential identity the permanent-failure verdict is scoped to; never logged.
    rejected_token_id: String,
}
impl RejectedAuth {
    pub fn capture(auth: Option<&CodelAuth>) -> Self {
        Self {
            principal: auth.map(|a| a.user_id.clone()).filter(|id| !id.is_empty()),
            token_kind: TokenType::from_auth(auth).telemetry_kind(),
            rejected_token_id: auth.map(|a| a.key.clone()).unwrap_or_default(),
        }
    }
    #[cfg(test)]
    pub fn principal_for_test(&self) -> Option<&str> {
        self.principal.as_deref()
    }
    #[cfg(test)]
    pub fn token_kind_for_test(&self) -> AuthTokenKind {
        self.token_kind
    }
}
/// Trigger and attribution for a user-facing recovery; set at construction iff the source emits the KPI.
struct ManualAuthEmit {
    trigger: ManualAuthSurface,
    snapshot: RejectedAuth,
}
/// Per-process debounce and emit for the `manual_auth` KPI.
/// Held by `AuthManager` so all recoveries on one process share the dedup state.
/// All fields are `Default` under both cfgs, so one derive serves both.
#[derive(Default)]
pub struct ManualAuthTracker {
    /// Id of the rejected credential we last emitted for (single slot: only the most recent).
    /// Repeats on the same bearer debounce; a new credential has a new id and emits again.
    last_token: parking_lot::Mutex<Option<String>>,
    /// Test-only: the last emitted event, so a test can assert what was emitted, not just that something fired.
    #[cfg(test)]
    last_emit: parking_lot::Mutex<Option<ManualAuth>>,
    /// Test-only: count of events that actually fired (post-debounce), so a concurrency test can assert the dedup mutex collapses N races to one.
    #[cfg(test)]
    emit_count: std::sync::atomic::AtomicU32,
}
impl ManualAuthTracker {
    /// Emit a terminal manual-auth event, debounced against the most-recent credential (single slot).
    /// No-op for transient failures (`manual_auth_reason` is `None`) and for API keys (a 401 there means rotate the key, not `/login`).
    pub fn record(&self, snapshot: &RejectedAuth, err: &AuthError, trigger: ManualAuthSurface) {
        if snapshot.token_kind == AuthTokenKind::ApiKey {
            return;
        }
        let Some(reason) = manual_auth_reason(err) else {
            return;
        };
        {
            let mut last = self.last_token.lock();
            if last.as_deref() == Some(snapshot.rejected_token_id.as_str()) {
                return;
            }
            *last = Some(snapshot.rejected_token_id.clone());
        }
        let event = ManualAuth {
            reason,
            trigger,
            token_kind: snapshot.token_kind,
            principal: snapshot.principal.clone(),
        };
        #[cfg(test)]
        {
            *self.last_emit.lock() = Some(event.clone());
            self.emit_count
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
        codel_logging::session_ctx::log_event(event);
    }
    #[cfg(test)]
    pub fn emit_count_for_test(&self) -> u32 {
        self.emit_count.load(std::sync::atomic::Ordering::SeqCst)
    }
    #[cfg(test)]
    pub fn last_token_for_test(&self) -> Option<String> {
        self.last_token.lock().clone()
    }
    #[cfg(test)]
    pub fn last_emit_for_test(&self) -> Option<ManualAuth> {
        self.last_emit.lock().clone()
    }
}
/// Which recovery step to attempt next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RecoveryStep {
    /// Re-read auth.json from disk (file-locked).
    ReloadFromDisk,
    /// Refresh via the authority (OIDC, external binary, etc.).
    RefreshFromAuthority,
    /// All strategies exhausted.
    Done,
}
/// State machine that walks through recovery strategies after a 401.
pub struct UnauthorizedRecovery {
    auth_manager: Arc<AuthManager>,
    /// The token that was rejected by the server.
    rejected_token: String,
    /// Current step in the recovery sequence.
    step: RecoveryStep,
    /// Error from `RefreshFromAuthority`, propagated as fallback when devbox recovery doesn't apply.
    authority_error: Option<AuthError>,
    /// Whether the last authority failure was transient.
    /// Kept after `authority_error` is taken so exhaustion preserves the transient/permanent axis (see the `Done` arm).
    authority_was_transient: bool,
    /// `Some` iff this recovery is user-facing, so a terminal failure emits.
    emit: Option<ManualAuthEmit>,
}
impl UnauthorizedRecovery {
    /// `rejected` is the credential the server rejected: its key drives recovery and (for user-facing sources) its identity is the KPI attribution.
    pub fn new(
        auth_manager: Arc<AuthManager>,
        rejected: Option<CodelAuth>,
        source: RecoverySource,
    ) -> Self {
        let rejected_token = rejected.as_ref().map(|a| a.key.clone()).unwrap_or_default();
        let emit = source.trigger().map(|trigger| ManualAuthEmit {
            trigger,
            snapshot: RejectedAuth::capture(rejected.as_ref()),
        });
        Self {
            auth_manager,
            rejected_token,
            step: RecoveryStep::ReloadFromDisk,
            authority_error: None,
            authority_was_transient: false,
            emit,
        }
    }
    /// Attempt the next recovery step. Walks disk reload, then the token authority, then any last-resort host recovery.
    /// The `token_type` span field is recorded lazily via `Span::is_disabled()` to avoid the lock when tracing is off.
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
        let result = self.resolve_next().await;
        if let (Err(e), Some(emit)) = (&result, &self.emit) {
            self.auth_manager
                .record_manual_auth(&emit.snapshot, e, emit.trigger);
        }
        result
    }
    /// Walk the recovery steps and apply the team-pin policy gate.
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
                    self.step = RecoveryStep::RefreshFromAuthority;
                    if let Some(auth) = self.try_reload_from_disk().await {
                        return Ok(auth);
                    }
                }
                RecoveryStep::RefreshFromAuthority => {
                    {
                        self.step = RecoveryStep::Done;
                    }
                    match self.try_refresh_from_authority().await {
                        Ok(auth) => return Ok(auth),
                        Err(e) => {
                            self.authority_was_transient =
                                matches!(e, AuthError::Refresh(RefreshTokenError::Transient(_)));
                            self.authority_error = Some(e);
                            {
                                return Err(self
                                    .authority_error
                                    .take()
                                    .unwrap_or(AuthError::RecoveryExhausted));
                            }
                        }
                    }
                }
                RecoveryStep::Done => {
                    return Err(if self.authority_was_transient {
                        AuthError::transient("recovery exhausted after transient refresh failure")
                    } else {
                        AuthError::RecoveryExhausted
                    });
                }
            }
        }
    }
    /// Re-read `auth.json` from disk. Accept the token only if it differs from the one that was rejected.
    async fn try_reload_from_disk(&self) -> Option<CodelAuth> {
        let _lock = self
            .auth_manager
            .try_lock_auth_file_async(
                crate::manager::AUTH_LOCK_TIMEOUT,
                crate::manager::lock::Heartbeat::Skip,
            )
            .await
            .into_guard();
        if _lock.is_none() {
            tracing::warn!("auth recovery: proceeding without file lock");
        }
        let Some(disk_auth) = self.auth_manager.read_disk_auth() else {
            codel_logging::unified_log::debug("auth recovery: no disk entry", None, None);
            return None;
        };
        if crate::is_expired(&disk_auth) {
            tracing::debug!("auth recovery: disk token is expired, skipping");
            codel_logging::unified_log::debug(
                "auth recovery: disk token expired",
                None,
                Some(serde_json::json!({
                    "disk_key_prefix": codel_auth::bearer_suffix(&disk_auth.key),
                    "expires_at": disk_auth.expires_at.map(|e| e.to_rfc3339()),
                })),
            );
            return None;
        }
        if self.is_different_token(&disk_auth) {
            tracing::info!("auth recovery: disk has a different token, accepting");
            codel_logging::unified_log::info(
                "auth recovery: adopted disk token",
                None,
                Some(serde_json::json!({
                    "adopted_key_prefix": codel_auth::bearer_suffix(&disk_auth.key),
                    "expires_at": disk_auth.expires_at.map(|e| e.to_rfc3339()),
                })),
            );
            self.auth_manager.hot_swap(disk_auth.clone());
            Some(disk_auth)
        } else {
            tracing::debug!("auth recovery: disk token is same as rejected, skipping");
            codel_logging::unified_log::debug(
                "auth recovery: disk token same as rejected",
                None,
                None,
            );
            None
        }
    }
    /// Return the live token instead of refreshing when its mint age is within ±[`FRESH_MINT_GUARD_SECS`]. Anything outside (including a clock that stepped far back) falls through to a normal refresh.
    /// A 401 moments after a successful mint is a stale rejection or validation lag on the new key. A stale rejection was sent with the previous key and mis-attributed; see `is_stale_snapshot`.
    /// There is no re-mint to fall back on: an API key is configured, not issued.
    fn fresh_mint_guard(&self) -> Option<CodelAuth> {
        let auth = self.auth_manager.current()?;
        let mint_age_seconds = auth.mint_age_seconds();
        if !(-FRESH_MINT_GUARD_SECS..FRESH_MINT_GUARD_SECS).contains(&mint_age_seconds) {
            return None;
        }
        tracing::info!(
            mint_age_seconds,
            "auth recovery: current token freshly minted, skipping refresh"
        );
        codel_logging::unified_log::info(
            "auth recovery: fresh mint, refresh skipped",
            None,
            Some(serde_json::json!({
                "key_prefix": codel_auth::bearer_suffix(&auth.key),
                "mint_age_seconds": mint_age_seconds,
                "guard_seconds": FRESH_MINT_GUARD_SECS,
                "expires_at": auth.expires_at.map(|e| e.to_rfc3339()),
            })),
        );
        Some(auth)
    }
    /// An API key has no renewal authority.
    ///
    /// Upstream dispatched to a per-token-type refresh chain here. A key cannot
    /// be refreshed, and the previous recovery step already re-read disk, so the
    /// server's 401 stands: surface [`AuthError::ServerRejectedNoRecovery`]
    /// rather than `TokenExpiredNoRefresh`, because the trigger is the server
    /// rejecting the credential, not a local TTL (an API key has none).
    async fn try_refresh_from_authority(&self) -> Result<CodelAuth, AuthError> {
        match self.auth_manager.token_type() {
            TokenType::None => Err(AuthError::NotLoggedIn),
            TokenType::ApiKey => {
                codel_logging::unified_log::warn(
                    "auth recovery: an API key has no refresh authority",
                    None,
                    Some(serde_json::json!({ "token_type": "ApiKey" })),
                );
                Err(AuthError::ServerRejectedNoRecovery)
            }
        }
    }
    /// Check if a candidate token is different from the rejected one.
    fn is_different_token(&self, candidate: &CodelAuth) -> bool {
        candidate.key != self.rejected_token
    }
}
#[cfg(test)]
mod tests {
    //! State-machine matrix tests for `UnauthorizedRecovery`. Coverage targets: All 5 `TokenType` variants crossed with dispatch in `try_refresh_from_authority`. `try_reload_from_disk`: same/different/no token on disk.
    //! `next()` exhaustion (`Done` surfaces `RecoveryExhausted`). Fresh-mint guard: ±window bounds, ExternalBinary, verdict grace, policy-hidden fall-through (fail closed).
    //! These tests use the same in-process `AuthManager` that production does. They inject a counting refresher so we can observe whether the authority was consulted.
    use super::*;
    use crate::config::CodelComConfig;
    use crate::error::{RefreshTokenError, RefreshTokenFailedReason};
    use crate::model::{AuthMode, CodelAuth};
    use crate::storage::{read_auth_json, write_auth_json};
    use chrono::{Duration, Utc};
    use std::sync::atomic::{AtomicU32, Ordering};
}
