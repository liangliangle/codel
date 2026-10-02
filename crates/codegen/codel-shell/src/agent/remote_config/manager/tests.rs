use super::super::resolution::CatalogSource;
use super::super::{
    CACHE_TTL, CacheAuthMethod, Commit, MODELS_CACHE_FILE, ModelsCache, ModelsCacheManager,
    ModelsCacheScope, ModelsFetchFuture, allowlist_denied_message, build_prefetched_map,
    degraded_log_level, evaluate_models_commit, resolve_live_models_cache_scope,
    resolve_models_cache_scope, resolve_prefetch_inputs_from_parts, resolve_startup_endpoints,
    selectable_catalog_key_for_persisted,
};
use super::*;
use chrono::{Duration as ChronoDuration, Utc};
use std::collections::BTreeSet;
fn test_manager() -> ModelsManager {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_test_writer()
        .try_init();
    let tmp = tempfile::TempDir::new().unwrap();
    let auth_manager = Arc::new(AuthManager::new(tmp.path(), CodelComConfig::default()));
    ModelsManagerBuilder::new(
        None,
        IndexMap::new(),
        acp::ModelId::new("default"),
        auth_manager,
        config::Config::default(),
    )
    .cache(test_cache_manager(tmp.path()))
    .build()
}
/// Cold manager (no prefetch, isolated cache and auth) over `endpoint`.
fn cold_manager(cfg: config::Config, endpoint: Arc<dyn ModelsEndpoint>) -> ModelsManager {
    let tmp = tempfile::TempDir::new().unwrap();
    let auth_manager = Arc::new(AuthManager::new(tmp.path(), CodelComConfig::default()));
    ModelsManagerBuilder::new(
        None,
        IndexMap::new(),
        acp::ModelId::new("default"),
        auth_manager,
        cfg,
    )
    .endpoint(endpoint)
    .cache(test_cache_manager(tmp.path()))
    .build()
}
struct HangingEndpoint;
impl ModelsEndpoint for HangingEndpoint {
    fn fetch_models(
        &self,
        _endpoints: config::EndpointsConfig,
        _auth: Option<CodelAuth>,
        _fetch_auth: ModelFetchAuth,
    ) -> ModelsFetchFuture {
        Box::pin(std::future::pending())
    }
}
struct FailingEndpoint;
impl ModelsEndpoint for FailingEndpoint {
    fn fetch_models(
        &self,
        _endpoints: config::EndpointsConfig,
        _auth: Option<CodelAuth>,
        _fetch_auth: ModelFetchAuth,
    ) -> ModelsFetchFuture {
        Box::pin(async { None })
    }
}
struct CountingEndpoint {
    calls: Arc<std::sync::atomic::AtomicUsize>,
}
impl ModelsEndpoint for CountingEndpoint {
    fn fetch_models(
        &self,
        _endpoints: config::EndpointsConfig,
        _auth: Option<CodelAuth>,
        _fetch_auth: ModelFetchAuth,
    ) -> ModelsFetchFuture {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Box::pin(async { None })
    }
}
struct SlowEndpoint {
    catalog: IndexMap<String, ModelEntry>,
    delay: std::time::Duration,
}
impl ModelsEndpoint for SlowEndpoint {
    fn fetch_models(
        &self,
        _endpoints: config::EndpointsConfig,
        _auth: Option<CodelAuth>,
        _fetch_auth: ModelFetchAuth,
    ) -> ModelsFetchFuture {
        let catalog = self.catalog.clone();
        let delay = self.delay;
        Box::pin(async move {
            tokio::time::sleep(delay).await;
            Some(catalog)
        })
    }
}




#[tokio::test(start_paused = true)]
async fn hanging_fetch_does_not_block_refresh() {
    let mgr = cold_manager(config::Config::default(), Arc::new(HangingEndpoint));
    tokio::time::timeout(
        crate::http::STARTUP_FETCH_TIMEOUT * 10,
        mgr.fetch_and_apply_inner(true),
    )
    .await
    .expect("fetch_and_apply_inner must return despite a hanging endpoint");
    assert!(
        !mgr.has_fetched_real_catalog(),
        "a timed-out fetch must not mark a real catalog",
    );
}


#[tokio::test(start_paused = true)]
async fn etag_refresh_is_bounded_and_single_flighted() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct CountingHangEndpoint {
        calls: Arc<AtomicUsize>,
    }
    impl ModelsEndpoint for CountingHangEndpoint {
        fn fetch_models(
            &self,
            _endpoints: config::EndpointsConfig,
            _auth: Option<CodelAuth>,
            _fetch_auth: ModelFetchAuth,
        ) -> ModelsFetchFuture {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Box::pin(std::future::pending())
        }
    }
    let calls = Arc::new(AtomicUsize::new(0));
    let tmp = tempfile::TempDir::new().unwrap();
    let auth_manager = Arc::new(AuthManager::new(tmp.path(), CodelComConfig::default()));
    let mgr = ModelsManagerBuilder::new(
        None,
        IndexMap::new(),
        acp::ModelId::new("default"),
        auth_manager,
        config::Config::default(),
    )
    .endpoint(Arc::new(CountingHangEndpoint {
        calls: calls.clone(),
    }))
    .build();
    mgr.spawn_fetch_inner(Some("etag-1".into()), true);
    tokio::task::yield_now().await;
    mgr.spawn_fetch_inner(Some("etag-2".into()), true);
    tokio::task::yield_now().await;
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "single-flight: only one etag fetch in flight at a time",
    );
    tokio::time::sleep(crate::http::STARTUP_FETCH_TIMEOUT * 2).await;
    tokio::task::yield_now().await;
    mgr.spawn_fetch_inner(Some("etag-3".into()), true);
    tokio::task::yield_now().await;
    assert_eq!(
        calls.load(Ordering::SeqCst),
        2,
        "after the timeout cleared the in-flight guard, a new etag fetch proceeds",
    );
    mgr.spawn_fetch_inner(Some("etag-4".into()), false);
    tokio::task::yield_now().await;
    assert_eq!(
        calls.load(Ordering::SeqCst),
        2,
        "disabled gate must not fetch"
    );
}


