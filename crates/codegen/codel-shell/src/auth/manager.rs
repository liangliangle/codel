//! `AuthManager` -- single source of truth for `auth.json` + the
//! in-memory bearer cache. API-key-only: no refresh chains, no OIDC,
//! no external providers.

use chrono::Utc;
use parking_lot::RwLock;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration as StdDuration;

use tokio_util::sync::CancellationToken;

#[path = "manager/enrichment.rs"]
mod enrichment;
#[path = "manager/lock.rs"]
mod lock;
#[path = "manager/sleep_gate.rs"]
mod sleep_gate;

use crate::auth::config::CodelComConfig;
use crate::auth::error::AuthError;
use crate::auth::token_type::TokenType;

#[cfg(test)]
use super::model::UserInfo;
use super::model::{
    AuthMode, CodelAuth, early_invalidation, is_expired, is_expired_with_buffer, lookup_auth,
    token_suffix,
};
use super::storage::{
    AuthFileLock, read_auth_json, read_auth_json_or_empty_recovering_corrupt, write_auth_json,
};

#[cfg(test)]
use super::storage::read_auth_json_or_empty;
#[cfg(test)]
use chrono::DateTime;
#[cfg(test)]
use enrichment::apply_user_info_enrichment;

#[cfg(test)]
use super::model::AuthStore;
use super::model::LEGACY_SCOPE;

/// Why a token refresh is being requested.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RefreshReason {
    /// Pre-request check. Return cached token if still valid.
    PreRequest,
    /// Server returned 401/403. Must obtain a different token.
    ServerRejected,
}

/// Timeout for acquiring the advisory `auth.json.lock` file lock.
pub(crate) const AUTH_LOCK_TIMEOUT: StdDuration = StdDuration::from_secs(10);

/// Long poll interval used by the proactive refresh task when no
/// productive refresh is possible.
pub(crate) const BACKOFF_INTERVAL: StdDuration = StdDuration::from_secs(300);

/// `force_reload_from_disk` re-read budget.
const RELOAD_RETRY_TRIES: usize = 3;

/// Backoff between `force_reload_from_disk` re-reads.
const RELOAD_RETRY_BACKOFF: StdDuration = StdDuration::from_millis(50);

/// Discriminated outcome of a disk read, for transition logging.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DiskAuthState {
    /// auth.json readable and the scope entry exists.
    Ok,
    /// auth.json does not exist.
    FileMissing,
    /// auth.json readable but has no usable entry for this scope.
    EntryMissing,
    /// auth.json exists but could not be read.
    Unreadable,
}

/// On-disk outcome of [`AuthManager::remove_scope_impl`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScopeRemoval {
    EntryRemoved,
    FileDeleted,
    SkippedLockUnavailable,
    SkippedUnreadable,
}

impl ScopeRemoval {
    fn label(self) -> &'static str {
        match self {
            Self::EntryRemoved => "entry removed",
            Self::FileDeleted => "file deleted (no scopes left)",
            Self::SkippedLockUnavailable => "skipped (lock unavailable)",
            Self::SkippedUnreadable => "skipped (auth.json unreadable)",
        }
    }
}

// ── Construction + builders ──────────────────────────────────────────

/// Single source of truth for `auth.json` + the in-memory bearer.
pub struct AuthManager {
    inner: Arc<RwLock<Option<CodelAuth>>>,
    path: PathBuf,
    scope: String,
    codel_com_config: CodelComConfig,
    proxy_base_url: String,
    /// Notified after every successful token change.
    refresh_notify: Arc<tokio::sync::Notify>,
    /// Last state `read_disk_auth` observed for this manager's scope.
    disk_state: RwLock<Option<DiskAuthState>>,
    /// See [`Self::cached_disk_api_key`].
    static_key_cache: parking_lot::Mutex<Option<StaticKeyCacheEntry>>,
    /// Model `api_key` / resolved `env_key` for voice/tools without a session.
    process_static_api_key: parking_lot::RwLock<Option<String>>,
    sleep_gate: sleep_gate::SleepGate,
}

impl std::fmt::Debug for AuthManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuthManager")
            .field("scope", &self.scope)
            .finish_non_exhaustive()
    }
}

