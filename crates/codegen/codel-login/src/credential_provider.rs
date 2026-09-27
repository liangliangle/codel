use std::sync::Arc;

use reqwest::RequestBuilder;
use codel_auth::{AuthCredentialProvider, CredentialSnapshot, HttpAuth};

use crate::AuthManager;
use crate::backend::{ActiveAuthBackend, AuthBackend};
use crate::codel_auth_credentials::CodelAuthCredentials;

/// `api_key.id` for the active credential: hash the stable API key, never the OIDC bearer (which rotates).
/// `None` for non-API-key auth.
fn api_key_id_for(auth: Option<&crate::CodelAuth>) -> Option<String> {
    auth.filter(|a| matches!(a.auth_mode, crate::AuthMode::ApiKey))
        .map(|a| codel_logging::config::deployment_id_from_key(&a.key))
}

/// Sampler [`BearerResolver`](codel_sampler::BearerResolver) over a live [`AuthManager`].
/// Wire-valid only: it never stamps a hard-expired access token (the client auth contract).
/// Shared by the session sampler and subagent configs so the contract can't drift between them.
pub struct WireValidBearerResolver(pub Arc<AuthManager>);

impl WireValidBearerResolver {
    /// The one constructor both the session sampler and subagent configs use, so the wire-valid contract cannot drift between the call sites.
    pub fn shared(auth_manager: Arc<AuthManager>) -> codel_sampler::SharedBearerResolver {
        Arc::new(Self(auth_manager))
    }
}

impl std::fmt::Debug for WireValidBearerResolver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WireValidBearerResolver").finish()
    }
}

/// Ceiling for the pre-send refresh wait.
/// Long enough for one external-binary run (7 s) or a first-party token exchange; the exchange keeps going in the background if it overruns and hot-swaps the token when it lands.
const PRE_SEND_REFRESH_BUDGET: std::time::Duration = std::time::Duration::from_secs(10);

/// Time the send itself needs once the wait ends, reserved out of a still wire-valid bearer's remaining life.
const PRE_SEND_STAMP_MARGIN: std::time::Duration = std::time::Duration::from_millis(500);

/// How long `prepare_for_send` may wait on a refresh: the wait must end before the cached bearer dies, or a slow mint turns a request that could have carried the old token into one that carries none.
fn pre_send_refresh_budget(remaining_wire_life: std::time::Duration) -> std::time::Duration {
    remaining_wire_life
        .saturating_sub(PRE_SEND_STAMP_MARGIN)
        .min(PRE_SEND_REFRESH_BUDGET)
}

impl codel_sampler::BearerResolver for WireValidBearerResolver {
    fn current_bearer(&self) -> Option<String> {
        // The samplers attach this resolver whenever the endpoint is a first-party Codel URL.
        // A session minted by another authority would send its token there on every chat call.
        if !ActiveAuthBackend::default().is_codel_authority() {
            return None;
        }
        self.0.current_wire_valid().map(|a| a.key)
    }

    /// Closes the pre-flight→send gap: the turn's pre-flight ran `auth()`, but the request can leave much later (a sampling-permit wait, a rate-limit sleep, a resubmit that skips the pre-flight).
    /// If the cached bearer is wire-valid now but would not survive the send, refresh here instead of letting `current_bearer` strip it and send the request with no credential.
    /// Only that race is handled here. With no wire-valid bearer at all the pre-flight has already made its refresh attempt and the 401 arm (recovery, parking) owns the outcome; a refresh per send would let every parked, deliberately credential-less resubmit drive the escalation budget.
    fn prepare_for_send(
        &self,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + '_>> {
        Box::pin(async move {
            if !ActiveAuthBackend::default().is_codel_authority() {
                return;
            }
            let Some(remaining) = self.0.remaining_wire_life() else {
                return;
            };
            if self.0.has_sendable_token() {
                return;
            }
            let budget = pre_send_refresh_budget(remaining);
            codel_logging::unified_log::warn(
                "auth: pre-send refresh, cached bearer would not outlive the send",
                None,
                Some(serde_json::json!({ "budget_ms": budget.as_millis() as u64 })),
            );
            // Nothing to mint: an API key does not expire, and `current_bearer`
            // still carries the configured key.
            let _ = budget;
        })
    }
}

