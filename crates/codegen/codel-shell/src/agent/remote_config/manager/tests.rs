use chrono::{Duration as ChronoDuration, Utc};

use super::super::{
    CACHE_TTL, CacheAuthMethod, Commit, MODELS_CACHE_FILE, ModelsCache, ModelsCacheScope,
    ModelsFetchFuture, SettingsCacheManager, allowlist_denied_message, build_prefetched_map,
    degraded_log_level, evaluate_models_commit, resolve_prefetch_inputs_from_parts,
    resolve_startup_endpoints, selectable_catalog_key_for_persisted,
};
use super::*;

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
        mgr.fetch_and_apply_inner(/*remote_fetch_enabled*/ true),
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

    mgr.spawn_fetch_inner(Some("etag-1".into()), /*remote_fetch_enabled*/ true);
    tokio::task::yield_now().await;
    mgr.spawn_fetch_inner(Some("etag-2".into()), /*remote_fetch_enabled*/ true);
    tokio::task::yield_now().await;
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "single-flight: only one etag fetch in flight at a time",
    );

    // Advance past the bound so the hung fetch is abandoned and the guard clears.
    tokio::time::sleep(crate::http::STARTUP_FETCH_TIMEOUT * 2).await;
    tokio::task::yield_now().await;

    mgr.spawn_fetch_inner(Some("etag-3".into()), /*remote_fetch_enabled*/ true);
    tokio::task::yield_now().await;
    assert_eq!(
        calls.load(Ordering::SeqCst),
        2,
        "after the timeout cleared the in-flight guard, a new etag fetch proceeds",
    );

    mgr.spawn_fetch_inner(Some("etag-4".into()), /*remote_fetch_enabled*/ false);
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
    mgr.spawn_fetch_inner(None, /*remote_fetch_enabled*/ true);
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
    mgr.spawn_fetch_inner(None, /*remote_fetch_enabled*/ true);
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
    mgr.fetch_and_apply_inner(/*remote_fetch_enabled*/ true)
        .await;
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
    mgr.spawn_background_refresh_inner(/*remote_fetch_enabled*/ true); // must not panic (no tokio::spawn taken)
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
    ModelsCacheManager {
        path: dir.join(MODELS_CACHE_FILE),
        ttl: CACHE_TTL,
    }
}












#[test]
#[serial]
fn api_key_scope_identity_differs_per_key() {
    let _no_legacy = EnvGuard::unset("CODEL_CODE_CODEL_API_KEY");
    let endpoints = config::EndpointsConfig::default();
    let identity_for = |key: &str| {
        let _key = EnvGuard::set("CODEL_API_KEY", key);
        ModelsCacheScope::resolve(&endpoints, ModelFetchAuth::ApiKey, None).identity
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
        ModelsCacheScope::resolve(&endpoints, ModelFetchAuth::CustomEndpoint, None).identity
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
fn custom_endpoint_session_scope_keys_on_account_not_bearer() {
    let _no_key = EnvGuard::unset("CODEL_API_KEY");
    let _no_legacy = EnvGuard::unset("CODEL_CODE_CODEL_API_KEY");
    let endpoints = config::EndpointsConfig::default();
    let identity_for = |user_id: &str, key: &str| {
        let auth = CodelAuth {
            user_id: user_id.to_string(),
            key: key.to_string(),
            ..CodelAuth::test_default()
        };
        ModelsCacheScope::resolve(&endpoints, ModelFetchAuth::CustomEndpoint, Some(&auth)).identity
    };
    assert_eq!(
        identity_for("account-a", "token-1"),
        identity_for("account-a", "token-2"),
        "a token refresh must keep the same account's custom-endpoint scope",
    );
    assert_ne!(
        identity_for("account-a", "token-1"),
        identity_for("account-b", "token-1"),
        "different accounts must not share a custom-endpoint catalog",
    );
}

#[test]
fn models_commit_gate_detects_account_switch() {
    let scope_for = |user_id: &str| ModelsCacheScope {
        auth_method: CacheAuthMethod::Session,
        origin: "https://origin.example/v1/models".to_string(),
        identity: SettingsCacheManager::identity(
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
            evaluate_models_commit(&expected, &scope_for("account-a")),
            Commit::CacheAndServe
        ),
        "the same account (token rotation) must still cache",
    );
    assert!(
        matches!(
            evaluate_models_commit(&expected, &scope_for("account-b")),
            Commit::ServeInMemory
        ),
        "a mid-fetch account switch must serve in memory, not cache under the old account",
    );
    let moved = ModelsCacheScope {
        origin: "https://other.example/v1/models".to_string(),
        ..expected.clone()
    };
    assert!(matches!(
        evaluate_models_commit(&expected, &moved),
        Commit::Abandon
    ));
}

#[test]
#[serial]
fn resolve_live_keeps_fetch_origin_when_disk_auth_absent() {
    let _no_legacy = EnvGuard::unset("CODEL_CODE_CODEL_API_KEY");
    // Session fetch, then disk auth is gone at commit while CODEL_API_KEY is set
    // (the just-logged-in / sign-out window). The live scope must stay on the
    // fetch-time Session origin so a good catalog is served, not abandoned.
    let _key = EnvGuard::set("CODEL_API_KEY", "boot-window-key");
    let session_auth = CodelAuth {
        user_id: "session-user".to_string(),
        ..CodelAuth::test_default()
    };
    let endpoints = resolve_startup_endpoints();
    let expected =
        ModelsCacheScope::resolve(&endpoints, ModelFetchAuth::Session, Some(&session_auth));
    let live = ModelsCacheScope::resolve_live(ModelFetchAuth::Session, None);
    assert_eq!(
        live.origin, expected.origin,
        "the live mode must not flip to the api-key origin",
    );
    assert!(
        matches!(
            evaluate_models_commit(&expected, &live),
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
    // A hung identity provider (a never-ready auth future) must degrade to None within the bound so a cold-cache boot fetch can't stall on it
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
    use crate::managed_config::LaunchProfile;
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