impl AuthManager {
    pub fn new(codel_home: &Path, codel_com_config: CodelComConfig) -> Self {
        let scope = codel_com_config.auth_scope();
        let proxy_base_url =
            crate::agent::config::EndpointsConfig::from_effective_config().proxy_url();


        // CODEL_AUTH: inline JSON credentials (highest priority, read-only).
        if let Ok(inline_json) = std::env::var("CODEL_AUTH") {
            if let Ok(auth) = serde_json::from_str::<CodelAuth>(&inline_json) {
                return Self::assemble(
                    Some(auth),
                    codel_home.join("auth.json"),
                    scope,
                    codel_com_config,
                    proxy_base_url,
                    None,
                );
            }
            tracing::warn!("CODEL_AUTH set but failed to parse as JSON, falling back to file");
        }

        // CODEL_AUTH_PATH: custom file path (overrides default $CODEL_HOME/auth.json).
        let path = std::env::var("CODEL_AUTH_PATH")
            .map(PathBuf::from)
            .unwrap_or_else(|_| codel_home.join("auth.json"));

        let (auth, auth_read_detail, initial_disk_state) = match read_auth_json(&path) {
            Ok(map) => {
                let found = lookup_auth(&map, &scope);
                let detail = serde_json::json!({
                    "read": "ok",
                    "resolved_path": path.display().to_string(),
                    "scopes_on_disk": map.keys().collect::<Vec<_>>(),
                    "target_scope": &scope,
                    "found": found.is_some(),
                    "auth_mode": found.as_ref().map(|a| format!("{:?}", a.auth_mode)),
                    "is_expired": found.as_ref().map(is_expired),
                    "key_prefix": found.as_ref().map(|a| token_suffix(&a.key).to_owned()),
                });
                let state = if found.is_some() {
                    DiskAuthState::Ok
                } else {
                    DiskAuthState::EntryMissing
                };
                (found, detail, state)
            }
            Err(e) => {
                let detail = serde_json::json!({
                    "read": "error",
                    "error": e.to_string(),
                    "path": path.display().to_string(),
                    "path_exists": path.exists(),
                });
                let state = if e.kind() == std::io::ErrorKind::NotFound {
                    DiskAuthState::FileMissing
                } else {
                    DiskAuthState::Unreadable
                };
                (None, detail, state)
            }
        };

        let manager = Self::assemble(
            auth,
            path,
            scope,
            codel_com_config,
            proxy_base_url,
            Some(initial_disk_state),
        );
        manager.enforce_pin_on_loaded_token();
        manager
    }

    fn assemble(
        inner: Option<CodelAuth>,
        path: PathBuf,
        scope: String,
        codel_com_config: CodelComConfig,
        proxy_base_url: String,
        disk_state: Option<DiskAuthState>,
    ) -> Self {
        Self {
            inner: Arc::new(RwLock::new(inner)),
            path,
            scope,
            codel_com_config,
            proxy_base_url,
            refresh_notify: Arc::new(tokio::sync::Notify::new()),
            disk_state: RwLock::new(disk_state),
            static_key_cache: parking_lot::Mutex::new(None),
            process_static_api_key: parking_lot::RwLock::new(None),
            sleep_gate: sleep_gate::SleepGate::default(),
        }
    }

    /// Clear the disk-loaded token if it violates the team pin.
    fn enforce_pin_on_loaded_token(&self) {
        let loaded = self.inner.read().clone();
        if let Some(auth) = loaded
            && let Some(e) = self.cached_token_policy_error(&auth)
        {
            self.reject_and_clear(&e);
        }
    }

    /// Override the proxy base URL.
    pub(crate) fn with_proxy_base_url(mut self, url: &str) -> Self {
        self.proxy_base_url = url.to_owned();
        self
    }

    // ── State mutation (clear, hot_swap, update) ──────────────────────

    pub(crate) fn clear(&self) -> std::io::Result<()> {
        self.remove_scope(&self.scope)
    }

    pub(crate) fn remove_scope(&self, scope: &str) -> std::io::Result<()> {
        self.remove_scope_impl(scope)
    }

    fn remove_scope_impl(&self, scope: &str) -> std::io::Result<()> {
        let disk_mutation = if let Some(_lock) = lock::try_lock_auth_file_nonblocking(&self.path) {
            self.write_scope_removal(scope)?
        } else {
            ScopeRemoval::SkippedLockUnavailable
        };
        if scope == self.scope {
            self.clear_inner();
        }
        Ok(())
    }