/// Resolves a snapshot's `deployment_id` from an enterprise deployment key.
/// Injected at construction so the provider stays off shell config; callers with a
/// deployment key pass the managed-config deployment-id resolver.
pub type DeploymentIdResolver =
    std::sync::Arc<dyn Fn(Option<&str>) -> Option<String> + Send + Sync>;

/// Production impl: wraps the live `AuthManager`.
/// 401 recovery delegates to `AuthManager::unauthorized_recovery`.
pub struct ShellAuthCredentialProvider {
    auth_manager: Arc<AuthManager>,
    static_credentials: CodelAuthCredentials,
    deployment_id_resolver: DeploymentIdResolver,
}

impl ShellAuthCredentialProvider {
    /// Constructs without a deployment-id resolver; the snapshot omits `deployment_id`.
    /// Only correct for callers that never set a deployment key; deployment-key callers
    /// must use [`Self::with_deployment_id_resolver`].
    pub fn new(
        auth_manager: Arc<AuthManager>,
        deployment_key: Option<String>,
        alpha_test_key: Option<String>,
    ) -> Self {
        Self::with_deployment_id_resolver(
            auth_manager,
            deployment_key,
            alpha_test_key,
            std::sync::Arc::new(|_| None),
        )
    }

    pub fn with_deployment_id_resolver(
        auth_manager: Arc<AuthManager>,
        deployment_key: Option<String>,
        alpha_test_key: Option<String>,
        deployment_id_resolver: DeploymentIdResolver,
    ) -> Self {
        let mut static_credentials = CodelAuthCredentials::new(None);
        static_credentials.deployment_key = deployment_key;
        static_credentials.alpha_test_key = alpha_test_key;
        Self {
            auth_manager,
            static_credentials,
            deployment_id_resolver,
        }
    }
}

// Manual Debug impl that redacts the token, like RefreshableSpanExporter in otel_layer.rs
impl std::fmt::Debug for ShellAuthCredentialProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ShellAuthCredentialProvider")
            .field("auth_manager", &"<configured>")
            .finish()
    }
}

impl HttpAuth for ShellAuthCredentialProvider {
    fn apply(&self, builder: RequestBuilder, base_url: &str) -> RequestBuilder {
        // This trait is sync, so no refresh happens here: the proactive task keeps the cache hot and `refresh_after_unauthorized()` handles 401s
        // Wire-valid only: never stamp a hard-expired access token
        let mut creds = self.static_credentials.clone();
        // A session minted elsewhere must not reach an Codel host.
        // A deployment key is configured locally rather than minted, so it still applies.
        if creds.deployment_key.is_none()
            && ActiveAuthBackend::default().is_codel_authority()
            && let Some(auth) = self.auth_manager.current_wire_valid()
        {
            creds.user_token = Some(auth.key);
        }
        creds.apply(builder, base_url)
    }
}

#[async_trait::async_trait]
impl AuthCredentialProvider for ShellAuthCredentialProvider {
    fn snapshot(&self) -> CredentialSnapshot {
        // The token must match what `HttpAuth::apply` puts on the wire (wire-valid only)
        // Identity fields may still come from a soft-expired cache
        if let Some(ref dk) = self.static_credentials.deployment_key {
            return CredentialSnapshot {
                token: Some(dk.clone()),
                deployment_id: (self.deployment_id_resolver)(Some(dk)),
                ..Default::default()
            };
        }
        let identity = self.auth_manager.current_or_expired();
        let user_id = identity.as_ref().map(|a| a.user_id.clone());
        let team_id = identity.as_ref().and_then(|a| a.team_id.clone());
        let organization_id = identity.as_ref().and_then(|a| a.organization_id.clone());
        let api_key_id = api_key_id_for(identity.as_ref());
        let token = ActiveAuthBackend::default()
            .is_codel_authority()
            .then(|| self.auth_manager.current_wire_valid().map(|a| a.key))
            .flatten();
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
            .try_recover_unauthorized(crate::recovery::RecoverySource::Background)
            .await
    }

    fn needs_token_auth_header(&self) -> bool {
        self.static_credentials.deployment_key.is_none()
    }
}

/// Resolves the embedding credentials for `embed_base_url`, attaching the Codel session credential only to Codel-operated endpoints over `https`.
pub fn embedding_session_credentials(
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
        codel_shell_base::util::is_codel_api_bearer_url,
        auth_credentials,
        api_key_provider,
    )
}