#[tokio::test(start_paused = true)]
async fn first_catalog_wait_unblocks_on_failed_fetch() {
    let mgr = cold_manager(
        config_from_toml("[endpoints]\ndeployment_key = \"deploy-key\""),
        Arc::new(FailingEndpoint),
    );
    let budget = crate::http::STARTUP_AUTH_REFRESH_TIMEOUT + crate::http::STARTUP_FETCH_TIMEOUT;
    let start = tokio::time::Instant::now();
    mgr.spawn_fetch_inner(None, true);
    assert!(
        !mgr.wait_for_first_catalog(/*remote_fetch_enabled*/ true)
            .await
    );
    assert!(start.elapsed() < budget, "failure must beat the budget");
}
#[tokio::test(start_paused = true)]
async fn first_catalog_wait_is_bounded() {
    let mgr = cold_manager(
        config_from_toml("[endpoints]\ndeployment_key = \"deploy-key\""),
        Arc::new(HangingEndpoint),
    );
    let budget = crate::http::STARTUP_AUTH_REFRESH_TIMEOUT + crate::http::STARTUP_FETCH_TIMEOUT;
    let _attempt = FetchAttemptGuard::begin(&mgr.inner);
    let start = tokio::time::Instant::now();
    assert!(
        !mgr.wait_for_first_catalog(/*remote_fetch_enabled*/ true)
            .await
    );
    assert_eq!(start.elapsed(), budget, "only the budget ends this wait");
}
#[tokio::test(start_paused = true)]
#[serial]
async fn first_catalog_wait_skips_doomed_signed_out_fetch() {
    let _no_key = EnvGuard::unset("CODEL_API_KEY");
    let _no_legacy_key = EnvGuard::unset("CODEL_CODE_CODEL_API_KEY");
    let mgr = cold_manager(config::Config::default(), Arc::new(HangingEndpoint));
    let start = tokio::time::Instant::now();
    mgr.spawn_fetch_inner(None, true);
    assert!(
        !mgr.wait_for_first_catalog(/*remote_fetch_enabled*/ true)
            .await
    );
    assert_eq!(start.elapsed(), std::time::Duration::ZERO);
}


#[tokio::test(start_paused = true)]
async fn new_fetch_attempt_supersedes_failed_latch() {
    let mgr = cold_manager(
        config_from_toml("[endpoints]\ndeployment_key = \"deploy-key\""),
        Arc::new(FailingEndpoint),
    );
    mgr.fetch_and_apply_inner(true).await;
    assert_eq!(
        *mgr.inner.catalog_progress.borrow(),
        CatalogProgress::Failed
    );
    let attempt = FetchAttemptGuard::begin(&mgr.inner);
    assert_eq!(
        *mgr.inner.catalog_progress.borrow(),
        CatalogProgress::Pending,
        "a new attempt must supersede the stale failure",
    );
    drop(attempt);
    assert_eq!(
        *mgr.inner.catalog_progress.borrow(),
        CatalogProgress::Failed,
        "the last attempt out without an outcome must latch",
    );
    let start = tokio::time::Instant::now();
    assert!(
        !mgr.wait_for_first_catalog(/*remote_fetch_enabled*/ true)
            .await
    );
    assert_eq!(start.elapsed(), std::time::Duration::ZERO);
}


fn config_from_toml(toml: &str) -> config::Config {
    config::Config::new_from_toml_cfg(&toml::from_str(toml).unwrap()).unwrap()
}



#[test]
fn default_model_honors_allowlist_when_no_default_set() {
    let cfg = config_from_toml(
        r#"
            [models]
            allowed_models = ["keep-*"]
            [model.zzz-first]
            model = "zzz-first"
            base_url = "https://api.codel.dev/v1"
            context_window = 256000
            [model.keep-one]
            model = "keep-one"
            base_url = "https://api.codel.dev/v1"
            context_window = 256000
            "#,
    );
    let catalog = resolve_model_catalog(&cfg, None);
    let (_key, entry, _src) = resolve_default_model(&cfg, &catalog, true);
    assert!(
        entry.info.user_selectable,
        "picked non-selectable {}",
        entry.model
    );
}
#[test]
fn validate_selectable_rejects_bad_allowlists() {
    let excluded = config_from_toml(
        r#"
            [models]
            default = "codel-3"
            allowed_models = ["codel-4*"]
            [model.codel-3]
            model = "codel-3"
            base_url = "https://api.codel.dev/v1"
            context_window = 256000
            [model.codel-4]
            model = "codel-4"
            base_url = "https://api.codel.dev/v1"
            context_window = 256000
            "#,
    );
    let catalog = resolve_model_catalog(&excluded, None);
    assert!(
        validate_selectable(&excluded, &catalog)
            .unwrap_err()
            .contains("codel-3")
    );
    let zero = config_from_toml(
        r#"
            [models]
            allowed_models = ["nomatch-*"]
            [model.codel-4]
            model = "codel-4"
            base_url = "https://api.codel.dev/v1"
            context_window = 256000
            "#,
    );
    let catalog = resolve_model_catalog(&zero, None);
    assert!(validate_selectable(&zero, &catalog).is_err());
}
#[test]
fn from_config_defers_validate_without_prefetch() {
    let tmp = tempfile::TempDir::new().unwrap();
    let auth_manager = Arc::new(AuthManager::new(tmp.path(), CodelComConfig::default()));
    let mut cfg = config::Config::default();
    cfg.requirements.allowed_models.pin(
        crate::agent::config::AllowlistPin::List(vec!["nomatch-*".into()]),
        crate::config::RequirementSource::Unknown,
    );
    let mgr = ModelsManager::from_config(&cfg, None, auth_manager)
        .expect("cold start must not validate against built-ins-only");
    assert!(
        !mgr.has_fetched_real_catalog(),
        "no prefetch means the first-fetch gate has not run"
    );
    assert!(
        mgr.allowlist_excludes_all(),
        "from_config must latch the prompt-path guard on a builtins-only miss"
    );
}
#[test]
fn set_session_model_fleet_deny_uses_organization_message() {
    let raw: toml::Value = toml::from_str(
        r#"
            [models]
            [model.codel-3]
            model = "codel-3"
            base_url = "https://api.codel.dev/v1"
            context_window = 256000
            [model.codel-4]
            model = "codel-4"
            base_url = "https://api.codel.dev/v1"
            context_window = 256000
            "#,
    )
    .unwrap();
    let mut cfg = config::Config::new_from_toml_cfg(&raw).unwrap();
    cfg.requirements.allowed_models.pin(
        crate::agent::config::AllowlistPin::List(vec!["codel-4".into()]),
        crate::config::RequirementSource::Unknown,
    );
    let catalog = resolve_model_catalog(&cfg, None);
    let Some(codel3) = catalog.get("codel-3") else {
        panic!("expected codel-3: {catalog:?}");
    };
    assert!(!codel3.info.user_selectable);
    let msg = allowlist_denied_message(&cfg);
    assert!(
        msg.contains("organization"),
        "set_session_model /model gate must use the fleet deny string: {msg}"
    );
    assert!(!msg.contains("allowed_models"));
}
#[tokio::test]
async fn refresh_if_new_etag_skips_when_same() {
    let mgr = test_manager();
    mgr.inner.catalog.write().etag = Some("\"abc123\"".to_string());
    mgr.refresh_if_new_etag("\"abc123\"".to_string()).await;
    assert_eq!(
        mgr.inner.catalog.read().etag.as_deref(),
        Some("\"abc123\""),
        "etag should remain unchanged when same"
    );
}
#[tokio::test]
async fn set_current_model_id_change_fires_watch_to_all_subscribers() {
    let mgr = test_manager();
    let mut rx_a = mgr.subscribe_model_switch();
    let mut rx_b = mgr.subscribe_model_switch();
    let initial_a = *rx_a.borrow_and_update();
    let initial_b = *rx_b.borrow_and_update();
    assert_eq!(initial_a, initial_b);
    mgr.set_current_model_id(acp::ModelId::new("default"));
    let same_id_ticked = tokio::time::timeout(std::time::Duration::from_millis(25), rx_a.changed())
        .await
        .is_ok();
    assert!(
        !same_id_ticked,
        "set_current_model_id(same id) must NOT bump the watch generation",
    );
    mgr.set_current_model_id(acp::ModelId::new("codel-4"));
    tokio::time::timeout(std::time::Duration::from_millis(100), rx_a.changed())
        .await
        .expect("rx_a saw the switch")
        .expect("watch channel still open");
    tokio::time::timeout(std::time::Duration::from_millis(100), rx_b.changed())
        .await
        .expect("rx_b saw the switch")
        .expect("watch channel still open");
    assert_ne!(*rx_a.borrow(), initial_a);
    assert_eq!(*rx_a.borrow(), *rx_b.borrow());
    assert!(mgr.model_switch_generation() > initial_a);
}
#[tokio::test]
async fn model_switch_generation_snapshot_reflects_current_state() {
    let mgr = test_manager();
    let start = mgr.model_switch_generation();
    mgr.set_current_model_id(acp::ModelId::new("codel-4"));
    assert_eq!(mgr.model_switch_generation(), start + 1);
    mgr.set_current_model_id(acp::ModelId::new("codel-4"));
    assert_eq!(mgr.model_switch_generation(), start + 1);
    mgr.set_current_model_id(acp::ModelId::new("codel-3"));
    assert_eq!(mgr.model_switch_generation(), start + 2);
}