    fn write_scope_removal(&self, scope: &str) -> std::io::Result<ScopeRemoval> {
        let Ok(mut auth_store) = read_auth_json(&self.path) else {
            return Ok(ScopeRemoval::SkippedUnreadable);
        };
        auth_store.remove(scope);
        if auth_store.is_empty() {
            let _ = std::fs::remove_file(&self.path);
            Ok(ScopeRemoval::FileDeleted)
        } else {
            write_auth_json(&self.path, &auth_store)?;
            Ok(ScopeRemoval::EntryRemoved)
        }
    }

    fn clear_inner(&self) {
        *self.inner.write() = None;
    }

    /// Re-read `auth.json` and reconcile the in-memory cache with it.
    pub(crate) fn force_reload_from_disk(&self) {
        self.force_reload_from_disk_with(RELOAD_RETRY_TRIES, RELOAD_RETRY_BACKOFF);
    }

    fn force_reload_from_disk_with(&self, tries: usize, backoff: StdDuration) {
        for attempt in 0..tries.max(1) {
            if attempt > 0 && !backoff.is_zero() {
                std::thread::sleep(backoff);
            }
            let (auth, state) = self.read_disk_auth_with_state();
            match state {
                DiskAuthState::Ok => {
                    *self.inner.write() = auth;
                    self.enforce_pin_on_loaded_token();
                    return;
                }
                DiskAuthState::EntryMissing => {
                    self.drop_in_memory_credentials("scope absent on readable auth.json");
                    self.enforce_pin_on_loaded_token();
                    return;
                }
                DiskAuthState::FileMissing | DiskAuthState::Unreadable => {}
            }
        }

        // Persistent disk anomaly: drop credentials.
        self.drop_in_memory_credentials(
            "disk anomaly; no live token to retain",
        );
        self.enforce_pin_on_loaded_token();
    }

    fn drop_in_memory_credentials(&self, reason: &str) {
        if let Some(d) = self.current_or_expired() {
        }
        self.clear_inner();
    }

    // ── Read methods ─────────────────────────────────────────────────

    /// `Some(error)` when a `force_login_team_uuid` pin is set and the token's
    /// team principal isn't allowed; `None` when compliant or unpinned.
    pub(crate) fn cached_token_policy_error(&self, auth: &CodelAuth) -> Option<AuthError> {
        if auth.auth_mode == AuthMode::ApiKey {
            return self
                .codel_com_config
                .api_key_auth_disabled()
                .then_some(AuthError::ApiKeyAuthDisabled);
        }
        None
    }

    /// Log and clear a policy-violating session.
    pub(crate) fn reject_and_clear(&self, error: &AuthError) {
        let policy = match error {
            AuthError::PinnedTeamMismatch { .. } => "team_pin",
            AuthError::ApiKeyAuthDisabled => "api_key_disabled",
            _ => "login_policy",
        };
        if let Err(e) = self.clear() {
            tracing::warn!(error = %e, "auth: failed to clear policy-violating session");
        }
    }

    fn vet_cached(&self, auth: CodelAuth) -> Option<CodelAuth> {
        match self.cached_token_policy_error(&auth) {
            None => Some(auth),
            Some(e) => {
                tracing::debug!(error = %e, "auth: hiding cached session rejected by login policy");
                None
            }
        }
    }

    /// Cached in-memory token if outside the early-invalidation buffer.
    pub(crate) fn current(&self) -> Option<CodelAuth> {
        let auth = self
            .inner
            .read()
            .as_ref()
            .filter(|a| !self.is_token_expired(a))
            .cloned()?;
        self.vet_cached(auth)
    }

    #[inline]
    pub(crate) fn with_inner_write<R>(&self, f: impl FnOnce(&mut Option<CodelAuth>) -> R) -> R {
        let mut guard = self.inner.write();
        f(&mut guard)
    }

    #[inline]
    pub(crate) fn with_inner_read<R>(&self, f: impl FnOnce(Option<&CodelAuth>) -> R) -> R {
        let guard = self.inner.read();
        f(guard.as_ref())
    }

    /// Returns true if credentials exist but have expired.
    pub(crate) fn is_expired(&self) -> bool {
        self.inner
            .read()
            .as_ref()
            .is_some_and(|a| self.is_token_expired(a))
    }

    /// In-memory bearer regardless of the early-invalidation buffer.
    pub(crate) fn current_or_expired(&self) -> Option<CodelAuth> {
        self.current().or_else(|| self.expired_auth())
    }