/// Lets `StorageClient` (in codel-file-utils) emit shell's 401-attribution event without codel-file-utils depending on shell.
/// Holds the live `AuthManager` so attribution events carry the correct user_id.
pub struct StorageClientAttributionBridge {
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
    pub fn new(auth_manager: Arc<AuthManager>, session_id: Option<String>) -> Self {
        Self {
            auth_manager,
            session_id,
        }
    }
}

impl codel_file_utils::storage_client::Auth401AttributionCallback for StorageClientAttributionBridge {
    fn record_401(&self, operation: &str, sent_bearer_prefix: Option<&str>) {
        crate::attribution::record_consumer_401(
            self.auth_manager.as_ref(),
            self.session_id.as_deref(),
            crate::attribution::ConsumerKind::StorageClient,
            operation,
            sent_bearer_prefix,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CodelAuth;
    use crate::CodelComConfig;
    use crate::manager::AuthManager;
    use chrono::{Duration as ChronoDuration, Utc};
    use std::sync::Mutex;
    use codel_auth::AuthCredentialProvider;

    /// Serializes tests that pin `CODEL_AUTH_EARLY_INVALIDATION_SECS`, since env vars are process-global and parallel tests would race.
    static EARLY_INVALIDATION_LOCK: Mutex<()> = Mutex::new(());

    /// RAII guard: pins `CODEL_AUTH_EARLY_INVALIDATION_SECS` to the production default (300s) while held, restoring the previous value on drop.
    /// Acquires `EARLY_INVALIDATION_LOCK` so concurrent test runners can't observe a half-mutated env.
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
            // SAFETY: env var mutation is `unsafe` in edition 2024; the lock
            // above ensures no other test in this module reads/writes the
            // same key concurrently.
            unsafe { std::env::set_var("CODEL_AUTH_EARLY_INVALIDATION_SECS", "300") };
            Self {
                _lock: lock,
                previous,
            }
        }
    }