#[test]
fn current_reasoning_effort_round_trip() {
    let mgr = test_manager();
    assert_eq!(mgr.current_reasoning_effort(), None);
    mgr.set_current_reasoning_effort(Some(ReasoningEffort::High));
    assert_eq!(mgr.current_reasoning_effort(), Some(ReasoningEffort::High));
    mgr.set_current_reasoning_effort(None);
    assert_eq!(mgr.current_reasoning_effort(), None);
}
#[test]
fn current_reasoning_effort_seeded_from_config() {
    let tmp = tempfile::TempDir::new().unwrap();
    let auth_manager = Arc::new(AuthManager::new(tmp.path(), CodelComConfig::default()));
    let mut cfg = config::Config::default();
    cfg.models.default_reasoning_effort = Some(ReasoningEffort::Xhigh);
    let mgr = ModelsManager::new(
        None,
        IndexMap::new(),
        acp::ModelId::new("default"),
        auth_manager,
        cfg,
    );
    assert_eq!(mgr.current_reasoning_effort(), Some(ReasoningEffort::Xhigh),);
}



#[test]
fn config_menu_only_model_derives_support_and_default() {
    let mut cfg = config::Config::default();
    cfg.config_models.insert(
        "menu-only".to_string(),
        config::ConfigModelOverride {
            reasoning_efforts: vec![
                ReasoningEffortOption {
                    id: "balanced".to_string(),
                    value: ReasoningEffort::Medium,
                    label: "Balanced".to_string(),
                    description: None,
                    default: false,
                },
                ReasoningEffortOption {
                    id: "deep".to_string(),
                    value: ReasoningEffort::Xhigh,
                    label: "Deep".to_string(),
                    description: None,
                    default: true,
                },
            ],
            ..Default::default()
        },
    );
    cfg.config_models
        .insert("plain".to_string(), config::ConfigModelOverride::default());
    let catalog = resolve_model_catalog(&cfg, None);
    let Some(menu_only) = catalog.get("menu-only") else {
        panic!("expected menu-only: {catalog:?}");
    };
    let info = &menu_only.info;
    assert!(
        info.supports_reasoning_effort,
        "menu-only model must derive support"
    );
    assert_eq!(
        info.reasoning_effort,
        Some(ReasoningEffort::Xhigh),
        "derived default = marked-default option value"
    );
    let Some(plain) = catalog.get("plain") else {
        panic!("expected plain: {catalog:?}");
    };
    assert!(!plain.info.supports_reasoning_effort);
    assert_eq!(plain.info.reasoning_effort, None);
    let tmp = tempfile::TempDir::new().unwrap();
    let auth_manager = Arc::new(AuthManager::new(tmp.path(), CodelComConfig::default()));
    let mgr = ModelsManager::new(
        None,
        catalog,
        acp::ModelId::new("menu-only"),
        auth_manager,
        cfg,
    );
    assert!(mgr.model_supports_reasoning_effort("menu-only"));
    assert_eq!(
        mgr.model_default_reasoning_effort("menu-only"),
        Some(ReasoningEffort::Xhigh)
    );
    assert_eq!(mgr.model_reasoning_efforts("menu-only").len(), 2);
    assert!(!mgr.model_supports_reasoning_effort("plain"));
    assert_eq!(mgr.model_default_reasoning_effort("plain"), None);
    mgr.set_current_model_id(acp::ModelId::new("plain"));
    assert_eq!(mgr.current_model_id().0.as_ref(), "plain");
    assert_eq!(mgr.model_reasoning_efforts("menu-only").len(), 2);
    assert!(mgr.model_reasoning_efforts("plain").is_empty());
}


#[test]
fn apply_refresh_result_only_updates_etag_on_success() {
    let mgr = test_manager();
    let cfg = config::Config::default();
    mgr.inner.catalog.write().etag = Some("\"old\"".to_string());
    assert!(
        !mgr.apply_refresh_result(&cfg, None, Some("\"new\"".to_string())),
        "failed refresh should report no update"
    );
    assert_eq!(
        mgr.inner.catalog.read().etag.as_deref(),
        Some("\"old\""),
        "etag should remain unchanged when refresh fails"
    );
    assert!(
        mgr.prefetched().is_none(),
        "prefetched models should stay unchanged"
    );
    assert!(
        !mgr.has_fetched_real_catalog(),
        "failed refresh must not flip has_fetched_real_catalog"
    );
}



#[test]
fn spawn_background_refresh_is_noop_when_real_catalog_present() {
    let mgr = test_manager();
    mgr.inner.catalog.write().has_fetched_real_catalog = true;
    mgr.spawn_background_refresh_inner(true);
    assert!(mgr.has_fetched_real_catalog());
}



#[test]
fn from_config_without_prefetch_produces_usable_catalog() {
    let tmp = tempfile::TempDir::new().unwrap();
    let auth_manager = Arc::new(AuthManager::new(tmp.path(), CodelComConfig::default()));
    let cfg = config::Config::default();
    let mgr = ModelsManager::from_config(&cfg, None, auth_manager).unwrap();
    let cat = mgr.inner.catalog.read();
    let catalog = &cat.models;
    assert!(
        !catalog.is_empty(),
        "zero-network boot must produce at least one model in the internal catalog"
    );
    let default = mgr.current_model_id();
    assert!(
        catalog.contains_key(default.0.as_ref()),
        "default model {:?} not in internal catalog: {:?}",
        default,
        catalog.keys().collect::<Vec<_>>()
    );
    drop(cat);
    assert!(
        !mgr.has_fetched_real_catalog(),
        "cold-cache boot must not claim a real catalog"
    );
}