    /// Cached token if still wire-valid.
    pub(crate) fn current_wire_valid(&self) -> Option<CodelAuth> {
        let auth = self
            .inner
            .read()
            .as_ref()
            .filter(|a| !self.is_token_hard_expired(a))
            .cloned()?;
        self.vet_cached(auth)
    }

    pub(crate) fn is_data_collection_disabled(&self) -> bool {
        self.current_or_expired()
            .is_some_and(|a| a.is_data_collection_disabled())
    }

    pub(crate) fn allows_data_collection(&self) -> bool {
        self.current_or_expired()
            .is_some_and(|a| !a.is_data_collection_disabled())
    }

    /// Expired in-memory entry.
    pub(crate) fn expired_auth(&self) -> Option<CodelAuth> {
        let auth = self
            .inner
            .read()
            .as_ref()
            .filter(|a| self.is_token_expired(a))
            .cloned()?;
        self.vet_cached(auth)
    }

    fn is_token_expired(&self, auth: &CodelAuth) -> bool {
        self.token_expired_with_buffer(auth, early_invalidation())
    }

    fn is_token_hard_expired(&self, auth: &CodelAuth) -> bool {
        self.token_expired_with_buffer(auth, chrono::Duration::zero())
    }

    fn token_expired_with_buffer(&self, auth: &CodelAuth, buffer: chrono::Duration) -> bool {
        is_expired_with_buffer(auth, buffer)
    }

    // ── Persistence + enrichment ──────────────────────────────────────

    /// Persist rotated tokens to disk + cache, then spawn `/user` enrichment.
    pub(crate) async fn update(self: &Arc<Self>, auth: CodelAuth) -> std::io::Result<CodelAuth> {
        let map = match read_auth_json_or_empty_recovering_corrupt(&self.path) {
            Ok(map) => map,
            Err(e) => {
                tracing::warn!(error = %e, "auth: read failed, updating in-memory only");
                self.with_inner_write(|inner| *inner = Some(auth.clone()));
                self.spawn_user_info_enrichment(auth.clone());
                return Ok(auth);
            }
        };
        let mut map = map;
        tracing::debug!(scope = %self.scope, "auth: storing token");
        map.insert(self.scope.clone(), auth.clone());
        let write_result = write_auth_json(&self.path, &map);
        self.with_inner_write(|inner| *inner = Some(auth.clone()));
        self.spawn_user_info_enrichment(auth.clone());
        write_result?;
        Ok(auth)
    }

    /// Persist to disk and cache without spawning the background `/user` task.
    pub(crate) async fn save_without_enrichment(
        &self,
        auth: CodelAuth,
    ) -> std::io::Result<CodelAuth> {
        let map = match read_auth_json_or_empty_recovering_corrupt(&self.path) {
            Ok(map) => map,
            Err(e) => {
                tracing::warn!(error = %e, "auth: read failed, updating in-memory only (no enrichment)");
                self.with_inner_write(|inner| *inner = Some(auth.clone()));
                return Ok(auth);
            }
        };
        let mut map = map;
        tracing::debug!(scope = %self.scope, "auth: storing token (no enrichment)");
        map.insert(self.scope.clone(), auth.clone());
        let write_result = write_auth_json(&self.path, &map);
        self.with_inner_write(|inner| *inner = Some(auth.clone()));
        write_result?;
        Ok(auth)
    }

    fn spawn_user_info_enrichment(self: &Arc<Self>, auth: CodelAuth) {
        enrichment::spawn(Arc::clone(self), auth);
    }

    /// Blocking `/user` enrichment for login flows.
    pub(crate) async fn enrich_auth_inline(&self, auth: &mut CodelAuth) {
        enrichment::enrich_inline(self, auth).await;
    }

    pub(crate) fn codel_com_config(&self) -> &CodelComConfig {
        &self.codel_com_config
    }

    pub fn refresh_notifier(&self) -> Arc<tokio::sync::Notify> {
        self.refresh_notify.clone()
    }

    /// Wait up to `timeout` for a token change.
    pub async fn wait_for_token_refresh(&self, timeout: std::time::Duration) -> bool {
        let pre_key = self.current().map(|a| a.key.clone());
        tokio::select! {
            _ = self.refresh_notify.notified() => {}
            _ = tokio::time::sleep(timeout) => {}
        }
        let post_key = self.current().map(|a| a.key.clone());
        post_key != pre_key
    }

