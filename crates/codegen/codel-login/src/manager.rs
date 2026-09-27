//! `AuthManager` is the single source of truth for `auth.json` and the in-memory bearer cache.
//! Mutations go through `update`; lock and enrichment helpers live in submodules.
use chrono::{Duration, Utc};
use parking_lot::RwLock;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration as StdDuration;
use codel_auth::bearer_suffix;
#[path = "manager/enrichment.rs"]
mod enrichment;
#[path = "manager/lock.rs"]
pub(super) mod lock;
#[path = "manager/remedy.rs"]
mod remedy;
pub use remedy::AuthRemedy;
use super::model::AuthStore;
#[cfg(test)]
use super::model::UserInfo;
use super::model::{
    AuthMode, CodelAuth, early_invalidation, is_expired, is_expired_with_buffer, lookup_auth,
};
#[cfg(test)]
use super::storage::read_auth_json_or_empty;
use super::storage::{
    AuthFileLock, auth_json_path, read_auth_json, read_auth_json_or_empty_recovering_corrupt,
    write_auth_json,
};
use crate::backend::{ActiveAuthBackend, AuthBackend};
use crate::config::CodelComConfig;
use crate::error::AuthError;
use crate::side_call_bearer::non_empty_key;
use crate::token_type::TokenType;
#[cfg(test)]
use chrono::DateTime;
#[cfg(test)]
use enrichment::apply_user_info_enrichment;
use lock::{LockAcquire, try_lock_auth_file_async};
use codel_shell_base::util::dual_clock::DualClock;
use codel_logging::events::ManualAuthSurface;
/// Why [`AuthManager::try_use_disk_token`] (the single enforcement point for disk-token adoption) declined a disk token.
/// Naming the decision, instead of collapsing every decline into a bare `None`, lets callers carry it into the structured log.
/// Tests can assert the exact guard.
#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::AsRefStr, strum::IntoStaticStr)]
#[strum(serialize_all = "snake_case")]
pub enum DiskTokenDecline {
    /// No token on disk for this scope (or `auth.json` was unreadable).
    Missing,
    /// The disk token is expired (buffer-inclusive, like every adopt path).
    Expired,
    /// The disk token was minted before the live in-memory one (beyond skew tolerance).
    /// Disk is lagging memory (`update()` keeps a successful mint in memory when its disk write fails), not a sibling rotation.
    LaggingMemoryMint,
    /// `ServerRejected` only: the disk key matches the rejected bearer, so no sibling has refreshed yet.
    SameKeyAsRejected,
}
/// Timeout for acquiring the advisory `auth.json.lock` file lock.
/// Used by advisory (non-critical) lock sites: `flow.rs`, `enrichment.rs`, `recovery.rs`.
pub const AUTH_LOCK_TIMEOUT: StdDuration = StdDuration::from_secs(10);
/// Back-off interval for the auth-recovery paths.
pub const BACKOFF_INTERVAL: StdDuration = StdDuration::from_secs(300);
/// How long to wait after a file lock timeout before re-reading disk, giving the lock holder time to finish writing.
const LOCK_TIMEOUT_WAIT: StdDuration = StdDuration::from_secs(2);
/// Remaining lifetime a cached token needs for `auth()` to serve it in place of a failed or verdict-blocked refresh.
/// Covers the gap between the pre-request `auth()` and the request leaving the sampler (sub-second in practice).
/// Inside this horizon the dispatch falls through to last-resort recovery or the refresh error instead, so the caller learns there is no usable credential while a mint can still be tried.
const SEND_HORIZON_SECS: i64 = 5;
/// Maximum random jitter (seconds) added to the proactive refresh sleep to stagger sibling processes and avoid thundering-herd IdP calls.
const JITTER_RANGE_SECS: i64 = 60;
/// `force_reload_from_disk` re-read budget. A single `auth.json` read can return `NotFound`/unreadable for reasons unrelated to logout.
/// The most notable is the first read right after wake-from-sleep, where the filesystem briefly resolves the path to `ENOENT`.
/// Retrying a few times absorbs that transient; a genuine deletion/logout stays missing across the budget.
const RELOAD_RETRY_TRIES: usize = 3;
/// Backoff between `force_reload_from_disk` re-reads.
/// Short enough to keep the (sync) caller responsive, long enough to outlast a wake-time FS settle.
/// Only paid on the disk-anomaly branch, never on a healthy read.
const RELOAD_RETRY_BACKOFF: StdDuration = StdDuration::from_millis(50);
/// Sticky permanent-refresh verdict, scoped to the credential that produced it (`token_key`).
/// The scope is what makes invalidation automatic: any other credential reads through as "no failure", so no manual clearing is needed.
struct ScopedRefreshFailure {
    token_key: String,
    error: crate::error::RefreshTokenFailedError,
    /// Two-clock timestamp (see [`DualClock`]): the TTL below is *real* time, so it must keep counting across a system sleep. The monotonic clock pauses during suspend.
    /// A failure cached just before sleep would then short-circuit `auth()` for [`PERMANENT_FAILURE_TTL`] of *awake* time after wake. That is exactly when the user comes back and expects a recovered session.
    recorded_at: DualClock,
}
/// Auto-expiry safety net for the recoverable reasons (`ClientRejected`, `Other`). They self-heal without re-login even if the credential never changes. `RefreshTokenRejected` is excluded (see `is_sticky`).
/// Independent of `BACKOFF_INTERVAL` (equal value is coincidental). Measured on both clocks: it expires once *either* the monotonic or the wall clock passes the bound.
/// It therefore means "5 real minutes", not "5 awake minutes" (a suspend doesn't extend it).
const PERMANENT_FAILURE_TTL: StdDuration = StdDuration::from_secs(300);
/// Redacted `Debug` so `AuthManager` (held via `Arc` inside `Debug`-derived types like `PersistenceMsg`) never leaks credentials into logs or panics.
impl std::fmt::Debug for AuthManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuthManager").finish_non_exhaustive()
    }
}
/// Single source of truth for `auth.json` and the in-memory bearer. Lock order: `refresh_lock` (async), then the sync locks (`inner` / `refresher` / `permanent_failure` / `manual_auth`), never co-held.
/// `permanent_failure()` reads `permanent_failure` first, then `inner` (via `attempted_verdict_key`, when a verdict is stored), never co-held. Never hold a `parking_lot` guard across `.await`.
pub struct AuthManager {
    /// In-memory bearer. Mutate via [`Self::with_inner_write`] or [`Self::update`].
    /// The closure helpers' sync return type enforces "no `.await` while holding the lock".
    /// `Arc` so the spawned `/user` enrichment task can write back.
    inner: Arc<RwLock<Option<CodelAuth>>>,
    path: PathBuf,
    scope: String,
    codel_com_config: CodelComConfig,
    proxy_base_url: String,
    permanent_failure: RwLock<Option<ScopedRefreshFailure>>,
    /// Last state `read_disk_auth` observed for this manager's scope.
    /// Drives transition-level unified logging: hot retry loops read the disk every few seconds, so per-read logging would flood.
    /// No logging at all would leave auth.json loss invisible in production captures.
    disk_state: RwLock<Option<DiskAuthState>>,
    /// See [`Self::cached_disk_api_key`].
    static_key_cache: parking_lot::Mutex<Option<StaticKeyCacheEntry>>,
    /// Model `api_key` / resolved `env_key` for voice/tools without a session.
    /// Not a session token (those live on `inner`). This key is preferred over the disk key; the env key wins.
    process_static_api_key: parking_lot::RwLock<Option<String>>,
    /// Per-process `manual_auth` KPI debounce, shared by all recoveries on this manager.
    /// Repeated 401s on the most-recent dead credential emit once.
    manual_auth: crate::recovery::ManualAuthTracker,
    /// First-party env key may advertise after initialize probe (default true).
    /// Lives here (not on `MvpAgent`) so the probe verdict is auth-owned.
    first_party_env_api_key_ok: std::sync::atomic::AtomicBool,
}
/// Discriminated outcome of a disk read, for transition logging.
/// `Ok` means the entry is present (possibly expired); the rest explain *why* `read_disk_auth` returned `None`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiskAuthState {
    /// auth.json readable and the scope entry exists.
    Ok,
    /// auth.json does not exist.
    FileMissing,
    /// auth.json readable but has no usable entry for this scope (scope removed, or only a skipped legacy WebLogin entry).
    EntryMissing,
    /// auth.json exists but could not be read (corrupt JSON, permission or I/O error).
    Unreadable,
}
/// [`AuthManager::cached_token_state`]'s point-in-time classification of the cached credential.
#[derive(Debug, Clone)]
pub enum CachedTokenState {
    /// Nothing cached, another authority's session, or a valid token the login policy hides.
    Missing,
    /// Serves on the wire right now. Carries what [`AuthManager::current`] would
    /// return so callers never re-read; boxed because `CodelAuth` is large and
    /// the other variants are unit-sized.
    Valid(Box<CodelAuth>),
    /// Cached but past the early-invalidation buffer (what [`AuthManager::is_expired`] reports).
    Expired,
}
/// On-disk outcome of [`AuthManager::remove_scope_impl`].
/// It is emitted as the `disk_mutation` field of the `auth: scope removed from auth.json` event.
/// A deliberate removal thus stays distinguishable from accidental credential loss.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScopeRemoval {
    /// Scope entry dropped; other scopes remain.
    EntryRemoved,
    /// Last scope dropped; auth.json deleted.
    FileDeleted,
    /// Lock unavailable (held by another process); disk left untouched.
    SkippedLockUnavailable,
    /// Lock held but auth.json was unreadable; disk left untouched.
    SkippedUnreadable,
}
impl ScopeRemoval {
    /// Stable telemetry label for the `disk_mutation` field.
    fn label(self) -> &'static str {
        match self {
            Self::EntryRemoved => "entry removed",
            Self::FileDeleted => "file deleted (no scopes left)",
            Self::SkippedLockUnavailable => "skipped (lock unavailable)",
            Self::SkippedUnreadable => "skipped (auth.json unreadable)",
        }
    }
}
impl AuthManager {
    /// Public default cli-chat-proxy base URL, mirroring `agent::config::CLI_CHAT_PROXY_BASE_URL_DEFAULT`.
    #[cfg(any(test, feature = "test-support"))]
    const DEFAULT_PROXY_BASE_URL: &str = "https://cli-chat-proxy.codel.dev/v1";
    /// Test/support-only convenience against the public default proxy. Production callers resolve the
    /// configured proxy and pass it via [`Self::new_with_proxy_base_url`], so this boundary never
    /// silently sends enrichment to the public host.
    #[cfg(any(test, feature = "test-support"))]
    pub fn new(codel_home: &Path, codel_com_config: CodelComConfig) -> Self {
        Self::new_with_proxy_base_url(
            codel_home,
            codel_com_config,
            Self::DEFAULT_PROXY_BASE_URL.to_string(),
        )
    }
    pub fn new_with_proxy_base_url(
        codel_home: &Path,
        codel_com_config: CodelComConfig,
        proxy_base_url: String,
    ) -> Self {
        let scope = ActiveAuthBackend::default().scope_key(&codel_com_config);
        codel_logging::unified_log::info(
            "AuthManager::new",
            None,
            Some(serde_json::json!({
                "scope": &scope,
                "codel_home": codel_home.display().to_string(),
                "HOME": std::env::var("HOME").unwrap_or_else(|_| "(unset)".into()),
                "CODEL_HOME": std::env::var("CODEL_HOME").unwrap_or_else(|_| "(unset)".into()),
                "CODEL_AUTH_PATH": std::env::var("CODEL_AUTH_PATH").unwrap_or_else(|_| "(unset)".into()),
                "CODEL_AUTH": std::env::var("CODEL_AUTH").map(|_| "(set)".to_string()).unwrap_or_else(|_| "(unset)".into()),
            })),
        );
        let path = auth_json_path(codel_home);
        if let Ok(inline_json) = std::env::var("CODEL_AUTH") {
            if let Ok(auth) = serde_json::from_str::<CodelAuth>(&inline_json) {
                return Self::assemble(
                    Some(auth),
                    path,
                    scope,
                    codel_com_config,
                    proxy_base_url,
                    None,
                );
            }
            tracing::warn!("CODEL_AUTH set but failed to parse as JSON, falling back to file");
        }
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
                    "key_prefix": found.as_ref().map(|a| bearer_suffix(&a.key).to_owned()),
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
        codel_logging::unified_log::info(
            "AuthManager::new auth.json load result",
            None,
            Some(auth_read_detail),
        );
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
    /// Single field-assembly point for [`Self::new`]'s two construction paths (inline `CODEL_AUTH` vs. on-disk `auth.json`), which differ only in the threaded fields. One literal means a newly added field can't be silently dropped from one branch.
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
            permanent_failure: RwLock::new(None),
            disk_state: RwLock::new(disk_state),
            static_key_cache: parking_lot::Mutex::new(None),
            process_static_api_key: parking_lot::RwLock::new(None),
            manual_auth: Default::default(),
            first_party_env_api_key_ok: std::sync::atomic::AtomicBool::new(true),
        }
    }
    /// Whether initialize's first-party env-key probe still allows advertising.
    pub fn first_party_env_api_key_ok(&self) -> bool {
        self.first_party_env_api_key_ok
            .load(std::sync::atomic::Ordering::Relaxed)
    }
    /// Record the initialize probe result so a cached-token fallthrough can still advertise the env key.
    pub fn set_first_party_env_api_key_ok(&self, ok: bool) {
        self.first_party_env_api_key_ok
            .store(ok, std::sync::atomic::Ordering::Relaxed);
    }
    /// Clear the disk-loaded token if it violates the team pin (startup only; the read/dispense gates cover everything cached afterwards).
    fn enforce_pin_on_loaded_token(&self) {
        let loaded = self.inner.read().clone();
        if let Some(auth) = loaded
            && let Some(e) = self.cached_token_policy_error(&auth)
        {
            self.reject_and_clear(&e);
        }
    }
    /// Override the proxy base URL (precedence over env var).
    pub fn with_proxy_base_url(mut self, url: &str) -> Self {
        self.proxy_base_url = url.to_owned();
        self
    }
    /// Proxy base URL this manager was built with (see `with_proxy_base_url`).
    pub fn proxy_base_url(&self) -> &str {
        &self.proxy_base_url
    }
    pub fn clear(&self) -> std::io::Result<()> {
        self.remove_scope(&self.scope)
    }
    /// Remove a scope entry from auth.json. When `scope == self.scope`, also drops in-memory auth so a later `auth()` reports `NotLoggedIn`, not stale `invalid_grant`. (The scoped verdict reads inert with no credential.)
    /// When the last scope goes, the file is deleted. Best-effort: takes a non-blocking lock and skips the disk write if another process holds it (the stale entry is cleaned up on next launch).
    pub fn remove_scope(&self, scope: &str) -> std::io::Result<()> {
        self.remove_scope_impl(scope)
    }
    fn remove_scope_impl(&self, scope: &str) -> std::io::Result<()> {
        let disk_mutation = if let Some(_lock) = lock::try_lock_auth_file_nonblocking(&self.path) {
            self.write_scope_removal(scope)?
        } else {
            ScopeRemoval::SkippedLockUnavailable
        };
        codel_logging::unified_log::warn(
            "auth: scope removed from auth.json",
            None,
            Some(serde_json::json!({
                "scope": scope,
                "is_current_scope": scope == self.scope,
                "disk_mutation": disk_mutation.label(),
                "path": self.path.display().to_string(),
            })),
        );
        if scope == self.scope {
            self.clear_inner();
            *self.permanent_failure.write() = None;
        }
        Ok(())
    }
    /// Drop `scope` from auth.json and persist, deleting the file when the last scope is gone.
    /// Caller holds the `auth.json` lock (taken by [`Self::remove_scope_impl`]).
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
    /// Drop the in-memory auth.
    /// Sticky `RefreshTokenRejected` still short-circuits with no live credential until a wire-valid login.
    /// Non-sticky verdicts read absent once their scoped key is gone.
    fn clear_inner(&self) {
        *self.inner.write() = None;
    }
    /// Re-read `auth.json` and reconcile the in-memory cache with it.
    /// A disk read returning "no usable token" has very different meanings that must not be conflated: [`DiskAuthState::EntryMissing`]: the file is readable but our scope is gone.
    /// This is the trustworthy "logged out / scope removed" signal. The in-memory credentials (and any cached permanent_failure) are dropped together. The classic case is the first read after wake-from-sleep transiently resolving `auth.json` to `ENOENT`. This is **not** proof the credentials are gone, so we retry briefly. If it persists, we retain a still-live in-memory refresh token rather than discard the only copy.
    pub fn force_reload_from_disk(&self) {
        self.force_reload_from_disk_with(RELOAD_RETRY_TRIES, RELOAD_RETRY_BACKOFF);
    }
    /// Inner of [`force_reload_from_disk`] with the retry budget injectable so the disk-anomaly branch is unit-testable without real sleeps.
    fn force_reload_from_disk_with(&self, tries: usize, backoff: StdDuration) {
        let mut last_state = DiskAuthState::FileMissing;
        for attempt in 0..tries.max(1) {
            if attempt > 0 && !backoff.is_zero() {
                std::thread::sleep(backoff);
            }
            let (auth, state) = self.read_disk_auth_with_state();
            last_state = state;
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
        let in_mem = self.current_or_expired();
        let sticky_verdict = matches!(
            self.permanent_failure(),
            Some(AuthError::Refresh(crate::error::RefreshTokenError::Permanent(ref e)))
                if e.reason.is_sticky()
        );
        let retain = in_mem.as_ref().is_some_and(|a| a.refresh_token.is_some()) && !sticky_verdict;
        if let Some(a) = in_mem.filter(|_| retain) {
            codel_logging::unified_log::warn(
                "auth: disk anomaly, retaining in-memory credentials",
                None,
                Some(serde_json::json!({
                    "disk_state": format!("{last_state:?}"),
                    "retained_key_prefix": bearer_suffix(&a.key),
                    "was_expired": is_expired(&a),
                })),
            );
        } else {
            self.drop_in_memory_credentials(
                "disk anomaly; no live refresh token to retain (missing RT or permanent failure)",
            );
        }
        self.enforce_pin_on_loaded_token();
    }
    /// Drop the in-memory credentials, loudly. Logs the discard (with `reason`) before routing through [`clear_inner`].
    /// Also clears a sticky permanent verdict so force-reload / disk-anomaly paths report `NotLoggedIn` rather than a retained `invalid_grant`.
    /// Permanent discard after a live IdP rejection uses [`clear_inner`] alone so the sticky short-circuit survives until login.
    fn drop_in_memory_credentials(&self, reason: &str) {
        if let Some(d) = self.current_or_expired() {
            codel_logging::unified_log::warn(
                "auth: in-memory credentials dropped (disk reload found none)",
                None,
                Some(serde_json::json!({
                    "reason": reason,
                    "dropped_key_prefix": bearer_suffix(&d.key),
                    "had_refresh_token": d.refresh_token.is_some(),
                    "was_expired": is_expired(&d),
                    "disk_state": (*self.disk_state.read()).map(|s| format!("{s:?}")),
                })),
            );
        }
        self.clear_inner();
        *self.permanent_failure.write() = None;
    }
    /// `Some(error)` when the credential must be refused.
    ///
    /// The only remaining rejection is the `api_key_auth_disabled` kill switch:
    /// the `force_login_team_uuid` pin it used to enforce was an OIDC login
    /// policy, and the fork has no OIDC login.
    pub fn cached_token_policy_error(&self, _auth: &CodelAuth) -> Option<AuthError> {
        self.codel_com_config
            .api_key_auth_disabled()
            .then_some(AuthError::ApiKeyAuthDisabled)
    }
    /// Log and clear a policy-violating session (disk and memory) so the next launch forces a fresh, compliant login.
    pub(crate) fn reject_and_clear(&self, error: &AuthError) {
        let policy = match error {
            AuthError::ApiKeyAuthDisabled => "api_key_disabled",
            _ => "credential_policy",
        };
        codel_logging::unified_log::warn(
            "auth: cached session rejected by login policy; clearing",
            None,
            Some(serde_json::json!({ "policy": policy, "reason": error.to_string() })),
        );
        if let Err(e) = self.clear() {
            tracing::warn!(error = %e, "auth: failed to clear policy-violating session");
        }
    }
    /// Every accessor that hands a credential to a caller reads `inner` through here.
    /// The direct reads left elsewhere compare token keys or look at `expires_at`, and hand out nothing.
    fn owned_inner(&self) -> Option<CodelAuth> {
        let auth = self.with_inner_read(|inner| inner.cloned())?;
        if !crate::backend::AuthBackend::owns(&crate::backend::ActiveAuthBackend::default(), &auth)
        {
            tracing::debug!("auth: hiding a cached session another authority minted");
            return None;
        }
        Some(auth)
    }
    /// Hide a cached token rejected by the login policy.
    /// No clear here (keeps the sync read path lock-free); `auth()`/recovery/`new()` do the clearing.
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
    pub fn current(&self) -> Option<CodelAuth> {
        let auth = self.owned_inner().filter(|a| !self.is_token_expired(a))?;
        self.vet_cached(auth)
    }
    /// Closure-scoped write. Sync return type prevents `.await` while the lock is held.
    /// Prefer this over `self.inner.write()`.
    #[inline]
    pub(crate) fn with_inner_write<R>(&self, f: impl FnOnce(&mut Option<CodelAuth>) -> R) -> R {
        let mut guard = self.inner.write();
        f(&mut guard)
    }
    /// Closure-scoped read counterpart to [`Self::with_inner_write`].
    #[inline]
    pub(crate) fn with_inner_read<R>(&self, f: impl FnOnce(Option<&CodelAuth>) -> R) -> R {
        let guard = self.inner.read();
        f(guard.as_ref())
    }
    /// Returns true if credentials exist but have expired.
    pub fn is_expired(&self) -> bool {
        self.owned_inner()
            .is_some_and(|a| self.is_token_expired(&a))
    }
    /// [`Self::current`] and [`Self::is_expired`] classified from one inner read, for callers that need both facts about the same credential: two separate reads let a refresh landing in between answer "no current token" and "not expired" at once.
    pub fn cached_token_state(&self) -> CachedTokenState {
        let Some(auth) = self.owned_inner() else {
            return CachedTokenState::Missing;
        };
        if self.is_token_expired(&auth) {
            return CachedTokenState::Expired;
        }
        match self.vet_cached(auth) {
            Some(auth) => CachedTokenState::Valid(Box::new(auth)),
            None => CachedTokenState::Missing,
        }
    }
    /// In-memory bearer regardless of the early-invalidation buffer.
    /// Prefer [`Self::auth`] when `.await` is available.
    pub fn current_or_expired(&self) -> Option<CodelAuth> {
        self.current().or_else(|| self.expired_auth())
    }
    /// Cached token if still wire-valid ([`Self::is_token_hard_expired`]), ignoring the early-invalidation buffer.
    /// For sync callers that cannot refresh and must not demote a still-accepted token.
    pub fn current_wire_valid(&self) -> Option<CodelAuth> {
        let auth = self
            .owned_inner()
            .filter(|a| !self.is_token_hard_expired(a))?;
        self.vet_cached(auth)
    }
    /// `true` when data collection must be suppressed: the team has ZDR or the user opted out of coding data retention.
    /// Reads [`Self::current_or_expired`] because neither flag changes on token expiry and `current()` returns `None` during the refresh window. Fail-open: with no credential this returns `false` (not disabled).
    /// Collection paths that must not act on unknown privacy state should use the fail-closed [`Self::allows_data_collection`] instead.
    pub fn is_data_collection_disabled(&self) -> bool {
        self.current_or_expired()
            .is_some_and(|a| a.is_data_collection_disabled())
    }
    /// Fail-closed collection predicate: `true` only when a credential exists and carries no ZDR / retention-opt-out flag.
    /// Missing or cleared auth (e.g. after a mid-session `/logout`) counts as disabled.
    /// Nothing may leave the machine while the privacy state is unknown.
    pub fn allows_data_collection(&self) -> bool {
        self.current_or_expired()
            .is_some_and(|a| !a.is_data_collection_disabled())
    }
    /// Expired in-memory entry (for its `refresh_token`).
    pub fn expired_auth(&self) -> Option<CodelAuth> {
        let auth = self.owned_inner().filter(|a| self.is_token_expired(a))?;
        self.vet_cached(auth)
    }
    /// Expiry policy: `expires_at - early_invalidation` if present.
    /// `External` with `auth_token_ttl` expires at `create_time + ttl`; the fallback is `create_time + 30d` (WebLogin-style).
    fn is_token_expired(&self, auth: &CodelAuth) -> bool {
        self.token_expired_with_buffer(auth, early_invalidation())
    }
    /// Actual (hard) expiry: the instant the proxy would actually reject the token, with no early-invalidation margin.
    /// The export gate ([`Self::has_usable_token`]) uses this instead of [`Self::is_token_expired`].
    /// A token still inside the buffer is sent (and accepted) on the wire via `current_or_expired()`, so it must not count as unusable.
    fn is_token_hard_expired(&self, auth: &CodelAuth) -> bool {
        self.token_expired_with_buffer(auth, Duration::zero())
    }
    /// Whether a cached token can be handed out without a refresh when the refresh authority is unavailable.
    /// Stricter than [`Self::is_token_hard_expired`] by [`SEND_HORIZON_SECS`]: the sampler's send-time resolver is wire-valid only.
    /// A token served here with milliseconds left is stripped before the request leaves, which goes out with no credential at all and 401s.
    fn outlives_send_horizon(&self, auth: &CodelAuth) -> bool {
        !self.token_expired_with_buffer(auth, Duration::seconds(SEND_HORIZON_SECS))
    }
    /// Whether the cached bearer would still be on the wire after the pre-flight→send gap ([`Self::outlives_send_horizon`]).
    /// The sampler's pre-send hook and the external refresher's cooldown both key off this: with `false` there is nothing left to serve, so a refresh attempt is the only way a request carries a credential.
    pub(crate) fn has_sendable_token(&self) -> bool {
        self.current_wire_valid()
            .is_some_and(|a| self.outlives_send_horizon(&a))
    }
    /// How long the cached bearer stays wire-valid, or `None` when there is no wire-valid bearer.
    /// A pre-send refresh must not wait past this: it would outlive the very token it was protecting and the request would leave with none.
    pub(crate) fn remaining_wire_life(&self) -> Option<StdDuration> {
        let auth = self.current_wire_valid()?;
        let expires_at = match auth.expires_at {
            Some(at) => at,
            None => auth.create_time + super::model::TOKEN_TTL,
        };
        expires_at.signed_duration_since(Utc::now()).to_std().ok()
    }
    fn token_expired_with_buffer(&self, auth: &CodelAuth, buffer: Duration) -> bool {
        if auth.expires_at.is_some() {
            return is_expired_with_buffer(auth, buffer);
        }
        is_expired_with_buffer(auth, buffer)
    }
    /// Persist rotated tokens to disk and cache, then spawn `/user` enrichment. Invariants: **Disk write before any network I/O** (else a sibling process can reuse the not-yet-rotated RT and the IdP returns `invalid_grant`).
    /// **Caller holds the `auth.json` file lock** (the production caller is `update`).
    /// Returns the input `CodelAuth` BEFORE enrichment lands; callers needing the post-enrichment view re-read `current()`.
    pub async fn update(self: &Arc<Self>, auth: CodelAuth) -> std::io::Result<CodelAuth> {
        let update_started = std::time::Instant::now();
        let map = match read_auth_json_or_empty_recovering_corrupt(&self.path) {
            Ok(map) => map,
            Err(e) => {
                tracing::warn!(error = %e, "auth: read failed, updating in-memory only");
                codel_logging::unified_log::warn(
                    "auth update skipped disk write (read failed)",
                    None,
                    Some(serde_json::json!({ "error": e.to_string() })),
                );
                self.with_inner_write(|inner| *inner = Some(auth.clone()));
                self.spawn_user_info_enrichment(auth.clone());
                return Ok(auth);
            }
        };
        let mut map = map;
        tracing::debug!(scope = %self.scope, "auth: storing token");
        map.insert(self.scope.clone(), auth.clone());
        let write_result = write_auth_json(&self.path, &map);
        let elapsed_ms = update_started.elapsed().as_millis() as u64;
        match &write_result {
            Ok(()) => codel_logging::unified_log::info(
                "auth update disk written",
                None,
                Some(serde_json::json!({
                    "rt_prefix": auth.refresh_token.as_deref().map(bearer_suffix),
                    "key_prefix": bearer_suffix(&auth.key),
                    "elapsed_ms": elapsed_ms,
                })),
            ),
            Err(e) => codel_logging::unified_log::error(
                "auth update disk write failed",
                None,
                Some(serde_json::json!({
                    "error": e.to_string(),
                    "elapsed_ms": elapsed_ms,
                })),
            ),
        }
        *self.permanent_failure.write() = None;
        self.with_inner_write(|inner| *inner = Some(auth.clone()));
        self.spawn_user_info_enrichment(auth.clone());
        write_result?;
        Ok(auth)
    }
    /// Persist to disk and cache without spawning the background `/user` task (already merged inline, or a stale fetch must not race a fresh write).
    pub async fn save_without_enrichment(&self, auth: CodelAuth) -> std::io::Result<CodelAuth> {
        let started = std::time::Instant::now();
        let map = match read_auth_json_or_empty_recovering_corrupt(&self.path) {
            Ok(map) => map,
            Err(e) => {
                tracing::warn!(error = %e, "auth: read failed, updating in-memory only (no enrichment)");
                codel_logging::unified_log::warn(
                    "auth update skipped disk write (read failed, no enrichment)",
                    None,
                    Some(serde_json::json!({ "error": e.to_string() })),
                );
                self.with_inner_write(|inner| *inner = Some(auth.clone()));
                return Ok(auth);
            }
        };
        let mut map = map;
        tracing::debug!(scope = %self.scope, "auth: storing token (no enrichment)");
        map.insert(self.scope.clone(), auth.clone());
        let write_result = write_auth_json(&self.path, &map);
        let elapsed_ms = started.elapsed().as_millis() as u64;
        match &write_result {
            Ok(()) => codel_logging::unified_log::info(
                "auth update disk written (no enrichment)",
                None,
                Some(serde_json::json!({
                    "rt_prefix": auth.refresh_token.as_deref().map(bearer_suffix),
                    "key_prefix": bearer_suffix(&auth.key),
                    "elapsed_ms": elapsed_ms,
                })),
            ),
            Err(e) => codel_logging::unified_log::error(
                "auth update disk write failed (no enrichment)",
                None,
                Some(serde_json::json!({
                    "error": e.to_string(),
                    "elapsed_ms": elapsed_ms,
                })),
            ),
        }
        *self.permanent_failure.write() = None;
        self.with_inner_write(|inner| *inner = Some(auth.clone()));
        write_result?;
        Ok(auth)
    }
    /// Spawn the `/user` enrichment task; body in the `enrichment` submodule.
    /// `/user` lives on the Codel proxy, so a build pointed elsewhere would send its bearer to the wrong host.
    /// That would happen on every login and every refresh.
    fn spawn_user_info_enrichment(self: &Arc<Self>, auth: CodelAuth) {
        if !ActiveAuthBackend::default().is_codel_authority() {
            return;
        }
        enrichment::spawn(Arc::clone(self), auth);
    }
    /// Blocking `/user` enrichment for login flows that exit before the background task lands.
    pub(crate) async fn enrich_auth_inline(&self, auth: &mut CodelAuth) {
        enrichment::enrich_inline(self, auth).await;
    }
    /// Answers only for the account the caller named (an absent email never matches) and fetches only for a Team principal: a User principal gets `null` from `/user` on every call.
    pub async fn hydrate_can_administer_team(
        &self,
        email: Option<&str>,
        team_id: Option<&str>,
    ) -> Option<bool> {
        let auth = self.current()?;
        if email.is_none() || auth.email.as_deref() != email || auth.team_id.as_deref() != team_id {
            return None;
        }
        if auth.can_administer_team.is_some()
            || !auth.is_codel_auth()
            || !auth.is_team_principal()
            || !ActiveAuthBackend::default().is_codel_authority()
        {
            return auth.can_administer_team;
        }
        enrichment::hydrate_can_administer_team(self, &auth).await
    }
    /// Path to the `auth.json` this manager reads/writes (respects `CODEL_AUTH_PATH` / constructor home).
    /// Prefer this over `codel_home()/auth.json` so temp-home tests and custom stores stay isolated.
    pub fn auth_json_path(&self) -> &Path {
        &self.path
    }
    pub fn codel_com_config(&self) -> &CodelComConfig {
        &self.codel_com_config
    }
    /// Hot-swap credentials (called by config watcher). Does NOT write to disk.
    /// Clears a sticky permanent verdict only when the new bearer is wire-valid (login / sibling adopt).
    /// Hard-expired swaps keep the sticky short-circuit so a dead RT is not re-tried until a real login.
    pub fn hot_swap(&self, new_auth: CodelAuth) {
        if !self.is_token_hard_expired(&new_auth) {
            *self.permanent_failure.write() = None;
        }
        self.with_inner_write(|inner| *inner = Some(new_auth));
    }
    /// Clear in-memory credentials. Does NOT touch disk.
    /// Sticky `RefreshTokenRejected` remains until wire-valid login; other verdicts are key-scoped and drop out once their credential is gone.
    pub fn clear_in_memory(&self) {
        self.clear_inner();
    }
    /// Accept a sibling-written disk token. Single enforcement point for disk adoption, so the guards and the shared `hot_swap` cannot drift between call sites.
    pub(crate) fn try_use_disk_token(
        &self,
        disk_auth: Option<&CodelAuth>,
    ) -> Result<CodelAuth, DiskTokenDecline> {
        let Some(disk_auth) = disk_auth else {
            return Err(DiskTokenDecline::Missing);
        };
        if self.is_token_expired(disk_auth) {
            return Err(DiskTokenDecline::Expired);
        }
        const DISK_MINT_SKEW_TOLERANCE: Duration = Duration::seconds(60);
        if let Some(current) = self.current_or_expired()
            && disk_auth.create_time + DISK_MINT_SKEW_TOLERANCE < current.create_time
        {
            return Err(DiskTokenDecline::LaggingMemoryMint);
        }
        tracing::info!("auth: another process already refreshed, using disk token");
        self.hot_swap(disk_auth.clone());
        Ok(disk_auth.clone())
    }
    /// Re-read disk and try to adopt a sibling-written token, emitting telemetry on success.
    fn try_adopt_disk_token(&self, msg: &str) -> Option<CodelAuth> {
        let disk_auth = self.read_disk_auth();
        let prev = self
            .current_or_expired()
            .map(|a| bearer_suffix(&a.key).to_owned());
        let refreshed = match self.try_use_disk_token(disk_auth.as_ref()) {
            Ok(refreshed) => refreshed,
            Err(
                decline @ (DiskTokenDecline::LaggingMemoryMint
                | DiskTokenDecline::SameKeyAsRejected),
            ) => {
                codel_logging::unified_log::info(
                    "auth: disk token declined",
                    None,
                    Some(serde_json::json!({
                        "decline": decline.as_ref(),
                        "prev_key_prefix": prev,
                        "disk_key_prefix": disk_auth.as_ref().map(|a| bearer_suffix(&a.key)),
                    })),
                );
                return None;
            }
            Err(_) => return None,
        };
        let adopted = bearer_suffix(&refreshed.key);
        codel_logging::unified_log::info(
            msg,
            None,
            Some(serde_json::json!({
                "adopted_key_prefix": adopted,
                "prev_key_prefix": prev,
                "key_changed": prev.as_deref() != Some(adopted),
            })),
        );
        Some(refreshed)
    }
    /// Test-only hot_swap and disk write (skips proxy `/user`).
    /// Production persistence routes through `update()`.
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
    pub fn read_disk_auth(&self) -> Option<CodelAuth> {
        self.read_disk_auth_with_state().0
    }
    /// Disk read for the configured scope with NO observation side effects (no `disk_state` write, no transition telemetry).
    /// For side-effect-free getters like [`Self::attempted_verdict_key`].
    /// Prefer [`Self::read_disk_auth`] when the read should drive transition logging.
    fn read_disk_auth_silent(&self) -> Option<CodelAuth> {
        read_auth_json(&self.path)
            .ok()
            .and_then(|map| lookup_auth(&map, &self.scope))
    }
    /// Wire-valid token present in on-disk `auth.json`, judged by actual expiry ([`Self::is_token_hard_expired`]).
    /// Never mutates in-memory state, unlike [`Self::force_reload_from_disk`].
    pub fn has_usable_disk_token(&self) -> bool {
        self.read_disk_auth()
            .is_some_and(|a| !self.is_token_hard_expired(&a))
    }
    /// Whether a wire-valid token is available in memory or on disk: a credential worth a real outbound attempt.
    /// Judged by actual expiry so it mirrors the `current_or_expired()` bearer the senders put on the wire.
    /// A token inside the early-invalidation buffer still counts.
    pub fn has_usable_token(&self) -> bool {
        self.current_or_expired()
            .is_some_and(|a| !self.is_token_hard_expired(&a))
            || self.has_usable_disk_token()
    }
    /// Like [`read_disk_auth`] but also returns the [`DiskAuthState`].
    /// Callers can then tell a transient disk anomaly (`FileMissing`/`Unreadable`) apart from a genuine logout (`EntryMissing`).
    /// Observes the state for transition logging, exactly like `read_disk_auth`.
    pub fn read_disk_auth_with_state(&self) -> (Option<CodelAuth>, DiskAuthState) {
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
    /// Transition-level unified logging for the on-disk auth state: exactly one line per state change.
    /// Hot retry loops must produce neither a log flood nor silence.
    /// One attributable event fires at the moment auth.json disappears (and one when it returns).
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
            "key_prefix": auth.map(|a| bearer_suffix(&a.key).to_owned()),
            "has_refresh_token": auth.map(|a| a.refresh_token.is_some()),
            "is_expired": auth.map(is_expired),
        });
        match new_state {
            DiskAuthState::Ok => {
                codel_logging::unified_log::info(
                    "auth disk state: entry present",
                    None,
                    Some(ctx),
                );
            }
            DiskAuthState::FileMissing
            | DiskAuthState::EntryMissing
            | DiskAuthState::Unreadable => {
                codel_logging::unified_log::warn(
                    "auth disk state: entry lost",
                    None,
                    Some(ctx),
                );
            }
        }
    }
    #[tracing::instrument(name = "auth.lock_wait", skip_all)]
    pub async fn try_lock_auth_file_async(
        &self,
        timeout: StdDuration,
        heartbeat: lock::Heartbeat,
    ) -> LockAcquire {
        try_lock_auth_file_async(&self.path, timeout, heartbeat).await
    }
    /// Classify the loaded credential.
    pub(crate) fn token_type(&self) -> TokenType {
        TokenType::from_auth(self.owned_inner().as_ref())
    }
    /// Pre-request entry point: per-`TokenType` dispatch. For just the key: [`Self::get_valid_token`].
    ///
    /// Also the team-pin gate: a cached/refreshed wrong-team session is cleared and rejected here, never handed to a consumer.
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
        let snapshot: Option<CodelAuth> = self.owned_inner();
        let token_type = TokenType::from_auth(snapshot.as_ref());
        tracing::Span::current().record("token_type", tracing::field::debug(token_type));
        if let Some(ref auth) = snapshot
            && !self.is_token_expired(auth)
        {
            return Ok(auth.clone());
        }
        if let Some(err) = self.permanent_failure() {
            if let Some(ref auth) = snapshot
                && self.outlives_send_horizon(auth)
            {
                return Ok(auth.clone());
            }
            if let Some(refreshed) =
                self.try_adopt_disk_token("auth: adopted sibling token during PermanentFailure in auth()")
            {
                return Ok(refreshed);
            }
            return Err(err);
        }
        let dispatch = async {
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
        };
        dispatch.await
    }
    /// Return the current valid token string, or an error.
    pub async fn get_valid_token(self: &Arc<Self>) -> Result<String, AuthError> {
        self.auth().await.map(|a| a.key)
    }
    /// Re-read auth.json from disk and update the in-memory cache (used by the refresh chains). Non-destructive: it updates in-memory only if disk has a different valid token.
    /// The token must pass the shared adoption guards in [`Self::try_use_disk_token`]. (That means a sibling process wrote a fresher one.) Returns `true` only when in-memory state was actually replaced.
    /// Callers can then log adoption truthfully instead of inferring it from "we have a token now". That inference is also true when our own token was fine all along. It made the proactive-refresh log actively misleading when reconstructing a rotation chain after an incident.
    pub fn pick_up_sibling_token(&self) -> bool {
        let auth = match read_auth_json(&self.path) {
            Ok(map) => lookup_auth(&map, &self.scope),
            _ => None,
        };
        let Some(auth) = auth.filter(|a| self.is_different_token(a)) else {
            return false;
        };
        match self.try_use_disk_token(Some(&auth)) {
            Ok(adopted) => {
                codel_logging::unified_log::info(
                    "auth: pick_up_sibling_token adopted",
                    None,
                    Some(serde_json::json!({
                        "adopted_key_prefix": bearer_suffix(&adopted.key),
                        "expires_at": adopted.expires_at.map(|e| e.to_rfc3339()),
                        "rt_prefix": adopted.refresh_token.as_deref().map(bearer_suffix),
                    })),
                );
                true
            }
            Err(decline) => {
                tracing::debug!(
                    decline = decline.as_ref(),
                    "auth: sibling disk token declined"
                );
                false
            }
        }
    }
    /// Check if a candidate auth has a different token than what's in memory.
    pub fn is_different_token(&self, candidate: &CodelAuth) -> bool {
        let current_key = self.inner.read().as_ref().map(|a| a.key.clone());
        current_key.as_deref() != Some(&candidate.key)
    }
    /// Record a permanent-failure verdict scoped to `token_key` (the rejected credential).
    pub fn record_permanent_failure(
        &self,
        token_key: String,
        error: crate::error::RefreshTokenFailedError,
    ) {
        let ttl_seconds = (!error.reason.is_sticky()).then(|| PERMANENT_FAILURE_TTL.as_secs());
        codel_logging::unified_log::warn(
            "auth.permanent_failure.set",
            None,
            Some(serde_json::json!({
                "reason": format!("{:?}", error.reason),
                "message": error.reason.user_message(),
                "ttl_seconds": ttl_seconds,
            })),
        );
        *self.permanent_failure.write() = Some(ScopedRefreshFailure {
            token_key,
            error,
            recorded_at: DualClock::now(),
        });
    }
    /// Reads the stored verdict first (cheap lock): the common no-verdict case returns before any disk I/O. Only a stored verdict triggers [`Self::attempted_verdict_key`]'s disk read.
    /// After a permanent failure **discards** credentials, sticky reasons (`RefreshTokenRejected`) still short-circuit with no live credential. Concurrent callers therefore cannot re-hit the IdP with a dead RT.
    /// Sticky applies only to the **same** rejected key or to **no** live credential (post-discard). A different attempted key (sibling RT/AT on disk) must be allowed to refresh. Without it, a recoverable failure cached just before the lid closes would keep short-circuiting `auth()`.
    pub fn permanent_failure(&self) -> Option<AuthError> {
        let (token_key, reason) = {
            let guard = self.permanent_failure.read();
            let pf = guard.as_ref()?;
            if !pf.error.reason.is_sticky() {
                let (mono, wall) = pf.recorded_at.elapsed();
                if mono >= PERMANENT_FAILURE_TTL || wall >= PERMANENT_FAILURE_TTL {
                    return None;
                }
            }
            (pf.token_key.clone(), pf.error.reason)
        };
        let _ = token_key;
        reason.is_sticky().then(|| AuthError::permanent(reason))
    }
    /// `true` iff [`Self::permanent_failure`] has a non-expired entry.
    /// Lets callers peek the IdP verdict without touching its `message` payload.
    pub fn has_permanent_failure(&self) -> bool {
        self.permanent_failure().is_some()
    }
    /// Whether the only way back is a new API key.
    ///
    /// `true` on a sticky server rejection, and whenever no credential is on the
    /// wire — nothing can be refreshed, so a missing key has to be configured
    /// again. `false` for anything that self-heals (transient failures,
    /// recoverable verdicts).
    pub fn requires_manual_reauth(&self) -> bool {
        use crate::error::RefreshTokenError;
        if let Some(AuthError::Refresh(RefreshTokenError::Permanent(e))) = self.permanent_failure()
            && e.reason.blocks_unattended_retry()
        {
            return true;
        }
        self.current_wire_valid().is_none() && self.read_disk_auth_silent().is_none()
    }
    /// Test-only: age the cached `permanent_failure` past its TTL so the `permanent_failure()` getter treats it as expired.
    #[cfg(any(test, feature = "test-support"))]
    pub fn force_permanent_failure_aged_out(&self) {
        if let Some(pf) = self.permanent_failure.write().as_mut() {
            let past_ttl = PERMANENT_FAILURE_TTL + StdDuration::from_secs(1);
            let now_mono = std::time::Instant::now();
            let now_wall = std::time::SystemTime::now();
            pf.recorded_at = DualClock {
                mono: now_mono.checked_sub(past_ttl).unwrap_or(now_mono),
                wall: now_wall.checked_sub(past_ttl).unwrap_or(now_wall),
            };
        }
    }
    /// Test-only: simulate a system suspend between recording and reading the cached `permanent_failure`.
    /// The monotonic clock stays fresh while the wall clock is rewound past the TTL.
    /// (A suspend pauses the monotonic clock, so on wake `mono` reads short while `wall` reads long.)
    #[cfg(test)]
    pub fn force_permanent_failure_wall_aged_out(&self) {
        if let Some(pf) = self.permanent_failure.write().as_mut() {
            let now = std::time::SystemTime::now();
            pf.recorded_at.wall = now
                .checked_sub(PERMANENT_FAILURE_TTL + StdDuration::from_secs(1))
                .unwrap_or(now);
        }
    }
    /// 401 recovery state machine driven by the `rejected` credential.
    /// For one-shot recovery off the live bearer, use `try_recover_unauthorized()`.
    pub fn unauthorized_recovery(
        self: &Arc<Self>,
        rejected: Option<CodelAuth>,
        source: crate::recovery::RecoverySource,
    ) -> crate::recovery::UnauthorizedRecovery {
        crate::recovery::UnauthorizedRecovery::new(self.clone(), rejected, source)
    }
    /// 401 recovery off the live bearer. Snapshots the rejected credential once for KPI attribution. On **transient** refresh failure (network, 5xx, sleep/dark-wake defer, lock timeout) retries with backoff before giving up.
    /// Permanent failures and NotLoggedIn stop immediately. After a successful recovery the **caller** retries the original request. (Turn-level may resubmit more than once; API resubmit is separate from refresh retries.)
    pub async fn try_recover_unauthorized(
        self: &Arc<Self>,
        source: crate::recovery::RecoverySource,
    ) -> bool {
        /// Bounded refresh attempts for non-permanent failures.
        /// Kept strictly below OidcRefresher's consecutive-transient escalation threshold.
        /// One 401 recovery then cannot alone escalate a network blip to permanent `Other`.
        const MAX_TRANSIENT_ATTEMPTS: u32 = 2;
        let cached = self.with_inner_read(|inner| inner.cloned());
        let mut delay = StdDuration::from_millis(500);
        for attempt in 0..MAX_TRANSIENT_ATTEMPTS {
            match self
                .unauthorized_recovery(cached.clone(), source)
                .next()
                .await
            {
                Ok(_) => return true,
                Err(e) if e.is_transient() && attempt + 1 < MAX_TRANSIENT_ATTEMPTS => {
                    codel_logging::unified_log::warn(
                        "auth recovery: transient failure, retrying",
                        None,
                        Some(serde_json::json!({
                            "attempt": attempt + 1,
                            "max_attempts": MAX_TRANSIENT_ATTEMPTS,
                            "delay_ms": delay.as_millis() as u64,
                            "error": format!("{e}"),
                        })),
                    );
                    tokio::time::sleep(delay).await;
                    delay = (delay.saturating_mul(2)).min(StdDuration::from_secs(4));
                }
                Err(_) => return false,
            }
        }
        false
    }
    pub(crate) fn record_manual_auth(
        &self,
        snapshot: &crate::recovery::RejectedAuth,
        err: &AuthError,
        trigger: ManualAuthSurface,
    ) {
        self.manual_auth.record(snapshot, err, trigger);
    }
    #[cfg(test)]
    pub fn manual_auth_last_token(&self) -> Option<String> {
        self.manual_auth.last_token_for_test()
    }
    #[cfg(test)]
    pub fn manual_auth_last_emit(&self) -> Option<codel_logging::events::ManualAuth> {
        self.manual_auth.last_emit_for_test()
    }
}
fn api_key_from_auth_file(path: &Path) -> Option<String> {
    let map = read_auth_json(path).ok()?;
    non_empty_key(map.get(super::model::API_KEY_SCOPE).map(|a| a.key.clone()))
}
/// Memo for [`AuthManager::cached_disk_api_key`]. `stamp == None` means the file is absent.
struct StaticKeyCacheEntry {
    stamp: Option<AuthFileStamp>,
    key: Option<String>,
}
/// (inode, mtime, len).
/// `write_auth_json`'s temp-then-rename allocates a new inode per rewrite, so even a same-length same-mtime rewrite misses the memo.
/// Windows has no stable inode (0 there); its fine mtimes suffice.
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
    /// `codel::api_key` from this manager's auth file, memoized on [`AuthFileStamp`].
    /// Bearer resolution runs per tool call, so this costs a `stat` instead of a read and parse on the hot path.
    pub(crate) fn cached_disk_api_key(&self) -> Option<String> {
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
    /// Set the process model key (empty clears). Not for session tokens.
    pub fn set_process_static_api_key(&self, key: Option<String>) {
        let key = key.map(|k| k.trim().to_string()).filter(|k| !k.is_empty());
        *self.process_static_api_key.write() = key;
    }
    pub(crate) fn process_static_api_key(&self) -> Option<String> {
        non_empty_key(self.process_static_api_key.read().clone())
    }
    /// Static/BYOK key for export paths (e.g. desktop `getBearerToken`).
    /// Never a session JWT; respects kill-switch and preferred-method pin.
    pub fn static_api_key_for_export(&self) -> Option<String> {
        crate::side_call_bearer::resolve_static_api_key(self)
    }
}
/// Compile-time check that `AuthManager` is `Send + Sync`. The proactive refresh task and arbitrary `Arc<AuthManager>` consumers can then safely cross a multi-threaded executor / thread boundary.
/// A future refactor that adds a `!Send` field would otherwise fail to compile in `tokio::spawn(... this.clone() ...)`. The trait-bound error there is confusing and far from the offending field.
const _: fn() = || {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<AuthManager>();
};
#[cfg(test)]
#[path = "manager_tests.rs"]
mod tests;