fn test_cache_manager(dir: &std::path::Path) -> ModelsCacheManager {
    ModelsCacheManager::at(dir.join(MODELS_CACHE_FILE), CACHE_TTL)
}












#[test]
#[serial]
fn api_key_scope_identity_differs_per_key() {
    let _no_legacy = EnvGuard::unset("CODEL_CODE_CODEL_API_KEY");
    let endpoints = config::EndpointsConfig::default();
    let identity_for = |key: &str| {
        let _key = EnvGuard::set("CODEL_API_KEY", key);
        resolve_models_cache_scope(&endpoints, ModelFetchAuth::ApiKey, None).identity
    };
    assert_eq!(
        identity_for("key-a"),
        identity_for("key-a"),
        "the same API key must resolve to one scope",
    );
    assert_ne!(
        identity_for("key-a"),
        identity_for("key-b"),
        "different API keys must not cross-read one cache entry",
    );
}
#[test]
#[serial]
fn custom_endpoint_scope_identity_differs_per_key() {
    let _no_legacy = EnvGuard::unset("CODEL_CODE_CODEL_API_KEY");
    let endpoints = config::EndpointsConfig::default();
    let identity_for = |key: &str| {
        let _key = EnvGuard::set("CODEL_API_KEY", key);
        resolve_models_cache_scope(&endpoints, ModelFetchAuth::CustomEndpoint, None).identity
    };
    assert_eq!(
        identity_for("key-a"),
        identity_for("key-a"),
        "the same custom-endpoint key must resolve to one scope",
    );
    assert_ne!(
        identity_for("key-a"),
        identity_for("key-b"),
        "different keys on one custom endpoint must not cross-read",
    );
}
#[test]
#[serial]
fn custom_endpoint_scope_ignores_session_identity() {
    let _no_key = EnvGuard::unset("CODEL_API_KEY");
    let _no_legacy = EnvGuard::unset("CODEL_CODE_CODEL_API_KEY");
    let endpoints = config::EndpointsConfig::default();
    let auth = CodelAuth {
        user_id: "account-a".to_string(),
        key: "token-1".to_string(),
        ..CodelAuth::test_default()
    };
    let with_session =
        resolve_models_cache_scope(&endpoints, ModelFetchAuth::CustomEndpoint, Some(&auth))
            .identity;
    let without_session =
        resolve_models_cache_scope(&endpoints, ModelFetchAuth::CustomEndpoint, None).identity;
    assert_eq!(
        with_session, without_session,
        "the session identity must not scope a custom-endpoint cache",
    );
}
#[test]
#[serial]
fn custom_endpoint_scope_keys_on_the_third_party_provider_login() {
    let _no_key = EnvGuard::unset("CODEL_API_KEY");
    let _no_legacy = EnvGuard::unset("CODEL_CODE_CODEL_API_KEY");
    let endpoints = config::EndpointsConfig::default();
    let identity_for = |auth: Option<&CodelAuth>| {
        resolve_models_cache_scope(&endpoints, ModelFetchAuth::CustomEndpoint, auth).identity
    };
    let provider_login = |key: &str| CodelAuth {
        key: key.to_owned(),
        user_id: String::new(),
        auth_mode: codel_login::AuthMode::External,
        ..CodelAuth::test_default()
    };
    let login_a = identity_for(Some(&provider_login("provider-token-a")));
    assert_ne!(identity_for(None), login_a);
    assert_ne!(
        identity_for(Some(&provider_login("provider-token-b"))),
        login_a
    );
}
#[test]
fn models_commit_gate_detects_account_switch() {
    let scope_for = |user_id: &str| ModelsCacheScope {
        auth_method: CacheAuthMethod::Session,
        origin: "https://origin.example/v1/models".to_string(),
        identity: codel_cloud_config::settings_cache_identity(
            &CodelAuth {
                user_id: user_id.to_string(),
                ..CodelAuth::test_default()
            },
            None,
        ),
    };
    let expected = scope_for("account-a");
    assert!(
        matches!(
            evaluate_models_commit(&expected, &scope_for("account-a"), true),
            Commit::CacheAndServe
        ),
        "the same account (token rotation) must still cache",
    );
    assert!(
        matches!(
            evaluate_models_commit(&expected, &scope_for("account-b"), true),
            Commit::ServeInMemory
        ),
        "a mid-fetch account switch must serve in memory, not cache under the old account",
    );
    let moved = ModelsCacheScope {
        origin: "https://other.example/v1/models".to_string(),
        ..expected.clone()
    };
    assert!(matches!(
        evaluate_models_commit(&expected, &moved, true),
        Commit::Abandon
    ));
}
#[test]
#[serial]
fn resolve_live_keeps_fetch_origin_when_disk_auth_absent() {
    let _no_legacy = EnvGuard::unset("CODEL_CODE_CODEL_API_KEY");
    let _key = EnvGuard::set("CODEL_API_KEY", "boot-window-key");
    let session_auth = CodelAuth {
        user_id: "session-user".to_string(),
        ..CodelAuth::test_default()
    };
    let endpoints = resolve_startup_endpoints();
    let expected =
        resolve_models_cache_scope(&endpoints, ModelFetchAuth::Session, Some(&session_auth));
    let live = resolve_live_models_cache_scope(ModelFetchAuth::Session, None);
    assert_eq!(
        live.origin, expected.origin,
        "the live mode must not flip to the api-key origin",
    );
    assert!(
        matches!(
            evaluate_models_commit(&expected, &live, true),
            Commit::ServeInMemory
        ),
        "a disk-absent Session commit must serve in memory, not abandon the catalog",
    );
}



#[test]
fn is_campaign_only_flip_detects_campaign_driven_changes() {
    let camp: std::collections::HashSet<String> = ["beta".into()].into_iter().collect();
    assert!(is_campaign_only_flip(
        &Some("alpha".into()),
        &Some("beta".into()),
        &camp
    ));
    assert!(is_campaign_only_flip(
        &Some("beta".into()),
        &Some("alpha".into()),
        &camp
    ));
    assert!(!is_campaign_only_flip(
        &Some("alpha".into()),
        &Some("gamma".into()),
        &camp
    ));
    assert!(!is_campaign_only_flip(
        &Some("beta".into()),
        &Some("beta".into()),
        &camp
    ));
    assert!(!is_campaign_only_flip(&Some("beta".into()), &None, &camp));
    assert!(!is_campaign_only_flip(
        &Some("alpha".into()),
        &Some("beta".into()),
        &std::collections::HashSet::new()
    ));
}