    /// Hot-swap credentials (called by config watcher). Does NOT write to disk.
    pub(crate) fn hot_swap(&self, new_auth: CodelAuth) {
        self.with_inner_write(|inner| *inner = Some(new_auth));
    }

    /// Clear in-memory credentials. Does NOT touch disk.
    pub(crate) fn clear_in_memory(&self) {
        self.clear_inner();
    }

    // ── Disk I/O helpers ──────────────────────────────────────────────

    /// Accept a sibling-written disk token.
    pub(crate) fn try_use_disk_token(
        &self,
        disk_auth: Option<&CodelAuth>,
        reason: RefreshReason,
    ) -> Option<CodelAuth> {
        let disk_auth = disk_auth?;
        if self.is_token_expired(disk_auth) {
            return None;
        }
        if reason == RefreshReason::ServerRejected {
            let current_key = self.inner.read().as_ref().map(|a| a.key.clone());
            if current_key.as_deref() == Some(&disk_auth.key) {
                tracing::info!("auth: disk token same as rejected token, skipping");
                return None;
            }
        }
        tracing::info!("auth: another process already refreshed, using disk token");
        self.hot_swap(disk_auth.clone());
        Some(disk_auth.clone())
    }

    /// Re-read disk and try to adopt a sibling-written token.
    fn try_adopt_disk_token(&self, reason: RefreshReason, msg: &str) -> Option<CodelAuth> {
        let disk_auth = self.read_disk_auth();
        let refreshed = self.try_use_disk_token(disk_auth.as_ref(), reason)?;
        let adopted = token_suffix(&refreshed.key);
        let prev = self.expired_auth().map(|a| token_suffix(&a.key).to_owned());
        Some(refreshed)
    }