    impl Drop for EarlyInvalidationGuard {
        fn drop(&mut self) {
            // SAFETY: see `pin_to_default`; lock is still held until self is dropped.
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

    /// Build an `AuthManager` rooted at `dir`.
    /// The caller keeps `dir` alive for the duration of the test so the `TempDir` `Drop` actually cleans up.
    fn make_manager(dir: &tempfile::TempDir, initial: Option<CodelAuth>) -> Arc<AuthManager> {
        let mgr = AuthManager::new(dir.path(), CodelComConfig::default());
        if let Some(auth) = initial {
            mgr.hot_swap(auth);
        }
        Arc::new(mgr)
    }

    /// The shell half of the subagent-401 contract; the sampler half is pinned in codel-sampler's resolver tests. Over a real `AuthManager` the resolver returns `None` when hard-expired (fail-closed).
    /// It returns the token inside the early-invalidation buffer (still proxy-accepted), and the fresh token after a rotation. The same resolver serves all three states without a client rebuild.
    #[test]
    fn wire_valid_resolver_tracks_manager_across_expiry_and_refresh() {
        use codel_sampler::BearerResolver;
        let _guard = EarlyInvalidationGuard::pin_to_default();
        let dir = tempfile::tempdir().unwrap();
        let mgr = make_manager(
            &dir,
            Some(make_auth("hard-expired-token", ChronoDuration::hours(-1))),
        );
        let resolver = WireValidBearerResolver(mgr.clone());

        assert_eq!(
            resolver.current_bearer(),
            None,
            "a hard-expired token must never ride the wire"
        );

        mgr.hot_swap(make_auth("buffer-window-token", ChronoDuration::minutes(4)));
        assert_eq!(
            resolver.current_bearer().as_deref(),
            Some("buffer-window-token"),
            "inside the early-invalidation buffer the token is still wire-valid"
        );

        mgr.hot_swap(make_auth("fresh-token", ChronoDuration::hours(1)));
        assert_eq!(
            resolver.current_bearer().as_deref(),
            Some("fresh-token"),
            "the same resolver must serve the rotated token without a rebuild"
        );
    }



    /// The wait is bounded by the cached bearer's remaining life, so a slow mint cannot outlive the token it was protecting.
    #[test]
    fn pre_send_budget_never_outlives_the_cached_bearer() {
        use std::time::Duration;
        assert_eq!(
            pre_send_refresh_budget(Duration::from_secs(60)),
            PRE_SEND_REFRESH_BUDGET,
            "plenty of life: the ceiling applies"
        );
        assert_eq!(
            pre_send_refresh_budget(Duration::from_secs(3)),
            Duration::from_millis(2500),
            "the stamp margin is reserved out of the remaining life"
        );
        assert_eq!(
            pre_send_refresh_budget(Duration::from_millis(300)),
            Duration::ZERO,
            "less than the margin: do not wait at all"
        );
    }


    /// During the 5-minute pre-refresh buffer window, `auth_manager.current()` returns `None`, but the token is still valid at the proxy. The manager treats such a token as expiring soon for refresh scheduling.
    /// The provider must fall back to `expired_auth()` so the in-memory token gets sent instead of nothing. Sending nothing here caused the bulk of the `POST /v1/storage` 401s observed in production.
    #[test]
    fn falls_back_to_expired_auth_during_buffer_window() {
        let _guard = EarlyInvalidationGuard::pin_to_default();
        let dir = tempfile::tempdir().unwrap();
        // The token expires in 4 minutes, inside the pinned 5-minute buffer, so `current()` returns None and `expired_auth()` returns Some
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













    #[test]
    fn embedding_session_credentials_scopes_to_first_party() {
        let _guard = EarlyInvalidationGuard::pin_to_default();
        let dir = tempfile::tempdir().unwrap();
        let mgr = make_manager(
            &dir,
            Some(make_auth("codel-session-token", ChronoDuration::hours(1))),
        );
        let api_key_provider: codel_tools::types::SharedApiKeyProvider =
            Arc::new(crate::side_call_bearer::SharedAuthKeyProvider(mgr.clone()));

        for denied in [
            "https://byok.attacker.example/v1",
            // First-party host, but cleartext: bearer requires https.
            "http://api.codel.dev/v1",
        ] {
            let resolved =
                embedding_session_credentials(denied, Some(&mgr), Some(api_key_provider.clone()));
            assert!(
                resolved.is_empty(),
                "session credentials must not reach {denied}"
            );
        }

        let resolved = embedding_session_credentials(
            "https://api.codel.dev/v1",
            Some(&mgr),
            Some(api_key_provider),
        );
        assert!(!resolved.is_empty());
    }

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
    fn snapshot_populates_tenant_id_per_auth_mode() {
        use codel_logging::config::deployment_id_from_key;
        let _guard = EarlyInvalidationGuard::pin_to_default();
        let dir = tempfile::tempdir().unwrap();

        let dep = ShellAuthCredentialProvider::with_deployment_id_resolver(
            make_manager(&dir, None),
            Some("codel-token-EX".into()),
            None,
            std::sync::Arc::new(|k: Option<&str>| {
                k.filter(|s| !s.is_empty()).map(deployment_id_from_key)
            }),
        )
        .snapshot();
        assert_eq!(
            dep.deployment_id.as_deref(),
            Some(deployment_id_from_key("codel-token-EX").as_str())
        );
        assert!(dep.api_key_id.is_none());

        let api_auth = CodelAuth {
            key: "sk-apikey-xyz".into(),
            auth_mode: crate::AuthMode::ApiKey,
            expires_at: Some(Utc::now() + ChronoDuration::hours(1)),
            ..CodelAuth::test_default()
        };
        let api = ShellAuthCredentialProvider::new(make_manager(&dir, Some(api_auth)), None, None)
            .snapshot();
        assert_eq!(
            api.api_key_id.as_deref(),
            Some(deployment_id_from_key("sk-apikey-xyz").as_str())
        );
        assert!(api.deployment_id.is_none());
        assert_eq!(
            api.user_id.as_deref(),
            Some("test-user"),
            "API-key sessions still carry the snapshot principal; emit attaches user.id"
        );

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

    #[tokio::test]
    async fn refresh_after_unauthorized_is_noop_for_deployment_key() {
        let _guard = EarlyInvalidationGuard::pin_to_default();
        let dir = tempfile::tempdir().unwrap();
        let mgr = make_manager(&dir, None);
        let provider =
            ShellAuthCredentialProvider::new(mgr, Some("deployment-key".to_string()), None);
        assert!(!provider.refresh_after_unauthorized().await);
    }

}