use serial_test::serial;
use codel_test_support::EnvGuard;
#[test]
#[serial]
fn resolve_custom_endpoint_always_wins() {
    let _key = EnvGuard::set("CODEL_API_KEY", "test-key");
    let endpoints = config::EndpointsConfig {
        models_base_url: Some("https://custom.example.com".to_owned()),
        ..config::EndpointsConfig::default()
    };
    assert_eq!(
        ModelFetchAuth::resolve(&endpoints, true),
        ModelFetchAuth::CustomEndpoint,
    );
    assert_eq!(
        ModelFetchAuth::resolve(&endpoints, false),
        ModelFetchAuth::CustomEndpoint,
    );
}
#[test]
#[serial]
fn resolve_cached_session_wins_over_api_key() {
    let _key = EnvGuard::set("CODEL_API_KEY", "test-key");
    let endpoints = config::EndpointsConfig::default();
    assert_eq!(
        ModelFetchAuth::resolve(&endpoints, true),
        ModelFetchAuth::Session,
        "cached session should take priority over API key",
    );
}
#[test]
#[serial]
fn resolve_api_key_used_when_no_session() {
    let _key = EnvGuard::set("CODEL_API_KEY", "test-key");
    let endpoints = config::EndpointsConfig::default();
    assert_eq!(
        ModelFetchAuth::resolve(&endpoints, false),
        ModelFetchAuth::ApiKey,
        "API key should be used when no cached session exists",
    );
}
#[test]
#[serial]
fn resolve_falls_back_to_session_when_nothing_set() {
    let _unset = EnvGuard::unset("CODEL_API_KEY");
    let _unset_legacy = EnvGuard::unset("CODEL_CODE_CODEL_API_KEY");
    let endpoints = config::EndpointsConfig::default();
    assert_eq!(
        ModelFetchAuth::resolve(&endpoints, false),
        ModelFetchAuth::Session,
        "should fall back to Session when nothing else is configured",
    );
}
#[test]
#[serial]
fn resolve_deployment_key_when_no_session_or_api_key() {
    let _unset = EnvGuard::unset("CODEL_API_KEY");
    let _unset_legacy = EnvGuard::unset("CODEL_CODE_CODEL_API_KEY");
    let endpoints = config::EndpointsConfig {
        deployment_key: Some("deploy-key".to_owned()),
        ..config::EndpointsConfig::default()
    };
    assert_eq!(
        ModelFetchAuth::resolve(&endpoints, false),
        ModelFetchAuth::Deployment,
    );
}
#[test]
#[serial]
fn resolve_deployment_key_outranks_ambient_api_key() {
    let _key = EnvGuard::set("CODEL_API_KEY", "stray-env-key");
    let endpoints = config::EndpointsConfig {
        deployment_key: Some("deploy-key".to_owned()),
        ..config::EndpointsConfig::default()
    };
    assert_eq!(
        ModelFetchAuth::resolve(&endpoints, false),
        ModelFetchAuth::Deployment,
        "managed deployment_key should outrank an ambient CODEL_API_KEY",
    );
    assert_eq!(
        ModelFetchAuth::resolve(&endpoints, true),
        ModelFetchAuth::Session,
        "an active session should still win over a managed deployment",
    );
}
#[test]
#[serial]
fn prefetch_env_none_when_remote_fetch_disabled_despite_credentials() {
    let _key = EnvGuard::set("CODEL_API_KEY", "stray-env-key");
    let endpoints = config::EndpointsConfig {
        deployment_key: Some("deploy-key".to_owned()),
        models_base_url: Some("https://custom.example.com".to_owned()),
        ..config::EndpointsConfig::default()
    };
    assert!(
        resolve_prefetch_inputs_from_parts(
            Some(CodelAuth::test_default()),
            endpoints.clone(),
            false,
        )
        .is_none(),
        "session auth must not re-arm the prefetch when remote_fetch is off",
    );
    assert!(
        resolve_prefetch_inputs_from_parts(None, endpoints, false).is_none(),
        "API key / deployment key / custom endpoint must not re-arm it either",
    );
}
#[test]
#[serial]
fn prefetch_env_resolves_when_remote_fetch_enabled() {
    let _unset = EnvGuard::unset("CODEL_API_KEY");
    let _unset_legacy = EnvGuard::unset("CODEL_CODE_CODEL_API_KEY");
    let endpoints = config::EndpointsConfig {
        deployment_key: Some("deploy-key".to_owned()),
        ..config::EndpointsConfig::default()
    };
    assert!(resolve_prefetch_inputs_from_parts(None, endpoints, true).is_some());
    assert!(
        resolve_prefetch_inputs_from_parts(None, config::EndpointsConfig::default(), true)
            .is_none(),
        "no credentials and no custom endpoint must stay a no-prefetch launch",
    );
}



#[test]
fn visible_for_auth_logic() {
    let mut info = config::ModelInfo::fallback("test");
    assert!(info.visible_for_auth(true));
    assert!(info.visible_for_auth(false));
    info.hidden = true;
    assert!(!info.visible_for_auth(true));
    assert!(!info.visible_for_auth(false));
    info.hidden = false;
    info.supported_in_api = false;
    assert!(info.visible_for_auth(true));
    assert!(!info.visible_for_auth(false));
}
fn make_entry_config(model: &str, name: Option<&str>) -> config::ModelEntryConfig {
    make_entry_config_with_id(None, model, name)
}
fn make_entry_config_with_id(
    id: Option<&str>,
    model: &str,
    name: Option<&str>,
) -> config::ModelEntryConfig {
    config::ModelEntryConfig {
        id: id.map(|s| s.to_owned()),
        model: model.to_owned(),
        base_url: "https://test.api/v1".to_owned(),
        name: name.map(|n| n.to_owned()),
        context_window: std::num::NonZeroU64::new(200_000).unwrap(),
        ..Default::default()
    }
}
#[test]
fn build_prefetched_map_distinct_ids_same_slug() {
    let entries = vec![
        make_entry_config_with_id(Some("auto"), "codel-build", Some("Auto")),
        make_entry_config_with_id(Some("codel-build"), "codel-build", Some("Codel Build")),
        make_entry_config_with_id(
            Some("experimental-fast"),
            "experimental-fast",
            Some("Codel Fast"),
        ),
    ];
    let map = build_prefetched_map(entries, None);
    assert_eq!(map.len(), 3, "all three entries should survive");
    assert!(map.contains_key("auto"));
    assert!(map.contains_key("codel-build"));
    assert!(map.contains_key("experimental-fast"));
    let Some(auto) = map.get("auto") else {
        panic!("expected auto: {map:?}");
    };
    assert_eq!(
        auto.info.model, "codel-build",
        "auto entry should still route to codel-build"
    );
    let Some(build) = map.get("codel-build") else {
        panic!("expected codel-build: {map:?}");
    };
    assert_eq!(build.info.model, "codel-build");
}
#[test]
fn build_prefetched_map_no_id_falls_back_to_slug() {
    let entries = vec![
        make_entry_config("model-a", Some("Model A")),
        make_entry_config("model-b", Some("Model B")),
    ];
    let map = build_prefetched_map(entries, None);
    assert_eq!(map.len(), 2);
    assert!(map.contains_key("model-a"));
    assert!(map.contains_key("model-b"));
}
#[test]
fn build_prefetched_map_duplicate_id_overwrites() {
    let entries = vec![
        make_entry_config_with_id(Some("codel-build"), "codel-build", Some("First")),
        make_entry_config_with_id(Some("codel-build"), "codel-build", Some("Second")),
    ];
    let map = build_prefetched_map(entries, None);
    assert_eq!(map.len(), 1, "duplicate id: second overwrites first");
    let Some(build) = map.get("codel-build") else {
        panic!("expected codel-build: {map:?}");
    };
    assert_eq!(build.info.name.as_deref(), Some("Second"));
}