    /// Test-only hot_swap + disk write (skips proxy `/user`).
    #[cfg(test)]
    fn persist_and_swap(&self, auth: CodelAuth) -> Option<CodelAuth> {
        self.hot_swap(auth.clone());
        let mut map = match read_auth_json_or_empty(&self.path) {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!(error = %e, "auth: read failed in persist_and_swap, skipping disk write");
                return Some(auth);
            }
        };
        map.insert(self.scope.clone(), auth.clone());
        if let Err(e) = write_auth_json(&self.path, &map) {
            tracing::warn!(error = %e, "auth: failed to persist refreshed token to disk");
        }
        Some(auth)
    }

    /// Re-read `auth.json` from disk without updating in-memory state.
    pub(crate) fn read_disk_auth(&self) -> Option<CodelAuth> {
        self.read_disk_auth_with_state().0
    }

    fn read_disk_auth_silent(&self) -> Option<CodelAuth> {
        read_auth_json(&self.path)
            .ok()
            .and_then(|map| lookup_auth(&map, &self.scope))
    }

    /// Wire-valid token present in on-disk `auth.json`.
    pub(crate) fn has_usable_disk_token(&self) -> bool {
        self.read_disk_auth()
            .is_some_and(|a| !self.is_token_hard_expired(&a))
    }

    /// Whether a wire-valid token is available in memory or on disk.
    pub(crate) fn has_usable_token(&self) -> bool {
        self.current_or_expired()
            .is_some_and(|a| !self.is_token_hard_expired(&a))
            || self.has_usable_disk_token()
    }

    pub(crate) fn read_disk_auth_with_state(&self) -> (Option<CodelAuth>, DiskAuthState) {
        let (auth, state, err_detail) = match read_auth_json(&self.path) {
            Ok(map) => {
                let found = lookup_auth(&map, &self.scope);
                let state = if found.is_some() {
                    DiskAuthState::Ok
                } else {
                    DiskAuthState::EntryMissing
                };
                (found, state, None)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                (None, DiskAuthState::FileMissing, None)
            }
            Err(e) => {
                tracing::warn!(
                    path = %self.path.display(),
                    error = %e,
                    "auth: failed to read auth.json"
                );
                (None, DiskAuthState::Unreadable, Some(e.to_string()))
            }
        };
        self.observe_disk_state(state, auth.as_ref(), err_detail);
        (auth, state)
    }

    fn observe_disk_state(
        &self,
        new_state: DiskAuthState,
        auth: Option<&CodelAuth>,
        err_detail: Option<String>,
    ) {
        let prev = {
            let mut guard = self.disk_state.write();
            let prev = *guard;
            *guard = Some(new_state);
            prev
        };
        if prev == Some(new_state) {
            return;
        }
        let ctx = serde_json::json!({
            "from": prev.map(|s| format!("{s:?}")),
            "to": format!("{new_state:?}"),
            "path": self.path.display().to_string(),
            "scope": &self.scope,
            "error": err_detail,
            "key_prefix": auth.map(|a| token_suffix(&a.key).to_owned()),
            "is_expired": auth.map(is_expired),
        });
        match new_state {
            DiskAuthState::Ok => {
            }
            DiskAuthState::FileMissing
            | DiskAuthState::EntryMissing
            | DiskAuthState::Unreadable => {
            }
        }
    }

    pub(crate) async fn try_lock_auth_file_async(
        &self,
        timeout: StdDuration,
    ) -> Option<AuthFileLock> {
        lock::try_lock_auth_file_async(&self.path, timeout).await
    }

    // ── Pre-request dispatch ──────────────────────────────────────────

    /// Pre-request entry point.
    #[tracing::instrument(skip(self), fields(token_type = tracing::field::Empty))]
    pub async fn auth(self: &Arc<Self>) -> Result<CodelAuth, AuthError> {
        let auth = self.auth_dispatch().await?;
        if let Some(e) = self.cached_token_policy_error(&auth) {
            self.reject_and_clear(&e);
            return Err(e);
        }
        Ok(auth)
    }

    async fn auth_dispatch(self: &Arc<Self>) -> Result<CodelAuth, AuthError> {
        let snapshot: Option<CodelAuth> = self.with_inner_read(|inner| inner.cloned());
        let token_type = TokenType::from_auth(snapshot.as_ref());
        tracing::Span::current().record("token_type", tracing::field::debug(token_type));

        // Fast path: valid cached token.
        if let Some(ref auth) = snapshot
            && !self.is_token_expired(auth)
        {
            return Ok(auth.clone());
        }

        match token_type {
            TokenType::None => Err(AuthError::NotLoggedIn),
            TokenType::ApiKey => {
                if snapshot.is_some() {
                    Err(AuthError::TokenExpiredNoRefresh)
                } else {
                    Err(AuthError::NotLoggedIn)
                }
            }
        }
    }

    /// Return the current valid token string, or an error.
    pub(crate) async fn get_valid_token(self: &Arc<Self>) -> Result<String, AuthError> {
        self.auth().await.map(|a| a.key)
    }

    // ── 401 recovery entry point ──────────────────────────────────────

    pub(crate) fn unauthorized_recovery(
        self: &Arc<Self>,
        rejected: Option<CodelAuth>,
        source: crate::auth::recovery::RecoverySource,
    ) -> crate::auth::recovery::UnauthorizedRecovery {
        crate::auth::recovery::UnauthorizedRecovery::new(self.clone(), rejected, source)
    }

    /// One-shot 401 recovery off the live bearer.
    pub(crate) async fn try_recover_unauthorized(
        self: &Arc<Self>,
        source: crate::auth::recovery::RecoverySource,
    ) -> bool {
        let cached = self.with_inner_read(|inner| inner.cloned());
        self.unauthorized_recovery(cached, source)
            .next()
            .await
            .is_ok()
    }

    // Manual auth KPI tracking removed.

    // ── Proactive refresh (no-op for API-key-only) ────────────────────

    /// Spawn a background task. No-op for API-key-only auth (nothing to refresh).
    pub(crate) fn start_proactive_refresh(self: &Arc<Self>, _cancel: CancellationToken) {
        // No refreshable token types exist; nothing to do.
    }

    /// Re-read auth.json from disk and update the in-memory cache.
    pub(crate) fn pick_up_sibling_token(&self) {
        let auth = match read_auth_json(&self.path) {
            Ok(map) => lookup_auth(&map, &self.scope),
            _ => None,
        };
        if let Some(ref a) = auth
            && !self.is_token_expired(a)
            && self.is_different_token(a)
        {
            tracing::info!("auth: picked up sibling-written token from disk");
            self.with_inner_write(|inner| *inner = Some(a.clone()));
        }
    }

    /// Check if a candidate auth has a different token than what's in memory.
    pub(crate) fn is_different_token(&self, candidate: &CodelAuth) -> bool {
        let current_key = self.inner.read().as_ref().map(|a| a.key.clone());
        current_key.as_deref() != Some(&candidate.key)
    }

    /// `pub(super)` — for token type dispatch.
    pub(super) fn token_type(&self) -> TokenType {
        TokenType::from_auth(self.inner.read().as_ref())
    }

    /// Whether we're running inside a devbox environment.
    pub(crate) fn is_devbox_environment(&self) -> bool {
        false
    }

    /// Last-resort devbox auth recovery — always fails (no OIDC).
    pub(crate) async fn try_devbox_recovery(self: &Arc<Self>) -> Result<CodelAuth, AuthError> {
        Err(AuthError::NotLoggedIn)
    }

    pub fn start_system_power_listener(self: &Arc<Self>) {}

    pub fn is_sleep_gated(&self) -> bool {
        false
    }

    pub fn is_dark_wake(&self) -> bool {
        false
    }

    pub fn should_defer_for_dark_wake(&self) -> bool {
        false
    }

    pub(crate) fn has_permanent_failure(&self) -> bool {
        false
    }

    pub(crate) fn has_refresher_attached(&self) -> bool {
        false
    }
}

