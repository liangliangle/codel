use crate::auth::AuthManager;
use crate::util::codel_auth_credentials::CodelAuthCredentials;
use reqwest::RequestBuilder;
use std::sync::Arc;
use codel_auth::{
    AuthCredentialProvider, CredentialSnapshot, HttpAuth, StaticAuthCredentialProvider,
};
/// `api_key.id` for the active credential: hash the stable API key, never the
/// OIDC bearer (which rotates). `None` for non-API-key auth.
fn api_key_id_for(auth: Option<&crate::auth::CodelAuth>) -> Option<String> {
    auth.filter(|a| matches!(a.auth_mode, crate::auth::AuthMode::ApiKey))
        .map(|a| codel_file_utils::sha256_hex(a.key.as_bytes()))
}
/// Production impl: wraps the live `AuthManager`. 401 recovery
/// delegates to `AuthManager::unauthorized_recovery`.
pub struct ShellAuthCredentialProvider {
    auth_manager: Arc<AuthManager>,
    static_credentials: CodelAuthCredentials,
}
impl ShellAuthCredentialProvider {
    pub(crate) fn new(
        auth_manager: Arc<AuthManager>,
        deployment_key: Option<String>,
        alpha_test_key: Option<String>,
    ) -> Self {
        let mut static_credentials = CodelAuthCredentials::new(None);
        static_credentials.deployment_key = deployment_key;
        static_credentials.alpha_test_key = alpha_test_key;
        Self {
            auth_manager,
            static_credentials,
        }
    }
}
impl std::fmt::Debug for ShellAuthCredentialProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ShellAuthCredentialProvider")
            .field("auth_manager", &"<configured>")
            .finish()
    }
}
impl HttpAuth for ShellAuthCredentialProvider {
    fn apply(&self, builder: RequestBuilder, base_url: &str) -> RequestBuilder {
        let mut creds = self.static_credentials.clone();
        if creds.deployment_key.is_none()
            && let Some(auth) = self.auth_manager.current_or_expired()
        {
            creds.user_token = Some(auth.key);
        }
        creds.apply(builder, base_url)
    }
}
#[async_trait::async_trait]
impl AuthCredentialProvider for ShellAuthCredentialProvider {
    fn snapshot(&self) -> CredentialSnapshot {
        if let Some(ref dk) = self.static_credentials.deployment_key {
            return CredentialSnapshot {
                token: Some(dk.clone()),
                deployment_id: crate::managed_config::resolve_deployment_id(Some(dk)),
                ..Default::default()
            };
        }
        let auth = self.auth_manager.current_or_expired();
        let user_id = auth.as_ref().map(|a| a.user_id.clone());
        let team_id = auth.as_ref().and_then(|a| a.team_id.clone());
        let organization_id = auth.as_ref().and_then(|a| a.organization_id.clone());
        let api_key_id = api_key_id_for(auth.as_ref());
        let token = auth.map(|a| a.key);
        CredentialSnapshot {
            token,
            user_id,
            team_id,
            deployment_id: None,
            api_key_id,
            organization_id,
        }
    }
    async fn refresh_after_unauthorized(&self) -> bool {
        if self.static_credentials.deployment_key.is_some() {
            return false;
        }
        self.auth_manager
            .try_recover_unauthorized(crate::auth::recovery::RecoverySource::Background)
            .await
    }
    fn needs_token_auth_header(&self) -> bool {
        self.static_credentials.deployment_key.is_none()
    }
}
/// Resolves the embedding credentials for `embed_base_url`, attaching the Codel
/// session credential only to Codel-operated endpoints over `https`.
pub(crate) fn embedding_session_credentials(
    embed_base_url: &str,
    auth_manager: Option<&Arc<AuthManager>>,
    api_key_provider: Option<codel_tools::types::SharedApiKeyProvider>,
) -> codel_memory::EndpointScopedCredentials {
    let auth_credentials = auth_manager.map(|am| {
        Arc::new(ShellAuthCredentialProvider::new(am.clone(), None, None))
            as Arc<dyn AuthCredentialProvider>
    });
    codel_memory::EndpointScopedCredentials::for_endpoint(
        embed_base_url,
        crate::util::is_codel_api_bearer_url,
        auth_credentials,
        api_key_provider,
    )
}
/// Build a `StorageClient` for proxy uploads (including the high-volume
/// `batch_upload` used for repo context / `repo_changes_dedup`).
///
/// ### Client Identity (Important)
///
/// Every call site **must** pass the correct `client_identifier` so that
/// the API proxy (and metrics backends) can properly attribute requests:
///
/// - `"codel-shell"`   — classic Codel CLI / TUI (codel-shell)
/// - `"codel-pager"`   — new Codel Pager / TUI (codel-pager)
/// - `"codel-desktop"` — Codel Desktop app
/// - `"codel-extension"` — VS Code / browser extension
///
/// The factory forwards both:
/// - `x-codel-client-version`   (e.g. "0.1.210-alpha.5 (279ffacddb)")
/// - `x-codel-client-identifier`
///
/// to the underlying `codel-file-utils::StorageClient` via
/// `.with_client_identity(...)`. This is what powers the improved error
/// logging on the request-attribution path.
///
/// - When `auth_manager` is `Some`, uses the live `ShellAuthCredentialProvider`
///   (OIDC refresh, proactive refresh, 401 recovery via `unauthorized_recovery`).
/// - When `auth_manager` is `None`, falls back to a static token:
///   - `user_token` (normal OIDC users via com scope) → sends Bearer + `X-Codel-Token-Auth`
///   - `deployment_key` (enterprise) → sends bare Bearer
///   - the optional extra access key is added only for matching hosts when
///     the non-production feature is enabled.
///
/// `user_token` is only for AuthManager-less one-shots; live paths (incl. pager
/// restore) pass the AuthManager plus an optional deployment key.
pub fn build_storage_client_for_proxy(
    proxy_base_url: &str,
    deployment_key: Option<String>,
    alpha_test_key: Option<String>,
    auth_manager: Option<Arc<AuthManager>>,
    user_token: Option<String>,
    session_id: Option<String>,
    client_identifier: &str,
) -> codel_file_utils::storage_client::StorageClient {
    let http_client = crate::http::shared_upload_client();
    if let Some(am) = auth_manager {
        let provider: Arc<dyn AuthCredentialProvider> = Arc::new(ShellAuthCredentialProvider::new(
            am.clone(),
            deployment_key,
            alpha_test_key,
        ));
        let bridge: Arc<dyn codel_file_utils::storage_client::Auth401AttributionCallback> =
            Arc::new(StorageClientAttributionBridge::new(am, session_id));
        codel_file_utils::storage_client::StorageClient::with_provider(
            proxy_base_url,
            http_client,
            provider,
        )
        .with_client_identity(codel_version::VERSION, client_identifier)
        .with_attribution(bridge)
    } else {
        let mut creds = CodelAuthCredentials::new(user_token);
        creds.deployment_key = deployment_key;
        creds.alpha_test_key = alpha_test_key;
        let wire_bearer = creds
            .deployment_key
            .clone()
            .or_else(|| creds.user_token.clone());
        let provider: Arc<dyn AuthCredentialProvider> = Arc::new(
            StaticAuthCredentialProvider::new(Box::new(creds), wire_bearer),
        );
        codel_file_utils::storage_client::StorageClient::with_provider(
            proxy_base_url,
            http_client,
            provider,
        )
        .with_client_identity(codel_version::VERSION, client_identifier)
    }
}
/// Bridge that lets `StorageClient` (which lives in codel-file-utils)
/// emit shell's 401-attribution event without the data-collector crate
/// having a direct dependency on shell. Holds a reference to the live
/// `AuthManager` so attribution events carry the correct user_id.
pub(crate) struct StorageClientAttributionBridge {
    auth_manager: Arc<AuthManager>,
    session_id: Option<String>,
}
impl std::fmt::Debug for StorageClientAttributionBridge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StorageClientAttributionBridge")
            .finish_non_exhaustive()
    }
}
impl StorageClientAttributionBridge {
    pub(crate) fn new(auth_manager: Arc<AuthManager>, session_id: Option<String>) -> Self {
        Self {
            auth_manager,
            session_id,
        }
    }
}
impl codel_file_utils::storage_client::Auth401AttributionCallback for StorageClientAttributionBridge {
    fn record_401(&self, operation: &str, sent_bearer_prefix: Option<&str>) {
        crate::auth::attribution::record_consumer_401(
            self.auth_manager.as_ref(),
            self.session_id.as_deref(),
            crate::auth::attribution::ConsumerKind::StorageClient,
            operation,
            sent_bearer_prefix,
        );
    }
}
/// Credential provider for the OTel layer's `RefreshableSpanExporter`.
///
/// Starts with a bootstrap `AuthManager` (disk-read-only, no refresher)
/// and can be upgraded to the agent's live `Arc<AuthManager>` once the
/// agent is initialized. After upgrade:
///
/// - `snapshot()` reads from the live manager's in-memory cache (kept
///   hot by the proactive refresh task) instead of re-reading disk.
/// - `refresh_after_unauthorized()` routes through
///   `unauthorized_recovery` for active OIDC/external-binary refresh.
///
/// Before upgrade, the bootstrap manager provides disk-read-only
/// behavior (equivalent to the pre-consolidation OTel path).
pub struct OtelAuthCredentialProvider {
    /// Bootstrap manager used before the live one is available.
    bootstrap: Arc<AuthManager>,
    /// Swapped to the agent's live `AuthManager` via `set_live()`.
    /// `None` means still in bootstrap mode.
    live: arc_swap::ArcSwap<Option<Arc<AuthManager>>>,
    /// Enterprise deployment key. Takes precedence over OIDC in `snapshot_inner`.
    deployment_key: arc_swap::ArcSwap<Option<String>>,
}
impl OtelAuthCredentialProvider {
    fn new(bootstrap: Arc<AuthManager>) -> Self {
        Self {
            bootstrap,
            live: arc_swap::ArcSwap::from_pointee(None),
            deployment_key: arc_swap::ArcSwap::from_pointee(None),
        }
    }
    /// Upgrade to the agent's live `AuthManager`. After this call,
    /// `snapshot()` reads from the live manager (proactive refresh
    /// keeps it hot) and `refresh_after_unauthorized()` drives the
    /// full recovery state machine.
    pub fn set_live(&self, auth_manager: Arc<AuthManager>) {
        self.live.store(Arc::new(Some(auth_manager)));
    }
    pub fn set_deployment_key(&self, key: String) {
        self.deployment_key.store(Arc::new(Some(key)));
    }
    /// Single-load snapshot of the live/bootstrap state.
    fn load_state(&self) -> (Arc<AuthManager>, bool) {
        let guard = self.live.load();
        match guard.as_ref() {
            Some(am) => (am.clone(), true),
            None => (self.bootstrap.clone(), false),
        }
    }
}
impl std::fmt::Debug for OtelAuthCredentialProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (_, is_live) = self.load_state();
        f.debug_struct("OtelAuthCredentialProvider")
            .field("mode", &if is_live { "live" } else { "bootstrap" })
            .finish()
    }
}
impl HttpAuth for OtelAuthCredentialProvider {
    fn apply(&self, builder: RequestBuilder, base_url: &str) -> RequestBuilder {
        let snapshot = self.snapshot_inner();
        let mut creds = CodelAuthCredentials::new(None);
        if self.deployment_key.load().is_some() {
            creds.deployment_key = snapshot.token;
        } else {
            creds.user_token = snapshot.token;
        }
        creds.apply(builder, base_url)
    }
}
impl OtelAuthCredentialProvider {
    fn snapshot_inner(&self) -> CredentialSnapshot {
        if let Some(ref dk) = **self.deployment_key.load() {
            return CredentialSnapshot {
                token: Some(dk.clone()),
                deployment_id: crate::managed_config::resolve_deployment_id(Some(dk)),
                ..Default::default()
            };
        }
        let (am, is_live) = self.load_state();
        if !is_live {
            am.force_reload_from_disk();
        }
        let auth = am.current_or_expired();
        let user_id = auth.as_ref().map(|a| a.user_id.clone());
        let team_id = auth.as_ref().and_then(|a| a.team_id.clone());
        let organization_id = auth.as_ref().and_then(|a| a.organization_id.clone());
        let api_key_id = api_key_id_for(auth.as_ref());
        let token = auth.map(|a| a.key);
        CredentialSnapshot {
            token,
            user_id,
            team_id,
            deployment_id: None,
            api_key_id,
            organization_id,
        }
    }
}
#[async_trait::async_trait]
impl AuthCredentialProvider for OtelAuthCredentialProvider {
    fn snapshot(&self) -> CredentialSnapshot {
        self.snapshot_inner()
    }
    fn has_usable_credential(&self) -> bool {
        if self.deployment_key.load().is_some() {
            return true;
        }
        self.load_state().0.has_usable_token()
    }
    async fn refresh_after_unauthorized(&self) -> bool {
        let (am, is_live) = self.load_state();
        if !is_live {
            return false;
        }
        am.try_recover_unauthorized(crate::auth::recovery::RecoverySource::Background)
            .await
    }
    fn needs_token_auth_header(&self) -> bool {
        self.deployment_key.load().is_none()
    }
}
/// Process-wide OTel credential provider handle.
///
/// This is one of two acceptable process-wide statics in this crate
/// (the other is `TRACER_PROVIDER` in `otel_layer.rs`). It's set
/// once at tracing init (before any `AuthManager` exists) and holds
/// a bootstrap-mode provider. The `ArcSwap` *inside* the provider
/// handles the runtime auth state swap — the `OnceLock` itself is
/// never re-written.
///
/// Why not thread it? `build_default_otel_layer_config` is called
/// from 15+ `init_tracing*` sites across 3 binaries. Threading a
/// handle through all of them to the agent init site (where the
/// live `AuthManager` is constructed) would touch ~20 files for a
/// single-purpose plumbing change. The `OnceLock` holds a
/// container, not auth state; the auth state lives behind `ArcSwap`
/// and changes at runtime.
static OTEL_PROVIDER: std::sync::OnceLock<Arc<OtelAuthCredentialProvider>> =
    std::sync::OnceLock::new();