fn test_available_keys(keys: &[&str]) -> IndexMap<acp::ModelId, acp::ModelInfo> {
    keys.iter()
        .map(|k| {
            let id = acp::ModelId::new(*k);
            (id.clone(), acp::ModelInfo::new(id, (*k).to_string()))
        })
        .collect()
}
#[tokio::test(start_paused = true)]
async fn bounded_auth_refresh_times_out_to_none() {
    let started = tokio::time::Instant::now();
    let result =
        ModelsManager::bounded_auth_refresh(std::future::pending::<Option<CodelAuth>>()).await;
    assert!(result.is_none(), "a hung auth refresh must yield None");
    assert!(
        started.elapsed() >= crate::http::STARTUP_AUTH_REFRESH_TIMEOUT,
        "must wait the full bound before giving up",
    );
}
#[tokio::test]
async fn bounded_auth_refresh_passes_through_ready_value() {
    let result =
        ModelsManager::bounded_auth_refresh(async { Some(CodelAuth::test_default()) }).await;
    assert!(
        result.is_some(),
        "a ready session must pass through unchanged"
    );
}



#[test]
fn personal_offline_boot_does_not_emit_a_managed_degraded_warn() {
    use codel_cloud_config::managed_config::LaunchProfile;
    use codel_logging::unified_log::LogLevel;
    assert_eq!(
        degraded_log_level(LaunchProfile::Personal),
        LogLevel::Debug,
        "a personal offline boot must not WARN on every degraded start",
    );
    assert_eq!(
        degraded_log_level(LaunchProfile::Managed),
        LogLevel::Warn,
        "a managed degraded start stays a WARN: settings can gate the client",
    );
}
const EXTERNAL_AUTH: &str = "[auth]\nauth_provider_command = \"/usr/local/bin/my-auth-provider\"\n";
const PROXY_MODELS_BASE_URL: &str =
    "[endpoints]\nmodels_base_url = \"https://proxy.example.com/v1\"\n";