/// Compute the sleep duration for the next iteration of the proactive
/// refresh loop. Always returns BACKOFF_INTERVAL (nothing to refresh).
pub(crate) fn compute_proactive_sleep(_this: &AuthManager) -> StdDuration {
    BACKOFF_INTERVAL
}

/// Tools + pager voice bearer. Static: env → process model key → disk.
pub(crate) struct SharedAuthKeyProvider(pub Arc<AuthManager>);

impl codel_tools::types::ApiKeyProvider for SharedAuthKeyProvider {
    fn current_api_key(&self) -> Option<String> {
        resolve_static_api_key(&self.0)
            .or_else(|| self.0.current_wire_valid().map(|a| a.key))
            .or_else(|| self.0.current_or_expired().map(|a| a.key))
    }

    fn current_api_key_async(
        &self,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Option<String>> + Send + '_>> {
        let am = self.0.clone();
        Box::pin(async move {
            am.get_valid_token()
                .await
                .ok()
                .or_else(|| resolve_static_api_key(&am))
        })
    }
}

/// Process model key → disk.
fn resolve_static_api_key(am: &AuthManager) -> Option<String> {
    if am.codel_com_config.api_key_auth_disabled() {
        return None;
    }
    non_empty_key(am.process_static_api_key.read().clone())
        .or_else(|| am.cached_disk_api_key())
}

fn api_key_from_auth_file(path: &Path) -> Option<String> {
    let map = read_auth_json(path).ok()?;
    non_empty_key(map.get(super::model::API_KEY_SCOPE).map(|a| a.key.clone()))
}

/// Memo for [`AuthManager::cached_disk_api_key`].
struct StaticKeyCacheEntry {
    stamp: Option<AuthFileStamp>,
    key: Option<String>,
}

type AuthFileStamp = (u64, Option<std::time::SystemTime>, u64);

fn auth_file_stamp(path: &Path) -> Option<AuthFileStamp> {
    let meta = std::fs::metadata(path).ok()?;
    #[cfg(unix)]
    let ino = std::os::unix::fs::MetadataExt::ino(&meta);
    #[cfg(not(unix))]
    let ino = 0;
    Some((ino, meta.modified().ok(), meta.len()))
}

impl AuthManager {
    fn cached_disk_api_key(&self) -> Option<String> {
        let stamp = auth_file_stamp(&self.path);
        let mut cache = self.static_key_cache.lock();
        match cache.as_ref() {
            Some(entry) if entry.stamp == stamp => entry.key.clone(),
            _ => {
                let key = stamp
                    .is_some()
                    .then(|| api_key_from_auth_file(&self.path))
                    .flatten();
                *cache = Some(StaticKeyCacheEntry {
                    stamp,
                    key: key.clone(),
                });
                key
            }
        }
    }

    /// Set the process model key (empty clears).
    pub fn set_process_static_api_key(&self, key: Option<String>) {
        let key = key.map(|k| k.trim().to_string()).filter(|k| !k.is_empty());
        *self.process_static_api_key.write() = key;
    }
}

fn non_empty_key(key: Option<String>) -> Option<String> {
    key.map(|k| k.trim().to_string()).filter(|k| !k.is_empty())
}

/// Per-request bearer for out-of-crate consumers.
pub fn shared_api_key_provider(
    auth_manager: Arc<AuthManager>,
) -> codel_tools::types::SharedApiKeyProvider {
    Arc::new(SharedAuthKeyProvider(auth_manager))
}

const _: fn() = || {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<AuthManager>();
};