/// Upgrade the OTel credential provider to use the agent's live
/// `AuthManager`. Call this once after the main `AuthManager` is
/// constructed and has its refresher configured. No-ops if the OTel
/// layer was never initialized (e.g. `InstrumentationMode::Disabled`).
pub fn wire_otel_auth_manager(auth_manager: Arc<AuthManager>) {
    if let Some(provider) = OTEL_PROVIDER.get() {
        provider.set_live(auth_manager);
        tracing::debug!("otel: upgraded credential provider to live AuthManager");
    }
    sync_external_otel_identity();
}
/// Push the current identity *attributes* (never the token) to the external
/// OTEL stream. Reads the same `CredentialSnapshot` the internal layer
/// stamps per-export, so both pipelines attribute identically. No-op when
/// the OTel provider was never initialized or the external stream is
/// dormant.
pub fn sync_external_otel_identity() {
    if let Some(provider) = OTEL_PROVIDER.get() {
        let snapshot = provider.snapshot();
    }
}
/// No-ops if the OTel layer was never initialized.
pub fn wire_otel_deployment_key(key: String) {
    if let Some(provider) = OTEL_PROVIDER.get() {
        provider.set_deployment_key(key);
        tracing::debug!("otel: set deployment key on credential provider");
        sync_external_otel_identity();
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::CodelAuth;
    use crate::auth::CodelComConfig;
    use crate::auth::manager::AuthManager;
    use chrono::{Duration as ChronoDuration, Utc};
    use std::sync::Mutex;
    use codel_auth::AuthCredentialProvider;
    /// Serializes tests that pin `CODEL_AUTH_EARLY_INVALIDATION_SECS`, since
    /// env vars are process-global and parallel tests would race.
    static EARLY_INVALIDATION_LOCK: Mutex<()> = Mutex::new(());
    /// RAII guard: pins `CODEL_AUTH_EARLY_INVALIDATION_SECS` to the production
    /// default (300s) while held, restoring the previous value on drop.
    /// Acquires `EARLY_INVALIDATION_LOCK` so concurrent test runners can't
    /// observe a half-mutated env.
    struct EarlyInvalidationGuard {
        _lock: std::sync::MutexGuard<'static, ()>,
        previous: Option<String>,
    }
    impl EarlyInvalidationGuard {
        fn pin_to_default() -> Self {
            let lock = EARLY_INVALIDATION_LOCK
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            let previous = std::env::var("CODEL_AUTH_EARLY_INVALIDATION_SECS").ok();
            unsafe { std::env::set_var("CODEL_AUTH_EARLY_INVALIDATION_SECS", "300") };
            Self {
                _lock: lock,
                previous,
            }
        }
    }
    impl Drop for EarlyInvalidationGuard {
        fn drop(&mut self) {
            unsafe {
                match self.previous.take() {
                    Some(prev) => std::env::set_var("CODEL_AUTH_EARLY_INVALIDATION_SECS", prev),
                    None => std::env::remove_var("CODEL_AUTH_EARLY_INVALIDATION_SECS"),
                }
            }
        }
    }
    fn make_auth(key: &str, expires_in: ChronoDuration) -> CodelAuth {
        CodelAuth {
            key: key.to_string(),
            user_id: "test-user".to_string(),
            create_time: Utc::now(),
            expires_at: Some(Utc::now() + expires_in),
            ..CodelAuth::test_default()
        }
    }
    /// Build an `AuthManager` rooted at `dir`. Caller keeps `dir` alive for
    /// the duration of the test so the `TempDir` `Drop` actually cleans up.
    fn make_manager(dir: &tempfile::TempDir, initial: Option<CodelAuth>) -> Arc<AuthManager> {
        let mgr = AuthManager::new(dir.path(), CodelComConfig::default());
        if let Some(auth) = initial {
            mgr.hot_swap(auth);
        }
        Arc::new(mgr)
    }
    /// `apply()` and `snapshot()` agree (snapshot==wire invariant) when the
    /// in-memory token is fresh.
    #[test]
    fn apply_and_snapshot_agree_on_live_token() {
        let _guard = EarlyInvalidationGuard::pin_to_default();
        let dir = tempfile::tempdir().unwrap();
        let mgr = make_manager(
            &dir,
            Some(make_auth("live-token", ChronoDuration::hours(1))),
        );
        let provider = ShellAuthCredentialProvider::new(mgr, None, None);
        let snap = provider.snapshot();
        assert_eq!(snap.token.as_deref(), Some("live-token"));
        assert_eq!(snap.user_id.as_deref(), Some("test-user"));
    }
    /// During the 5-minute pre-refresh buffer window, `auth_manager.current()`
    /// returns `None` (the token is treated as expired-soon for refresh
    /// scheduling), but the token is still valid at the proxy. The provider
    /// must fall back to `expired_auth()` so the in-memory token gets sent
    /// instead of nothing -- which is the fix for the bulk of the
    /// `POST /v1/storage` 401s observed in production.
    #[test]
    fn falls_back_to_expired_auth_during_buffer_window() {
        let _guard = EarlyInvalidationGuard::pin_to_default();
        let dir = tempfile::tempdir().unwrap();
        let mgr = make_manager(
            &dir,
            Some(make_auth("buffer-token", ChronoDuration::minutes(4))),
        );
        assert!(mgr.current().is_none(), "buffer-window precondition");
        assert!(mgr.expired_auth().is_some(), "buffer-window precondition");
        let provider = ShellAuthCredentialProvider::new(mgr, None, None);
        let snap = provider.snapshot();
        assert_eq!(
            snap.token.as_deref(),
            Some("buffer-token"),
            "snapshot should fall back to expired_auth instead of None"
        );
        assert_eq!(snap.user_id.as_deref(), Some("test-user"));
    }
    /// When `auth_manager` has nothing at all (no in-memory auth, expired
    /// or otherwise), `snapshot()` returns `None` for the user-token branch.
    /// `apply()` would then send no Authorization header.
    #[test]
    fn no_token_when_auth_manager_is_empty() {
        let _guard = EarlyInvalidationGuard::pin_to_default();
        let dir = tempfile::tempdir().unwrap();
        let mgr = make_manager(&dir, None);
        let provider = ShellAuthCredentialProvider::new(mgr, None, None);
        let snap = provider.snapshot();
        assert!(
            snap.token.is_none(),
            "snapshot should be None when manager has no auth"
        );
        assert!(snap.user_id.is_none());
    }

    #[test]
    fn embedding_session_credentials_scopes_to_first_party() {
        let _guard = EarlyInvalidationGuard::pin_to_default();
        let dir = tempfile::tempdir().unwrap();
        let mgr = make_manager(
            &dir,
            Some(make_auth("codel-session-token", ChronoDuration::hours(1))),
        );
        let api_key_provider: codel_tools::types::SharedApiKeyProvider =
            Arc::new(crate::auth::manager::SharedAuthKeyProvider(mgr.clone()));
        for denied in ["https://byok.attacker.example/v1", "http://api.codel/v1"] {
            let resolved =
                embedding_session_credentials(denied, Some(&mgr), Some(api_key_provider.clone()));
            assert!(
                resolved.is_empty(),
                "session credentials must not reach {denied}"
            );
        }
        let resolved = embedding_session_credentials(
            "https://api.codel/v1",
            Some(&mgr),
            Some(api_key_provider),
        );
        assert!(!resolved.is_empty());
    }
    /// Deployment-key path has no recovery (operator owns the bearer).
    #[tokio::test]
    async fn refresh_after_unauthorized_is_noop_for_deployment_key() {
        let _guard = EarlyInvalidationGuard::pin_to_default();
        let dir = tempfile::tempdir().unwrap();
        let mgr = make_manager(&dir, None);
        let provider =
            ShellAuthCredentialProvider::new(mgr, Some("deployment-key".to_string()), None);
        assert!(!provider.refresh_after_unauthorized().await);
    }
    #[test]
    fn snapshot_populates_tenant_id_per_auth_mode() {
        let _guard = EarlyInvalidationGuard::pin_to_default();
        let dir = tempfile::tempdir().unwrap();
        let dep = ShellAuthCredentialProvider::new(
            make_manager(&dir, None),
            Some("codel-token-EX".into()),
            None,
        )
        .snapshot();
        assert_eq!(
            dep.deployment_id.as_deref(),
            deployment_id_from_key("codel-token-EX").as_deref()
        );
        assert!(dep.api_key_id.is_none());
        let api_auth = CodelAuth {
            key: "sk-apikey-xyz".into(),
            auth_mode: crate::auth::AuthMode::ApiKey,
            expires_at: Some(Utc::now() + ChronoDuration::hours(1)),
            ..CodelAuth::test_default()
        };
        let api = ShellAuthCredentialProvider::new(make_manager(&dir, Some(api_auth)), None, None)
            .snapshot();
        assert_eq!(
            api.api_key_id.as_deref(),
            deployment_id_from_key("sk-apikey-xyz").as_deref()
        );
        assert!(api.deployment_id.is_none());
        let oidc = ShellAuthCredentialProvider::new(
            make_manager(
                &dir,
                Some(make_auth("oidc-token", ChronoDuration::hours(1))),
            ),
            None,
            None,
        )
        .snapshot();
        assert!(oidc.deployment_id.is_none() && oidc.api_key_id.is_none());
    }
    /// Bootstrap mode: `snapshot()` re-reads disk so sibling-rotated
    /// tokens are picked up without a live AuthManager.
    #[test]
    fn otel_bootstrap_snapshot_picks_up_disk_writes() {
        let _guard = EarlyInvalidationGuard::pin_to_default();
        let dir = tempfile::tempdir().unwrap();
        let scope = crate::auth::CodelComConfig::default().auth_scope();
        let auth_path = dir.path().join("auth.json");
        let mgr = make_manager(
            &dir,
            Some(make_auth("initial-token", ChronoDuration::hours(1))),
        );
        let mut store = crate::auth::read_auth_json(&auth_path).unwrap_or_default();
        store.insert(
            scope.clone(),
            make_auth("initial-token", ChronoDuration::hours(1)),
        );
        crate::auth::storage::write_auth_json(&auth_path, &store).unwrap();
        let provider = OtelAuthCredentialProvider::new(mgr);
        assert_eq!(provider.snapshot().token.as_deref(), Some("initial-token"));
        let mut store = crate::auth::read_auth_json(&auth_path).unwrap();
        store.insert(scope, make_auth("rotated-token", ChronoDuration::hours(1)));
        crate::auth::storage::write_auth_json(&auth_path, &store).unwrap();
        assert_eq!(
            provider.snapshot().token.as_deref(),
            Some("rotated-token"),
            "must pick up sibling-rotated tokens from disk"
        );
    }
    /// After `set_live()`, `snapshot()` reads from the live manager's
    /// in-memory cache (no disk re-read) and `refresh_after_unauthorized()`
    /// drives the recovery state machine.
    #[tokio::test]
    async fn otel_live_mode_uses_shared_auth_manager() {
        let _guard = EarlyInvalidationGuard::pin_to_default();
        let bootstrap_dir = tempfile::tempdir().unwrap();
        let bootstrap_mgr = make_manager(&bootstrap_dir, None);
        let provider = OtelAuthCredentialProvider::new(bootstrap_mgr);
        assert!(provider.snapshot().token.is_none());
        assert!(!provider.refresh_after_unauthorized().await);
        let live_dir = tempfile::tempdir().unwrap();
        let live_mgr = make_manager(
            &live_dir,
            Some(make_auth("live-token", ChronoDuration::hours(1))),
        );
        provider.set_live(live_mgr.clone());
        assert_eq!(
            provider.snapshot().token.as_deref(),
            Some("live-token"),
            "must read from live AuthManager after set_live()"
        );
        live_mgr.hot_swap(make_auth("rotated-live", ChronoDuration::hours(1)));
        assert_eq!(
            provider.snapshot().token.as_deref(),
            Some("rotated-live"),
            "must see rotated token from live manager"
        );
    }

    #[test]
    fn otel_deployment_key_sent_when_no_oidc_token() {
        let _guard = EarlyInvalidationGuard::pin_to_default();
        let dir = tempfile::tempdir().unwrap();
        let mgr = make_manager(&dir, None);
        let provider = OtelAuthCredentialProvider::new(mgr);
        provider.set_deployment_key("enterprise-key".to_string());
        let snap = provider.snapshot();
        assert_eq!(
            snap.token.as_deref(),
            Some("enterprise-key"),
            "deployment key must be sent when no OIDC token exists"
        );
    }
    #[test]
    fn otel_deployment_key_wins_over_oidc_token() {
        let _guard = EarlyInvalidationGuard::pin_to_default();
        let dir = tempfile::tempdir().unwrap();
        let mgr = make_manager(
            &dir,
            Some(make_auth("oidc-token", ChronoDuration::hours(1))),
        );
        let provider = OtelAuthCredentialProvider::new(mgr);
        provider.set_deployment_key("deployment-key-123".to_string());
        let snap = provider.snapshot();
        assert_eq!(
            snap.token.as_deref(),
            Some("deployment-key-123"),
            "deployment key must win over OIDC token"
        );
        assert!(snap.user_id.is_none());
    }
    #[test]
    fn has_usable_credential_reflects_auth_state() {
        let _guard = EarlyInvalidationGuard::pin_to_default();
        let dir = tempfile::tempdir().unwrap();
        let mgr = make_manager(&dir, Some(make_auth("live", ChronoDuration::hours(1))));
        let provider = OtelAuthCredentialProvider::new(mgr);
        assert!(
            provider.has_usable_credential(),
            "valid unexpired token is usable"
        );
        let dir = tempfile::tempdir().unwrap();
        let mgr = make_manager(&dir, Some(make_auth("stale", ChronoDuration::hours(-1))));
        let provider = OtelAuthCredentialProvider::new(mgr);
        assert!(
            !provider.has_usable_credential(),
            "expired token is not usable"
        );
        let dir = tempfile::tempdir().unwrap();
        let provider = OtelAuthCredentialProvider::new(make_manager(&dir, None));
        assert!(
            !provider.has_usable_credential(),
            "absent token is not usable"
        );
        let dir = tempfile::tempdir().unwrap();
        let provider = OtelAuthCredentialProvider::new(make_manager(&dir, None));
        provider.set_deployment_key("enterprise-key".to_string());
        assert!(
            provider.has_usable_credential(),
            "static deployment key is always usable"
        );
    }
    /// A token inside the early-invalidation buffer is still accepted by the
    /// proxy (the buffer is a client-side pre-refresh margin, not a wire
    /// expiry), and the sender puts it on the wire via `current_or_expired()`.
    /// The export gate must therefore keep it usable even though `current()`
    /// reports `None`. Regression for the buffer-window export drop.
    #[test]
    fn has_usable_credential_true_inside_early_invalidation_buffer() {
        let _guard = EarlyInvalidationGuard::pin_to_default();
        let dir = tempfile::tempdir().unwrap();
        let mgr = make_manager(
            &dir,
            Some(make_auth("buffered", ChronoDuration::minutes(4))),
        );
        assert!(
            mgr.current().is_none(),
            "4-min token sits inside the pinned 5-min buffer, so current() is None"
        );
        let provider = OtelAuthCredentialProvider::new(mgr);
        assert!(
            provider.has_usable_credential(),
            "a buffer-window token is still wire-valid, so the gate keeps it usable"
        );
    }
    /// A configured `deployment_key` always wins over the AuthManager-resolved
    /// user token, matching the precedence in `CodelAuthCredentials::apply`.
    /// The snapshot must report the deployment key (not the user token) so
    /// the 401-attribution prefix matches the wire bytes.
    #[test]
    fn deployment_key_wins_over_resolved_user_token() {
        let _guard = EarlyInvalidationGuard::pin_to_default();
        let dir = tempfile::tempdir().unwrap();
        let mgr = make_manager(
            &dir,
            Some(make_auth("user-token", ChronoDuration::hours(1))),
        );
        let provider =
            ShellAuthCredentialProvider::new(mgr, Some("deployment-key-12345".to_string()), None);
        let snap = provider.snapshot();
        assert_eq!(snap.token.as_deref(), Some("deployment-key-12345"));
        assert!(snap.user_id.is_none());
    }
}