/// External auth plus a models endpoint, with `extra` appended.
fn external_auth_config(extra: &str) -> config::Config {
    config_from_toml(&format!("{EXTERNAL_AUTH}{PROXY_MODELS_BASE_URL}{extra}"))
}
fn catalog_ids(catalog: &IndexMap<String, ModelEntry>) -> BTreeSet<&str> {
    catalog.keys().map(String::as_str).collect()
}
#[test]
fn catalog_source_needs_external_auth_and_a_models_endpoint() {
    let list_url = "[endpoints]\nmodels_list_url = \"https://proxy.example.com/v1/models\"\n";
    let sources = [
        format!("{EXTERNAL_AUTH}{PROXY_MODELS_BASE_URL}"),
        format!("{EXTERNAL_AUTH}{list_url}"),
        EXTERNAL_AUTH.to_owned(),
        PROXY_MODELS_BASE_URL.to_owned(),
        String::new(),
    ]
    .map(|toml| CatalogSource::for_config(&config_from_toml(&toml)));
    assert_eq!(
        [
            CatalogSource::ModelsEndpoint,
            CatalogSource::ModelsEndpoint,
            CatalogSource::Standard,
            CatalogSource::ModelsEndpoint,
            CatalogSource::Standard,
        ],
        sources
    );
}
#[test]
fn models_endpoint_catalog_is_exactly_the_listed_rows() {
    let tables = r#"
        [model.proxy-a]
        name = "Proxy A from config"
        [model.config-only]
        model = "config-only"
        base_url = "https://elsewhere.example.com/v1"
    "#;
    let listed = make_prefetched(&["proxy-a", "proxy-b"]);
    let external = resolve_model_catalog(&external_auth_config(tables), Some(listed.clone()));
    let standard = resolve_model_catalog(
        &config_from_toml(&format!("{PROXY_MODELS_BASE_URL}{tables}")),
        Some(listed.clone()),
    );
    assert_eq!(
        BTreeSet::from(["proxy-a", "proxy-b"]),
        catalog_ids(&external)
    );
    assert_eq!(
        listed.get("proxy-a").and_then(|e| e.info.name.as_deref()),
        external.get("proxy-a").and_then(|e| e.info.name.as_deref())
    );
    assert_eq!(
        Some("Proxy A from config"),
        standard.get("proxy-a").and_then(|e| e.info.name.as_deref())
    );
    assert_eq!(
        BTreeSet::from(["config-only", "proxy-a", "proxy-b"]),
        catalog_ids(&standard)
    );
}
#[test]
fn models_endpoint_catalog_without_a_list_is_empty() {
    let tables = "[model.config-only]\nmodel = \"config-only\"\n";
    let external = resolve_model_catalog(&external_auth_config(tables), None);
    let standard = resolve_model_catalog(
        &config_from_toml(&format!("{PROXY_MODELS_BASE_URL}{tables}")),
        None,
    );
    assert!(external.is_empty());
    assert!(standard.contains_key("config-only"));
}
#[test]
fn empty_list_after_a_real_list_does_not_bring_dropped_tables_back() {
    let cfg = external_auth_config("[model.config-only]\nmodel = \"config-only\"\n");
    let mgr = cold_manager(cfg.clone(), Arc::new(FailingEndpoint));
    assert!(mgr.apply_refresh_result(&cfg, Some(make_prefetched(&["proxy-a"])), None));
    assert_eq!(BTreeSet::from(["proxy-a"]), catalog_ids(&mgr.models()));
    assert!(mgr.apply_refresh_result(&cfg, Some(IndexMap::new()), None));
    assert!(mgr.models().is_empty());
}
#[test]
fn table_keyed_by_a_listed_id_does_not_change_the_listed_model() {
    let cfg = external_auth_config("[model.proxy-a]\nmodel = \"unlisted-model\"\n");
    let catalog = resolve_model_catalog(&cfg, Some(make_prefetched(&["proxy-a", "proxy-b"])));
    assert_eq!(
        BTreeSet::from(["proxy-a", "proxy-b"]),
        catalog_ids(&catalog)
    );
    assert_eq!(
        Some("proxy-a"),
        catalog.get("proxy-a").map(|e| e.info.model.as_str())
    );
}
#[test]
fn tables_add_no_models_on_any_host() {
    let tables = |host: &str| {
        external_auth_config(&format!(
            "[model.corp-a]\nmodel = \"proxy-a\"\nbase_url = \"https://{host}/v1\"\n"
        ))
    };
    let rows = make_prefetched(&["proxy-a"]);
    let same_host = resolve_model_catalog(&tables("proxy.example.com"), Some(rows.clone()));
    let other_host = resolve_model_catalog(&tables("elsewhere.example.com"), Some(rows.clone()));
    let before_a_list = resolve_model_catalog(&tables("elsewhere.example.com"), None);
    assert_eq!(BTreeSet::from(["proxy-a"]), catalog_ids(&same_host));
    assert_eq!(BTreeSet::from(["proxy-a"]), catalog_ids(&other_host));
    assert!(before_a_list.is_empty());
}
#[test]
fn fallback_model_id_ignores_a_remote_settings_default_under_external_auth() {
    let mut cfg = external_auth_config("");
    cfg.remote_settings = Some(crate::util::config::RemoteSettings {
        default_model: Some("remote-pick".to_owned()),
        ..Default::default()
    });
    let (key, _, _) = resolve_default_model(&cfg, &IndexMap::new(), true);
    assert_eq!("", key);
}
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn empty_models_reply_is_a_failed_refresh_that_keeps_the_last_list() {
    use codel_test_support::MockInferenceServer;
    let _no_key = EnvGuard::unset("CODEL_API_KEY");
    let _no_legacy_key = EnvGuard::unset("CODEL_CODE_CODEL_API_KEY");
    let server = MockInferenceServer::start_with_models(vec![])
        .await
        .expect("mock models endpoint starts");
    let cfg = config_from_toml(&format!(
        "{EXTERNAL_AUTH}[endpoints]\nmodels_base_url = \"{}\"\n",
        server.url()
    ));
    let mgr = cold_manager(cfg.clone(), Arc::new(FailingEndpoint));
    assert!(mgr.apply_refresh_result(&cfg, Some(make_prefetched(&["proxy-a"])), None));
    let endpoints = cfg.endpoints.clone();
    let auth = CodelAuth {
        key: "external-provider-token".to_owned(),
        auth_mode: codel_login::AuthMode::External,
        ..CodelAuth::default()
    };
    let reply = tokio::task::spawn_blocking(move || {
        match super::super::fetch_models_uncommitted(
            &endpoints,
            Some(&auth),
            ModelFetchAuth::CustomEndpoint,
            true,
        ) {
            super::super::ModelsPrefetch::Unavailable => None,
            super::super::ModelsPrefetch::Cached(models) => Some(models),
            super::super::ModelsPrefetch::Fetched(write) => Some(write.into_models()),
        }
    })
    .await
    .expect("fetch task joins");
    let applied = mgr.apply_refresh_result(&cfg, reply, None);
    assert_eq!(1, server.request_count_for("/v1/models"));
    assert!(!applied);
    assert_eq!(BTreeSet::from(["proxy-a"]), catalog_ids(&mgr.models()));
}
#[test]
fn sampling_config_never_falls_back_to_a_bundled_model_under_external_auth() {
    let manager = |cfg: config::Config, current: &str| {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let auth_manager = Arc::new(AuthManager::new(tmp.path(), CodelComConfig::default()));
        ModelsManagerBuilder::new(
            None,
            IndexMap::new(),
            acp::ModelId::new(current),
            auth_manager,
            cfg,
        )
        .cache(test_cache_manager(tmp.path()))
        .build()
    };
    let external = manager(external_auth_config(""), "corp-build-model");
    let standard = manager(config::Config::default(), "");
    assert_eq!("corp-build-model", external.sampling_config().model);
    assert_eq!(
        crate::models::default_model(),
        standard.sampling_config().model
    );
}
#[tokio::test]
async fn prompt_gate_waits_for_the_first_fetch_under_external_auth() {
    let cfg = external_auth_config(
        "[models]\nallowed_models = [\"config-only\"]\n[model.config-only]\nmodel = \"config-only\"\n",
    );
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let auth_manager = Arc::new(AuthManager::new(tmp.path(), CodelComConfig::default()));
    let mgr = ModelsManagerBuilder::new(
        None,
        resolve_model_catalog(&cfg, None),
        acp::ModelId::new("config-only"),
        auth_manager,
        cfg.clone(),
    )
    .endpoint(Arc::new(SlowEndpoint {
        catalog: make_prefetched(&["proxy-a"]),
        delay: std::time::Duration::from_millis(50),
    }))
    .cache(test_cache_manager(tmp.path()))
    .build();
    mgr.spawn_background_refresh_inner(true);
    let blocked = mgr.models_endpoint_block_message(|| true).await;
    assert_eq!(None, blocked);
    assert_eq!(BTreeSet::from(["proxy-a"]), catalog_ids(&mgr.models()));
}
const MANAGED_PROXY: &str = "https://proxy.example.com/v1";
/// An external-auth managed config shaped like a real customer's, trimmed to the model and auth keys.
/// Its one table sends `model` to the proxy.
fn managed_proxy_config(model: &str) -> config::Config {
    config_from_toml(&format!(
        r#"
        [auth]
        auth_provider_command = "/usr/local/bin/my-auth-provider"
        auth_provider_label = "Corp"

        [models]
        default = "corp-build-model"
        allowed_models = ["corp-build-model"]
        default_reasoning_effort = "medium"

        [endpoints]
        models_base_url = "{MANAGED_PROXY}"

        [model.corp-build-model]
        model = "{model}"
        base_url = "{MANAGED_PROXY}/"
        name = "Corp Build Model"
        context_window = 500000
        supports_reasoning_effort = true

        [features]
        remote_fetch = false
        "#
    ))
}
/// `/models` rows shaped like that customer's. The shared parser ignores their `aliases` and `context_length`.
fn managed_proxy_rows() -> IndexMap<String, ModelEntry> {
    let row = |id: &str, aliases: &[&str]| {
        serde_json::json!({
            "id": id,
            "aliases": aliases,
            "context_length": 512_000,
            "object": "model",
            "owned_by": "codel",
            "created": 1_758_000_000,
            "prompt_text_token_price": 20_000,
            "completion_text_token_price": 150_000,
        })
    };
    let rows = [
        row("model-large", &["model-latest", "build-alias"]),
        row("model-large-alt", &["model-previous-alt"]),
        row("model-small", &[]),
    ];
    build_prefetched_map(
        rows.iter()
            .filter_map(|row| crate::remote::client::parse_remote_model_value(row, MANAGED_PROXY))
            .collect(),
        None,
    )
}
#[test]
fn managed_tables_are_ignored_without_a_fetched_list() {
    let cfg = managed_proxy_config("build-alias");
    let catalog = resolve_model_catalog(&cfg, None);
    assert!(cfg.config_models.is_empty());
    assert!(catalog.is_empty());
}
#[test]
fn managed_table_allowlist_and_default_yield_to_the_endpoint_list() {
    let by_id = managed_proxy_config("model-large");
    let by_alias = managed_proxy_config("build-alias");
    let by_id_catalog = resolve_model_catalog(&by_id, Some(managed_proxy_rows()));
    let by_alias_catalog = resolve_model_catalog(&by_alias, Some(managed_proxy_rows()));
    assert_eq!(
        BTreeSet::from(["model-large", "model-large-alt", "model-small"]),
        catalog_ids(&by_id_catalog)
    );
    assert_eq!(catalog_ids(&by_id_catalog), catalog_ids(&by_alias_catalog));
    for (cfg, catalog) in [(&by_id, &by_id_catalog), (&by_alias, &by_alias_catalog)] {
        assert!(!allowlist_matches_nothing(cfg, catalog));
        assert_eq!(Ok(()), validate_selectable(cfg, catalog));
        assert_eq!("model-large", resolve_default_model(cfg, catalog, false).0);
    }
}
#[test]
fn external_auth_without_models_base_url_keeps_the_standard_catalog() {
    let cfg = config_from_toml(EXTERNAL_AUTH);
    let catalog = resolve_model_catalog(&cfg, None);
    assert!(catalog.contains_key(crate::models::default_model()));
}
#[test]
fn models_endpoint_default_never_names_a_bundled_model() {
    let empty = IndexMap::new();
    let (configured, _, source) = resolve_default_model(
        &external_auth_config("[models]\ndefault = \"proxy-default\"\n"),
        &empty,
        true,
    );
    let (unconfigured, _, _) = resolve_default_model(&external_auth_config(""), &empty, true);
    let (standard, _, _) = resolve_default_model(&config::Config::default(), &empty, true);
    assert_eq!(
        ("proxy-default", config::ConfigSource::Default),
        (configured.as_str(), source)
    );
    assert_eq!("", unconfigured);
    assert_eq!(crate::models::default_model(), standard);
}
#[test]
fn models_endpoint_default_missing_from_the_list_picks_the_first_listed_model() {
    let cfg = external_auth_config(
        "[models]\ndefault = \"not-listed\"\n[model.not-listed]\nmodel = \"not-listed\"\n",
    );
    let catalog = resolve_model_catalog(&cfg, Some(make_prefetched(&["proxy-a", "proxy-b"])));
    let (key, _, source) = resolve_default_model(&cfg, &catalog, true);
    assert_eq!(
        ("proxy-a", config::ConfigSource::Default),
        (key.as_str(), source)
    );
}
#[test]
fn allowed_models_is_ignored_under_external_auth() {
    let tables = "[models]\nallowed_models = [\"config-only\"]\n[model.config-only]\nmodel = \"config-only\"\n";
    let listed = make_prefetched(&["proxy-a"]);
    let external = external_auth_config(tables);
    let standard = config_from_toml(&format!("{PROXY_MODELS_BASE_URL}{tables}"));
    let external_catalog = resolve_model_catalog(&external, Some(listed.clone()));
    let standard_catalog = resolve_model_catalog(&standard, Some(listed));
    assert_eq!(None, external.models.allowed_models);
    assert!(!allowlist_matches_nothing(&external, &external_catalog));
    assert_eq!(Ok(()), validate_selectable(&external, &external_catalog));
    assert_eq!(
        Some(vec!["config-only".to_owned()]),
        standard.models.allowed_models
    );
    assert!(!allowlist_matches_nothing(&standard, &standard_catalog));
}
#[tokio::test]
async fn failed_models_endpoint_fetch_blocks_prompts_with_the_endpoint_url() {
    let external = cold_manager(external_auth_config(""), Arc::new(FailingEndpoint));
    let standard = cold_manager(
        config_from_toml(PROXY_MODELS_BASE_URL),
        Arc::new(FailingEndpoint),
    );
    external.fetch_and_apply_inner(true).await;
    standard.fetch_and_apply_inner(true).await;
    assert_eq!(
        Some(
            "No models are available: https://proxy.example.com/v1/models returned none or could not be reached. \
             Check the endpoint and your login, then try again."
        ),
        external
            .models_endpoint_block_message(|| true)
            .await
            .as_deref()
    );
    assert_eq!(
        Some(
            "No models are available: `[features] remote_fetch = false` stops Codel from reading \
             https://proxy.example.com/v1/models. Add a `[model.<id>]` table that names an endpoint id, \
             or turn remote_fetch on."
        ),
        external
            .models_endpoint_block_message(|| false)
            .await
            .as_deref()
    );
    assert_eq!(None, standard.prompt_block_message().await);
}
#[tokio::test]
async fn failed_models_endpoint_refresh_keeps_the_listed_models_and_prompts_unblocked() {
    let cfg = external_auth_config("");
    let mgr = cold_manager(cfg.clone(), Arc::new(FailingEndpoint));
    assert!(mgr.apply_refresh_result(&cfg, Some(make_prefetched(&["proxy-a"])), None));
    mgr.fetch_and_apply_inner(true).await;
    assert_eq!(
        vec![acp::ModelId::new("proxy-a")],
        mgr.available().into_keys().collect::<Vec<_>>()
    );
    assert_eq!(None, mgr.models_endpoint_block_message(|| true).await);
}
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn picker_lists_exactly_the_mocked_models_endpoint_under_external_auth() {
    use crate::remote::ModelSource;
    use codel_test_support::{MockInferenceServer, MockModelEntry};
    let _no_key = EnvGuard::unset("CODEL_API_KEY");
    let _no_legacy_key = EnvGuard::unset("CODEL_CODE_CODEL_API_KEY");
    let server = MockInferenceServer::start_with_models(vec![
        MockModelEntry::new("proxy-a"),
        MockModelEntry::new("proxy-b"),
    ])
    .await
    .expect("mock models endpoint starts");
    let endpoint_and_table = format!(
        "[endpoints]\nmodels_base_url = \"{}\"\n[model.config-only]\nmodel = \"config-only\"\n",
        server.url()
    );
    let external = config_from_toml(&format!("{EXTERNAL_AUTH}{endpoint_and_table}"));
    let standard = config_from_toml(&endpoint_and_table);
    let endpoints = external.endpoints.clone();
    let auth = CodelAuth {
        key: "external-provider-token".to_owned(),
        auth_mode: codel_login::AuthMode::External,
        ..CodelAuth::default()
    };
    let fetched = tokio::task::spawn_blocking(move || {
        crate::remote::active_model_source(&endpoints, ModelFetchAuth::CustomEndpoint)
            .fetch(Some(&auth))
    })
    .await
    .expect("fetch task joins")
    .expect("mocked models endpoint answers");
    let listed = build_prefetched_map(fetched.models, None);
    let picker = |cfg: &config::Config| {
        let home = tempfile::TempDir::new().expect("tempdir");
        let auth_manager = Arc::new(AuthManager::new(home.path(), CodelComConfig::default()));
        ModelsManager::from_config(cfg, Some(listed.clone()), auth_manager)
            .expect("catalog resolves")
            .available()
            .into_keys()
            .map(|id| id.0.to_string())
            .collect::<BTreeSet<_>>()
    };
    assert_eq!(1, server.request_count_for("/v1/models"));
    assert_eq!(
        vec![Some("Bearer external-provider-token".to_owned())],
        server
            .requests()
            .into_iter()
            .filter(|entry| entry.path == "/v1/models")
            .map(|entry| entry.authorization)
            .collect::<Vec<_>>()
    );
    assert_eq!(
        BTreeSet::from(["proxy-a".to_owned(), "proxy-b".to_owned()]),
        picker(&external)
    );
    assert_eq!(
        BTreeSet::from([
            "config-only".to_owned(),
            "proxy-a".to_owned(),
            "proxy-b".to_owned()
        ]),
        picker(&standard)
    );
}
