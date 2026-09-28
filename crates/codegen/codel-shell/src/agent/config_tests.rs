use super::*;
use serial_test::serial;
use codel_test_support::EnvGuard;
#[test]
fn main_cli_tools_override_preserves_profile_injection_policy() {
    let overrides = CliAgentOverrides {
        tools: Some(vec!["read_file".into()]),
        ..Default::default()
    };
    let mut cases = vec![(AgentDefinition::default_codel_build(), true)];
    for (mut definition, expected_injection) in cases {
        overrides.apply_to_definition(&mut definition);
        assert_eq!(definition.tools, vec!["read_file".to_string()]);
        assert_eq!(definition.inject_default_tools, expected_injection);
    }
}
/// `AutoModeConfig` parses identically from a local `[auto_mode]` TOML table and an equivalent remote settings JSON object.
/// Serde is format-agnostic.
/// The lean shape is all scalars/enums, so no custom tolerant deserializer is needed.
#[test]
fn auto_mode_config_parses_from_toml_and_json_equivalently() {
    use codel_workspace::permission::ClassifierPromptType;
    let toml_src = r#"
enabled = true
prompt_type = "no_user_tool_prefix"
classifier_model = "codel-4.5"
classify_timeout_ms = 45000
reasoning_effort = "low"
"#;
    let from_toml: AutoModeConfig = toml::from_str(toml_src).unwrap();
    let json = serde_json::json!({
        "enabled": true,
        "prompt_type": "no_user_tool_prefix",
        "classifier_model": "codel-4.5",
        "classify_timeout_ms": 45000,
        "reasoning_effort": "low"
    });
    let from_json: AutoModeConfig = serde_json::from_value(json).unwrap();
    assert_eq!(
        serde_json::to_value(&from_toml).unwrap(),
        serde_json::to_value(&from_json).unwrap()
    );
    for cfg in [&from_toml, &from_json] {
        assert_eq!(cfg.enabled, Some(true));
        assert_eq!(
            cfg.prompt_type,
            Some(ClassifierPromptType::NoUserToolPrefix)
        );
        assert_eq!(cfg.classifier_model.as_deref(), Some("codel-4.5"));
        assert_eq!(cfg.classify_timeout_ms, Some(45_000));
        assert_eq!(cfg.reasoning_effort, Some(ReasoningEffort::Low));
    }
    let empty: AutoModeConfig = toml::from_str("").unwrap();
    assert_eq!(serde_json::to_value(&empty).unwrap(), serde_json::json!({}));
}
/// `prompt_type` wire values are the snake_case `ClassifierPromptType` names.
#[test]
fn auto_mode_prompt_type_parses_snake_case() {
    use codel_workspace::permission::ClassifierPromptType;
    for (s, variant) in [
        ("full", ClassifierPromptType::Full),
        (
            "no_user_tool_prefix",
            ClassifierPromptType::NoUserToolPrefix,
        ),
        ("bare_instructions", ClassifierPromptType::BareInstructions),
        ("just_command", ClassifierPromptType::JustCommand),
    ] {
        let cfg: AutoModeConfig = toml::from_str(&format!("prompt_type = \"{s}\"")).unwrap();
        assert_eq!(cfg.prompt_type, Some(variant));
    }
}
#[test]
fn laziness_detector_default_is_all_disabled() {
    let cfg = LazinessDetectorPerModelConfig::default();
    assert!(!cfg.enabled);
    assert_eq!(cfg.max_nudges_per_session, 0);
    assert_eq!(cfg.idle_threshold_ms, None);
    assert_eq!(cfg.min_confidence, None);
    assert_eq!(
        cfg.include_reasoning, None,
        "include_reasoning defaults to None so the harness default applies",
    );
}
#[test]
fn laziness_detector_absent_block_deserializes_to_default() {
    let json = serde_json::json!({
        "model": "test",
        "base_url": "https://test.api/v1",
        "context_window": 200_000,
    });
    let entry: ModelEntryConfig =
        serde_json::from_value(json).expect("ModelEntryConfig deserializes without detector");
    assert_eq!(
        entry.laziness_detector,
        LazinessDetectorPerModelConfig::default()
    );
    let info = ModelInfo::from_config(&entry);
    assert!(!info.laziness_detector.enabled);
}
#[test]
fn laziness_detector_fallback_modelinfo_is_disabled() {
    let info = ModelInfo::fallback("unknown-model");
    assert_eq!(
        info.laziness_detector,
        LazinessDetectorPerModelConfig::default(),
    );
    assert!(!info.laziness_detector.enabled);
    assert_eq!(info.laziness_detector.max_nudges_per_session, 0);
}
#[test]
fn laziness_detector_block_round_trips_through_serde() {
    let json = serde_json::json!({
        "enabled": true,
        "max_nudges_per_session": 3,
        "idle_threshold_ms": 15_000,
        "min_confidence": 0.8,
        "include_reasoning": false,
    });
    let cfg: LazinessDetectorPerModelConfig =
        serde_json::from_value(json).expect("deserialize populated block");
    assert!(cfg.enabled);
    assert_eq!(cfg.max_nudges_per_session, 3);
    assert_eq!(cfg.idle_threshold_ms, Some(15_000));
    assert_eq!(cfg.min_confidence, Some(0.8));
    assert_eq!(cfg.include_reasoning, Some(false));
}
/// Pins all three states of the per-model `include_reasoning` override (`Some(true)`, `Some(false)`, and absent giving `None`).
/// A future drift on the `#[serde(default)]` attribute or the field type fails the test rather than silently changing the resolved default.
#[test]
fn laziness_detector_include_reasoning_serde_states() {
    let some_true: LazinessDetectorPerModelConfig =
        serde_json::from_value(serde_json::json!({ "include_reasoning": true }))
            .expect("Some(true)");
    assert_eq!(some_true.include_reasoning, Some(true));
    let some_false: LazinessDetectorPerModelConfig =
        serde_json::from_value(serde_json::json!({ "include_reasoning": false }))
            .expect("Some(false)");
    assert_eq!(some_false.include_reasoning, Some(false));
    let absent: LazinessDetectorPerModelConfig =
        serde_json::from_value(serde_json::json!({})).expect("absent → None");
    assert_eq!(absent.include_reasoning, None);
}
#[test]
fn subagent_permission_mode_precedence() {
    let own = PermissionMode::Plan;
    let cases = [
        (
            PermissionMode::BypassPermissions,
            PermissionMode::BypassPermissions,
        ),
        (PermissionMode::AcceptEdits, PermissionMode::AcceptEdits),
        (PermissionMode::Auto, PermissionMode::Auto),
        (PermissionMode::Default, own.clone()),
        (PermissionMode::DontAsk, own.clone()),
        (PermissionMode::Plan, own.clone()),
    ];
    for (parent, expected) in cases {
        assert_eq!(
            resolve_subagent_permission_mode(own.clone(), &parent),
            expected,
            "parent={parent:?}"
        );
    }
}
#[test]
fn inject_url_derived_headers_adds_proxy_headers_for_cli_chat_proxy_url() {
    let mut headers = IndexMap::new();
    inject_url_derived_headers(&mut headers, None, crate::env::PROD_CLI_CHAT_PROXY_BASE_URL);
    assert_eq!(
        headers.get("X-CODEL-Token-Auth").map(String::as_str),
        Some("codel-cli")
    );
    assert_eq!(
        headers.get("x-authenticateresponse").map(String::as_str),
        Some("authenticate-response")
    );
    assert_eq!(
        headers
            .get(crate::http::CLIENT_MODE_HEADER)
            .map(String::as_str),
        Some(crate::http::process_client_mode())
    );
}
#[test]
fn inject_url_derived_headers_skips_proxy_headers_for_external_url() {
    let mut headers = IndexMap::new();
    inject_url_derived_headers(&mut headers, None, "https://api.codel.dev/v1");
    assert!(headers.get("X-CODEL-Token-Auth").is_none());
    assert!(headers.get("x-authenticateresponse").is_none());
    assert_eq!(
        headers
            .get(crate::http::CLIENT_MODE_HEADER)
            .map(String::as_str),
        Some(crate::http::process_client_mode())
    );
}
#[test]
fn inject_url_derived_headers_preserves_caller_extra_headers() {
    let mut headers = IndexMap::new();
    headers.insert("x-custom-byok".to_string(), "value".to_string());
    inject_url_derived_headers(&mut headers, None, crate::env::PROD_CLI_CHAT_PROXY_BASE_URL);
    assert_eq!(
        headers.get("x-custom-byok").map(String::as_str),
        Some("value")
    );
    assert_eq!(
        headers.get("X-CODEL-Token-Auth").map(String::as_str),
        Some("codel-cli")
    );
}
#[test]
fn inject_url_derived_headers_does_not_overwrite_existing_entries() {
    let mut headers = IndexMap::new();
    headers.insert("X-CODEL-Token-Auth".to_string(), "caller-set".to_string());
    inject_url_derived_headers(&mut headers, None, crate::env::PROD_CLI_CHAT_PROXY_BASE_URL);
    assert_eq!(
        headers.get("X-CODEL-Token-Auth").map(String::as_str),
        Some("caller-set"),
    );
}
#[test]
fn parses_toolset_overrides() {
    let raw_config: toml::Value = toml::from_str(
        r#"
            [toolset.bash]
            timeout_secs = 123

            [toolset.ask_user_question]
            timeout_enabled = false
            timeout_secs = 30
            "#,
    )
    .unwrap();
    let cfg = Config::new_from_toml_cfg(&raw_config).expect("config should parse");
    assert_eq!(cfg.toolset.bash.timeout_secs, Some(123.0));
    assert_eq!(cfg.toolset.ask_user_question.timeout_enabled, Some(false));
    assert_eq!(cfg.toolset.ask_user_question.timeout_secs, Some(30));
}
#[test]
fn parses_cursor_worker_table_without_unrecognized_keys() {
    let raw_config: toml::Value = toml::from_str(
        r#"
            [cursor_worker]
            auto_start = true
            name = "devbox"
            worker_dirs = ["/srv/a", "/srv/b"]
            max_agents = 2
            any_repo = false
            hub_url = "wss://hub.example/v1/tools"
            rewrite_shell_output = true
            "#,
    )
    .unwrap();
    let cfg = Config::new_from_toml_cfg(&raw_config).expect("config should parse");
    assert_eq!(
        CursorWorkerConfig {
            auto_start: true,
            name: Some("devbox".to_owned()),
            worker_dirs: vec!["/srv/a".to_owned(), "/srv/b".to_owned()],
            max_agents: Some(2),
            any_repo: Some(false),
            hub_url: Some("wss://hub.example/v1/tools".to_owned()),
            rewrite_shell_output: true,
        },
        cfg.cursor_worker
    );
    assert!(
        cfg.config_warnings.is_empty(),
        "every key is a declared field: {:?}",
        cfg.config_warnings
    );
    let empty: toml::Value = toml::from_str("").unwrap();
    let cfg = Config::new_from_toml_cfg(&empty).expect("config should parse");
    assert_eq!(CursorWorkerConfig::default(), cfg.cursor_worker);
}
#[test]
fn parses_toolset_bash_float_timeout() {
    let raw_config: toml::Value = toml::from_str(
        r#"
            [toolset.bash]
            timeout_secs = 30.5
            "#,
    )
    .unwrap();
    let cfg = Config::new_from_toml_cfg(&raw_config).expect("config should parse");
    assert_eq!(cfg.toolset.bash.timeout_secs, Some(30.5));
}
#[test]
fn resolve_runtime_fields_propagates_disable_zdr_incompatible_tools() {
    fn ctx(raw: &toml::Value) -> RuntimeResolutionContext<'_> {
        RuntimeResolutionContext {
            raw_config: raw,
            remote_settings: None,
            is_headless: false,
            cli_subagents: None,
            cli_web_search_model: None,
            cli_session_summary_model: None,
            memory_enabled_override: None,
            disable_web_search: false,
            todo_gate: false,
            laziness_debug_log: None,
            storage_mode: None,
        }
    }
    let empty: toml::Value = toml::Value::Table(toml::map::Map::new());
    let mut cfg = Config::new_from_toml_cfg(&empty).unwrap();
    cfg.resolve_runtime_fields(&ctx(&empty));
    assert!(!cfg.disable_zdr_incompatible_tools);
    let zdr: toml::Value =
        toml::from_str("[tools]\ndisable_zdr_incompatible_tools = true").unwrap();
    let mut cfg = Config::new_from_toml_cfg(&zdr).unwrap();
    cfg.resolve_runtime_fields(&ctx(&zdr));
    assert!(cfg.disable_zdr_incompatible_tools);
}
#[test]
fn re_resolve_runtime_fields_refreshes_typed_memory_from_raw_config() {
    let initial: toml::Value = toml::from_str(
            "[memory]\nenabled = true\n[memory.search]\nmax_results = 6\n[memory_v2]\nenabled = false\ncapture_status_enabled = false",
        )
        .unwrap();
    let updated: toml::Value = toml::from_str(
            "[memory]\nenabled = true\n[memory.search]\nmax_results = 12\n[memory_v2]\nenabled = true\ncapture_status_enabled = true",
        )
        .unwrap();
    let mut cfg = Config::new_from_toml_cfg(&initial).unwrap();
    cfg.memory_enabled_override = Some(true);
    cfg.re_resolve_runtime_fields(&updated);
    let memory = cfg.memory_config.unwrap();
    assert_eq!(memory.search.max_results, 12);
    assert_eq!(memory.mode, crate::config::MemoryMode::V2);
    assert!(memory.v2.capture_status_enabled);
}
#[test]
fn resolved_memory_config_uses_isolated_v2_namespace() {
    let raw: toml::Value =
        toml::from_str("[memory]\nenabled = true\n[memory_v2]\nenabled = true").unwrap();
    let mut cfg = Config::new_from_toml_cfg(&raw).unwrap();
    cfg.re_resolve_runtime_fields(&raw);
    let memory = cfg.memory_config.expect("resolved config is retained");
    assert!(memory.enabled);
    assert_eq!(memory.mode, crate::config::MemoryMode::V2);
}
#[test]
fn resolve_runtime_fields_propagates_disable_web_search() {
    fn ctx(raw: &toml::Value, disable_web_search: bool) -> RuntimeResolutionContext<'_> {
        RuntimeResolutionContext {
            raw_config: raw,
            remote_settings: None,
            is_headless: true,
            cli_subagents: None,
            cli_web_search_model: None,
            cli_session_summary_model: None,
            memory_enabled_override: None,
            disable_web_search,
            todo_gate: false,
            laziness_debug_log: None,
            storage_mode: None,
        }
    }
    let empty: toml::Value = toml::Value::Table(toml::map::Map::new());
    let mut cfg = Config::new_from_toml_cfg(&empty).unwrap();
    cfg.resolve_runtime_fields(&ctx(&empty, false));
    assert!(!cfg.disable_web_search);
    let mut cfg = Config::new_from_toml_cfg(&empty).unwrap();
    cfg.resolve_runtime_fields(&ctx(&empty, true));
    assert!(cfg.disable_web_search);
    let toml_on: toml::Value = toml::from_str("disable_web_search = true").unwrap();
    let mut cfg = Config::new_from_toml_cfg(&toml_on).unwrap();
    cfg.resolve_runtime_fields(&ctx(&toml_on, false));
    assert!(cfg.disable_web_search);
}
#[test]
fn new_from_toml_cfg_restores_web_search_and_session_summary_models() {
    let empty: toml::Value = toml::Value::Table(toml::map::Map::new());
    let cfg = Config::new_from_toml_cfg(&empty).expect("empty config should parse");
    assert_eq!(
        cfg.web_search_model,
        crate::models::default_web_search_model(),
        "empty config should produce the compiled-in default web_search model"
    );
    assert_eq!(
        cfg.session_summary_model,
        Some(crate::models::default_session_summary_model().to_owned()),
        "empty config should produce compiled default session_summary model"
    );
    assert_eq!(
        cfg.image_description_model,
        Some(crate::models::default_image_description_model().to_owned()),
        "empty config should produce compiled default image_description model"
    );
    let with_overrides: toml::Value = toml::from_str(
        r#"
            [models]
            web_search = "custom-ws-model"
            session_summary = "custom-ss-model"
            image_description = "custom-id-model"
            "#,
    )
    .unwrap();
    let cfg2 = Config::new_from_toml_cfg(&with_overrides).expect("config should parse");
    assert_eq!(cfg2.web_search_model, "custom-ws-model");
    assert_eq!(
        cfg2.session_summary_model,
        Some("custom-ss-model".to_owned())
    );
    assert_eq!(
        cfg2.image_description_model,
        Some("custom-id-model".to_owned())
    );
}
#[test]
fn hidden_default_web_search_resolution_is_explicit_and_responses_only() {
    let endpoints = EndpointsConfig::default();
    let resolved = resolve_web_search_sampling_config(
        crate::models::default_web_search_model(),
        &IndexMap::new(),
        Some("session-token"),
        false,
        None,
        None,
        &endpoints,
    )
    .expect("hidden default web search model should resolve");
    assert_eq!(resolved.model, crate::models::default_web_search_model());
    assert_eq!(resolved.base_url, endpoints.proxy_url());
    assert_eq!(resolved.api_backend, ApiBackend::Responses);
    assert_eq!(
        resolved.api_key.as_deref(),
        Some("session-token"),
        "hidden default should still use normal credential resolution"
    );
}
#[test]
fn finalize_image_describe_sampler_none_uses_active_session_model_not_forced_helper() {
    let active = SamplerConfig {
        model: "composer-session-model".into(),
        ..Default::default()
    };
    let (model, cfg) = finalize_image_describe_sampler_config(None, &active, None, Some(3));
    assert_eq!(model, "composer-session-model");
    assert_eq!(cfg.model, "composer-session-model");
    assert_ne!(cfg.model, "codel-build");
}
#[test]
fn finalize_image_describe_sampler_some_stamps_session_fields() {
    let active = SamplerConfig {
        model: "composer-session-model".into(),
        ..Default::default()
    };
    let aux = SamplerConfig {
        model: "codel-build".into(),
        ..Default::default()
    };
    let (model, cfg) =
        finalize_image_describe_sampler_config(Some(aux), &active, Some("cli".into()), Some(7));
    assert_eq!(model, "codel-build");
    assert_eq!(cfg.model, "codel-build");
    assert_eq!(cfg.client_identifier.as_deref(), Some("cli"));
    assert_eq!(cfg.max_retries, Some(7));
}
/// The session bearer resolver must never be stamped onto a third-party sampler: the sampler substitutes the resolver's bearer at request time.
#[test]
fn session_resolver_is_not_stamped_onto_third_party_samplers() {
    #[derive(Debug)]
    struct SessionResolver;
    impl codel_sampler::BearerResolver for SessionResolver {
        fn current_bearer(&self) -> Option<String> {
            Some("session-jwt".into())
        }
    }
    let session_cfg = SamplerConfig {
        bearer_resolver: Some(std::sync::Arc::new(SessionResolver)),
        conversation_group_id: Some("root-group".into()),
        ..SamplerConfig::default()
    };
    let mut third_party = SamplerConfig {
        base_url: "https://litellm.corp.example/v1".into(),
        ..SamplerConfig::default()
    };
    stamp_session_local_sampler_fields(&mut third_party, &session_cfg, None, None);
    assert!(
        third_party.bearer_resolver.is_none(),
        "a third-party endpoint must keep its resolved credential"
    );
    assert_eq!(
        third_party
            .conversation_group_id
            .as_ref()
            .map(|id| id.as_ref()),
        Some("root-group")
    );
    let mut first_party = SamplerConfig {
        base_url: EndpointsConfig::default().resolve_inference_base_url(),
        ..SamplerConfig::default()
    };
    stamp_session_local_sampler_fields(&mut first_party, &session_cfg, None, None);
    assert!(
        first_party.bearer_resolver.is_some(),
        "first-party aux samplers keep the session refresh behavior"
    );
    assert_eq!(
        first_party
            .conversation_group_id
            .as_ref()
            .map(|id| id.as_ref()),
        Some("root-group")
    );
}
/// Bad `[mcp_servers.*]` entries are dropped, not fatal.
#[test]
fn invalid_mcp_server_stub_does_not_fail_config_load() {
    let raw_config: toml::Value = toml::from_str(
        r#"
            [mcp_servers.github]
            enabled = false

            mcp_servers.broken = "not-a-table"

            [mcp_servers.also_broken]
            enabled = "yes"

            [mcp_servers.linear]
            command = "npx"
            args = ["-y", "mcp-remote", "https://mcp.linear.app/mcp"]
            "#,
    )
    .unwrap();
    let cfg = Config::new_from_toml_cfg(&raw_config)
        .expect("bad mcp stubs must be dropped, not fail whole config");
    assert!(
        !cfg.mcp_servers.contains_key("broken"),
        "non-table entry is dropped"
    );
    assert!(
        !cfg.mcp_servers.contains_key("also_broken"),
        "wrong-type enabled is dropped"
    );
    assert!(
        !cfg.mcp_servers.contains_key("github"),
        "transport-less stub is dropped (disable via disabled_mcp_servers)"
    );
    assert!(
        cfg.mcp_servers.contains_key("linear"),
        "valid MCP neighbor must still load"
    );
    assert!(cfg.mcp_servers.get("linear").is_some_and(|s| s.enabled));
}
#[test]
fn shell_environment_policy_typo_does_not_fail_config() {
    let cfg: toml::Value = toml::from_str(
        r#"
            [shell_environment_policy]
            inhert = "core"
            exclude = 123
            "#,
    )
    .unwrap();
    Config::new_from_toml_cfg(&cfg).expect("a policy typo must not fail the config");
}
#[test]
fn shell_environment_policy_known_keys_track_the_policy_struct() {
    let codel_tools::util::ShellEnvironmentPolicy {
        inherit: _,
        ignore_default_excludes: _,
        exclude: _,
        set: _,
        include_only: _,
    } = codel_tools::util::ShellEnvironmentPolicy::default();
    let ShellEnvironmentPolicyKnownKeys {
        inherit: _,
        ignore_default_excludes: _,
        exclude: _,
        set: _,
        include_only: _,
    } = ShellEnvironmentPolicyKnownKeys::default();
}
#[test]
fn parses_model_api_key() {
    let raw_config: toml::Value = toml::from_str(
        r#"
            [model.my-custom-model]
            model = "codel-4.5"
            base_url = "https://api.example.com/v1"
            context_window = 200000
            api_key = "sk-test-key-12345"
            "#,
    )
    .unwrap();
    let cfg = Config::new_from_toml_cfg(&raw_config).expect("config should parse");
    let resolved = resolve_model_list(&cfg, None);
    let model = resolved.get("my-custom-model").expect("model should exist");
    assert_eq!(model.info.model, "codel-4.5");
    assert_eq!(model.info.base_url, "https://api.example.com/v1");
    assert_eq!(model.api_key, Some("sk-test-key-12345".to_string()));
}
#[test]
fn default_models_dual_endpoint_routing() {
    let endpoints = EndpointsConfig::default();
    for (model_id, entry) in default_model_entries(&endpoints) {
        if entry.api_base_url.is_none() {
            continue;
        }
        let session_creds = resolve_credentials(&entry, Some("tok"));
        assert_eq!(
            session_creds.base_url,
            endpoints.proxy_url(),
            "{model_id}: SessionToken must route to cli-chat-proxy"
        );
        let api_key_creds = ResolvedCredentials {
            api_key: Some("key".into()),
            base_url: entry
                .api_base_url
                .clone()
                .unwrap_or(entry.info().base_url.clone()),
            auth_type: codel_chat_state::AuthType::ApiKey,
            auth_scheme: AuthScheme::Bearer,
        };
        assert_eq!(
            api_key_creds.base_url, endpoints.codel_api_base_url,
            "{model_id}: ExternalApiKey must route to api.codel.dev"
        );
    }
}
#[test]
fn env_keys_deser_string_or_array() {
    let one: EnvKeys = serde_json::from_str(r#""ANTHROPIC_AUTH_TOKEN""#).unwrap();
    assert_eq!(one.names(), vec!["ANTHROPIC_AUTH_TOKEN"]);
    let many: EnvKeys =
        serde_json::from_str(r#"["ANTHROPIC_AUTH_TOKEN", "LC_ANTHROPIC_AUTH_TOKEN"]"#).unwrap();
    assert_eq!(
        many.names(),
        vec!["ANTHROPIC_AUTH_TOKEN", "LC_ANTHROPIC_AUTH_TOKEN"]
    );
    let ser = serde_json::to_value(&one).unwrap();
    assert_eq!(ser, serde_json::json!("ANTHROPIC_AUTH_TOKEN"));
    let ser_many = serde_json::to_value(&many).unwrap();
    assert_eq!(
        ser_many,
        serde_json::json!(["ANTHROPIC_AUTH_TOKEN", "LC_ANTHROPIC_AUTH_TOKEN"])
    );
}
#[test]
fn env_keys_resolve_first_set_wins() {
    let keys = EnvKeys::new(["CODEL_TEST_ENV_KEY_PRIMARY", "CODEL_TEST_ENV_KEY_FALLBACK"]);
    assert_eq!(keys.resolve_value_with(|_| None), None, "none set");
    assert_eq!(
        keys.resolve_value_with(
            |n| (n == "CODEL_TEST_ENV_KEY_FALLBACK").then(|| "from-fallback".into())
        ),
        Some("from-fallback".into())
    );
    assert_eq!(
        keys.resolve_value_with(|n| match n {
            "CODEL_TEST_ENV_KEY_PRIMARY" => Some("from-primary".into()),
            "CODEL_TEST_ENV_KEY_FALLBACK" => Some("from-fallback".into()),
            _ => None,
        }),
        Some("from-primary".into()),
        "primary wins when both set"
    );
    assert_eq!(
        keys.resolve_value_with(|n| match n {
            "CODEL_TEST_ENV_KEY_PRIMARY" => Some(String::new()),
            "CODEL_TEST_ENV_KEY_FALLBACK" => Some("from-fallback".into()),
            _ => None,
        }),
        Some("from-fallback".into())
    );
}
#[test]
fn env_keys_single_and_array_are_semantically_equal() {
    let from_array: EnvKeys = serde_json::from_str(r#"["X"]"#).unwrap();
    assert_eq!(EnvKeys::new(["X"]), from_array);
    let from_string: EnvKeys = serde_json::from_str(r#""X""#).unwrap();
    assert_eq!(EnvKeys::new(["X"]), from_string);
}
#[test]
fn env_keys_resolve_skips_whitespace_only_value() {
    let keys = EnvKeys::new(["CODEL_TEST_WS_PRIMARY", "CODEL_TEST_WS_FALLBACK"]);
    assert_eq!(
        keys.resolve_value_with(|n| match n {
            "CODEL_TEST_WS_PRIMARY" => Some("   ".into()),
            "CODEL_TEST_WS_FALLBACK" => Some("real".into()),
            _ => None,
        }),
        Some("real".into())
    );
    assert_eq!(
        EnvKeys::single("CODEL_TEST_WS_ONLY").resolve_value_with(|_| Some("   ".into())),
        None
    );
    assert_eq!(
        EnvKeys::single("CODEL_TEST_WS_PAD").resolve_value_with(|_| Some("  tok  ".into())),
        Some("  tok  ".into())
    );
}
#[test]
#[serial]
fn first_own_credential_empty_api_key_falls_through_to_env_key() {
    use codel_test_support::EnvGuard;
    let var = "CODEL_TEST_FIRST_OWN_CRED_ENV";
    let _guard = EnvGuard::set(var, "env-token");
    let env_key = EnvKeys::single(var);
    assert_eq!(
        first_own_credential(Some("   "), Some(&env_key)).as_deref(),
        Some("env-token")
    );
    assert_eq!(
        first_own_credential(Some("real-key"), Some(&env_key)).as_deref(),
        Some("real-key")
    );
}
#[test]
#[serial]
fn config_toml_env_key_array_parses() {
    let dm = crate::models::default_model();
    let (_, models) = resolve_models_from_toml(
        &format!(
            r#"
            [model."{dm}"]
            model = "{dm}"
            base_url = "https://inference.example.com/v1"
            env_key = ["ANTHROPIC_AUTH_TOKEN", "LC_ANTHROPIC_AUTH_TOKEN"]
            "#,
        ),
        None,
    );
    let model = models.get(dm).expect("model should exist");
    assert_eq!(
        model.env_key.as_ref().map(|k| k.names()),
        Some(vec!["ANTHROPIC_AUTH_TOKEN", "LC_ANTHROPIC_AUTH_TOKEN"])
    );
}
fn api_key_creds(base_url: &str) -> ResolvedCredentials {
    ResolvedCredentials {
        api_key: Some("codel-secret".to_string()),
        base_url: base_url.to_string(),
        auth_type: codel_chat_state::AuthType::ApiKey,
        auth_scheme: Default::default(),
    }
}
/// `disable_api_key_auth` kill switch (Claude `forceLoginMethod` parity).
#[test]
fn enforce_disable_api_key_auth_blocks_first_party_only() {
    use codel_chat_state::AuthType;
    let mut creds = api_key_creds("https://api.codel.dev/v1");
    enforce_disable_api_key_auth(&mut creds, false, Some("session-jwt"));
    assert_eq!(creds.auth_type, AuthType::ApiKey);
    assert_eq!(creds.api_key.as_deref(), Some("codel-secret"));
    let mut creds = api_key_creds("https://api.codel.dev/v1");
    enforce_disable_api_key_auth(&mut creds, true, Some("session-jwt"));
    assert_eq!(creds.auth_type, AuthType::SessionToken);
    assert_eq!(creds.api_key.as_deref(), Some("session-jwt"));
    let mut creds = api_key_creds("https://api.codel.dev/v1");
    enforce_disable_api_key_auth(&mut creds, true, None);
    assert_eq!(creds.auth_type, AuthType::SessionToken);
    assert_eq!(creds.api_key, None);
    let mut creds = api_key_creds("https://api.example.com/v1");
    enforce_disable_api_key_auth(&mut creds, true, Some("session-jwt"));
    assert_eq!(creds.auth_type, AuthType::ApiKey);
    assert_eq!(creds.api_key.as_deref(), Some("codel-secret"));
    let mut creds = ResolvedCredentials {
        auth_type: AuthType::SessionToken,
        ..api_key_creds("https://api.codel.dev/v1")
    };
    enforce_disable_api_key_auth(&mut creds, true, Some("session-jwt"));
    assert_eq!(creds.auth_type, AuthType::SessionToken);
}
#[test]
fn config_override_applies_show_model_fingerprint() {
    let endpoints = EndpointsConfig::default();
    let override_on = ConfigModelOverride {
        show_model_fingerprint: Some(true),
        ..Default::default()
    };
    let entry = override_on.apply("some-model", None, &endpoints);
    assert!(
        entry.info.show_model_fingerprint,
        "Some(true) override should enable show_model_fingerprint"
    );
    let mut base = ModelEntry::fallback("some-model", &endpoints);
    base.info.show_model_fingerprint = true;
    let override_absent = ConfigModelOverride::default();
    let entry = override_absent.apply("some-model", Some(base), &endpoints);
    assert!(
        entry.info.show_model_fingerprint,
        "None override should preserve the base entry's show_model_fingerprint"
    );
    let mut base = ModelEntry::fallback("some-model", &endpoints);
    base.info.show_model_fingerprint = true;
    let override_off = ConfigModelOverride {
        show_model_fingerprint: Some(false),
        ..Default::default()
    };
    let entry = override_off.apply("some-model", Some(base), &endpoints);
    assert!(
        !entry.info.show_model_fingerprint,
        "Some(false) override should disable show_model_fingerprint over a true base"
    );
}
#[test]
fn user_override_parses_compaction_at_tokens_from_toml() {
    use codel_sampling_types::CompactionAtTokens;
    let dm = crate::models::default_model();
    let raw_config: toml::Value = toml::from_str(&format!(
        r#"
            [model."{dm}"]
            compaction_at_tokens = true
            "#,
    ))
    .unwrap();
    let cfg = Config::new_from_toml_cfg(&raw_config).expect("config should parse");
    let model = resolve_model_list(&cfg, None)
        .get(dm)
        .expect("model should exist")
        .clone();
    assert_eq!(
        model.info.compaction_at_tokens,
        Some(CompactionAtTokens::Enabled(true)),
    );
    let raw_config: toml::Value = toml::from_str(&format!(
        r#"
            [model."{dm}"]
            compaction_at_tokens = 367000
            "#,
    ))
    .unwrap();
    let cfg = Config::new_from_toml_cfg(&raw_config).expect("config should parse");
    let model = resolve_model_list(&cfg, None)
        .get(dm)
        .expect("model should exist")
        .clone();
    assert_eq!(
        model.info.compaction_at_tokens,
        Some(CompactionAtTokens::Fixed(367_000)),
    );
}
#[test]
fn user_override_parses_compactions_remaining_from_toml() {
    use codel_sampling_types::CompactionsRemaining;
    let dm = crate::models::default_model();
    let raw_config: toml::Value = toml::from_str(&format!(
        r#"
            [model."{dm}"]
            compactions_remaining = true
            "#,
    ))
    .unwrap();
    let cfg = Config::new_from_toml_cfg(&raw_config).expect("config should parse");
    let model = resolve_model_list(&cfg, None)
        .get(dm)
        .expect("model should exist")
        .clone();
    assert_eq!(
        model.info.compactions_remaining,
        Some(CompactionsRemaining::Dynamic(true)),
    );
    let raw_config: toml::Value = toml::from_str(&format!(
        r#"
            [model."{dm}"]
            compactions_remaining = 1
            "#,
    ))
    .unwrap();
    let cfg = Config::new_from_toml_cfg(&raw_config).expect("config should parse");
    let model = resolve_model_list(&cfg, None)
        .get(dm)
        .expect("model should exist")
        .clone();
    assert_eq!(
        model.info.compactions_remaining,
        Some(CompactionsRemaining::Fixed(1)),
    );
    let raw_config: toml::Value = toml::from_str(&format!(
        r#"
            [model."{dm}"]
            send_compactions_remaining = true
            "#,
    ))
    .unwrap();
    let cfg = Config::new_from_toml_cfg(&raw_config).expect("config should parse");
    let model = resolve_model_list(&cfg, None)
        .get(dm)
        .expect("model should exist")
        .clone();
    assert_eq!(
        model.info.compactions_remaining,
        Some(CompactionsRemaining::Dynamic(true)),
    );
}
#[test]
fn default_auto_compact_threshold_is_none() {
    let cfg = Config::default();
    assert_eq!(cfg.session.auto_compact_threshold_percent, None);
}
#[test]
fn parses_auto_compact_threshold_percent() {
    let raw_config: toml::Value = toml::from_str(
        r#"
            [session]
            auto_compact_threshold_percent = 75
            "#,
    )
    .unwrap();
    let cfg = Config::new_from_toml_cfg(&raw_config).expect("config should parse");
    assert_eq!(cfg.session.auto_compact_threshold_percent, Some(75));
}
#[test]
fn compaction_mode_precedence_env_over_config_over_remote_over_default() {
    use codel_chat_state::CompactionMode;
    assert_eq!(
        resolve_compaction_mode_from(Some("transcript"), Some("segments"), Some("summary")),
        CompactionMode::Transcript
    );
    assert_eq!(
        resolve_compaction_mode_from(None, Some("segments"), Some("summary")),
        CompactionMode::Segments(codel_chat_state::CompactionDetail::default())
    );
    assert_eq!(
        resolve_compaction_mode_from(None, None, Some("segments")),
        CompactionMode::Segments(codel_chat_state::CompactionDetail::default())
    );
    assert_eq!(
        resolve_compaction_mode_from(Some("garbage"), None, Some("segments")),
        CompactionMode::Segments(codel_chat_state::CompactionDetail::default())
    );
    assert_eq!(
        resolve_compaction_mode_from(None, None, None),
        CompactionMode::Segments(codel_chat_state::CompactionDetail::default())
    );
}
/// Detail shares the env>config>remote>default combinator that the mode test exercises.
/// The detail-specific facts are remote settings routing and the `Verbose` default (with unrecognized values falling through).
#[test]
fn compaction_detail_resolves_remote_settings_and_verbose_default() {
    use codel_chat_state::CompactionDetail;
    assert_eq!(
        resolve_compaction_detail_from(None, None, Some("minimal")),
        CompactionDetail::Minimal
    );
    assert_eq!(
        resolve_compaction_detail_from(Some("garbage"), None, None),
        CompactionDetail::Verbose
    );
}
#[test]
fn auto_compact_threshold_percent_defaults_when_not_specified() {
    let raw_config: toml::Value = toml::from_str(
        r#"
            [toolset.bash]
            timeout_secs = 123
            "#,
    )
    .unwrap();
    let cfg = Config::new_from_toml_cfg(&raw_config).expect("config should parse");
    assert_eq!(cfg.session.auto_compact_threshold_percent, None);
}
#[test]
fn parses_repo_changes_dedup_config() {
    let raw_config: toml::Value = toml::from_str(
        r#"
            [repo_changes_dedup]
            enabled = false
            include_inline_fallback = true
            max_inline_bytes = 1024
            dedup_untracked = false
            dedup_binary = false
            untracked_max_bytes = 2048
            untracked_exclude_globs = ["*.zip", "tmp/**"]
            "#,
    )
    .unwrap();
    let cfg = Config::new_from_toml_cfg(&raw_config).expect("config should parse");
    let dedup = cfg.repo_changes_dedup;
    assert!(!dedup.enabled);
    assert!(dedup.include_inline_fallback);
    assert_eq!(dedup.max_inline_bytes, 1024);
    assert!(!dedup.dedup_untracked);
    assert!(!dedup.dedup_binary);
    assert_eq!(dedup.untracked_max_bytes, 2048);
    assert_eq!(dedup.untracked_exclude_globs, vec!["*.zip", "tmp/**"]);
}
#[test]
fn parses_model_context_window() {
    let raw_config: toml::Value = toml::from_str(
        r#"
            [model.my-custom-model]
            model = "custom-llm"
            base_url = "https://api.example.com/v1"
            context_window = 256000
            "#,
    )
    .unwrap();
    let cfg = Config::new_from_toml_cfg(&raw_config).expect("config should parse");
    let resolved = resolve_model_list(&cfg, None);
    let model = resolved.get("my-custom-model").expect("model should exist");
    assert_eq!(model.info.context_window, NonZeroU64::new(256_000).unwrap());
}
#[test]
fn unset_max_request_bytes_defaults_from_api_backend() {
    let raw_config: toml::Value = toml::from_str(
        r#"
            [model.capped-messages]
            model = "claude"
            base_url = "https://api.example.com/v1"
            api_backend = "messages"
            context_window = 1000000
            max_request_bytes = 20000000

            [model.uncapped-messages]
            model = "claude"
            base_url = "https://api.example.com/v1"
            api_backend = "messages"
            context_window = 1000000

            [model.uncapped-chat]
            model = "m"
            base_url = "https://api.example.com/v1"
            api_backend = "chat_completions"
            context_window = 1000000

            [model.uncapped-responses]
            model = "m"
            base_url = "https://api.example.com/v1"
            api_backend = "responses"
            context_window = 1000000
            "#,
    )
    .unwrap();
    let cfg = Config::new_from_toml_cfg(&raw_config).expect("config should parse");
    let resolved = resolve_model_list(&cfg, None);
    let max_request_bytes = |key: &str| {
        let model = resolved.get(key).expect("model should exist");
        sampling_config_for_model(
            model,
            resolve_credentials(model, None),
            None,
            None,
            None,
            None,
        )
        .max_request_bytes
    };
    assert_eq!(
        NonZeroU64::new(20_000_000),
        max_request_bytes("capped-messages"),
        "an explicit cap overrides the backend default"
    );
    assert_eq!(
        NonZeroU64::new(30_000_000),
        max_request_bytes("uncapped-messages"),
        "a messages model budgets to the 30 MB Messages host cap"
    );
    assert_eq!(
        NonZeroU64::new(50 * 1024 * 1024),
        max_request_bytes("uncapped-chat")
    );
    assert_eq!(
        NonZeroU64::new(50 * 1024 * 1024),
        max_request_bytes("uncapped-responses")
    );
}
#[test]
fn parses_model_api_backend_responses() {
    let raw_config: toml::Value = toml::from_str(
        r#"
            [model.my-responses-model]
            model = "codel-4.5"
            base_url = "https://api.example.com/v1"
            context_window = 200000
            api_backend = "responses"
            "#,
    )
    .unwrap();
    let cfg = Config::new_from_toml_cfg(&raw_config).expect("config should parse");
    let resolved = resolve_model_list(&cfg, None);
    let model = resolved
        .get("my-responses-model")
        .expect("model should exist");
    assert_eq!(model.info.api_backend, ApiBackend::Responses);
}
#[test]
fn parses_model_api_backend_chat_completions() {
    let raw_config: toml::Value = toml::from_str(
        r#"
            [model.my-chat-model]
            model = "codel-4.5"
            base_url = "https://api.example.com/v1"
            context_window = 200000
            api_backend = "chat_completions"
            "#,
    )
    .unwrap();
    let cfg = Config::new_from_toml_cfg(&raw_config).expect("config should parse");
    let resolved = resolve_model_list(&cfg, None);
    let model = resolved.get("my-chat-model").expect("model should exist");
    assert_eq!(model.info.api_backend, ApiBackend::ChatCompletions);
}
/// Messages backend auto-defaults supports_reasoning_effort=true.
/// Without this, `--reasoning-effort` is silently dropped by
/// `model_offers_reasoning_effort` in agent/remote_config/resolution.rs for any
/// BYOK Claude config.
#[test]
fn model_messages_backend_auto_defaults_supports_reasoning_effort() {
    let raw_config: toml::Value = toml::from_str(
        r#"
            [model.my-claude]
            model = "codel-4.5"
            base_url = "https://messages.example.com"
            context_window = 200000
            api_backend = "messages"
            "#,
    )
    .unwrap();
    let cfg = Config::new_from_toml_cfg(&raw_config).expect("config should parse");
    let resolved = resolve_model_list(&cfg, None);
    let model = resolved.get("my-claude").expect("model should exist");
    assert!(
        model.info.supports_reasoning_effort,
        "Messages backend should auto-default supports_reasoning_effort=true",
    );
}
/// An explicit `supports_reasoning_effort = false` in config must override the Messages auto-default; config wins.
#[test]
fn model_messages_backend_respects_explicit_supports_reasoning_effort_false() {
    let raw_config: toml::Value = toml::from_str(
        r#"
            [model.my-claude]
            model = "codel-4.5"
            base_url = "https://messages.example.com"
            context_window = 200000
            api_backend = "messages"
            supports_reasoning_effort = false
            "#,
    )
    .unwrap();
    let cfg = Config::new_from_toml_cfg(&raw_config).expect("config should parse");
    let resolved = resolve_model_list(&cfg, None);
    let model = resolved.get("my-claude").expect("model should exist");
    assert!(
        !model.info.supports_reasoning_effort,
        "explicit supports_reasoning_effort=false in config must override the Messages auto-default",
    );
}
/// Non-Messages backends keep their existing default (false).
/// Adaptive thinking is specific to the Messages backend, and other providers vary per upstream model.
/// The row aliases a wire id with no catalog menu, so the assertion isolates the backend default from slug propagation.
#[test]
fn model_chat_completions_backend_does_not_auto_default_supports_reasoning_effort() {
    let raw_config: toml::Value = toml::from_str(
        r#"
            [model.my-openai]
            model = "upstream-model"
            base_url = "https://api.example.com/v1"
            context_window = 200000
            api_backend = "chat_completions"
            "#,
    )
    .unwrap();
    let cfg = Config::new_from_toml_cfg(&raw_config).expect("config should parse");
    let resolved = resolve_model_list(&cfg, None);
    let model = resolved.get("my-openai").expect("model should exist");
    assert!(
        !model.info.supports_reasoning_effort,
        "ChatCompletions backend must not auto-default supports_reasoning_effort=true",
    );
}
#[test]
fn model_api_backend_defaults_to_chat_completions() {
    let raw_config: toml::Value = toml::from_str(
        r#"
            [model.my-model]
            model = "codel-4.5"
            base_url = "https://api.example.com/v1"
            context_window = 200000
            "#,
    )
    .unwrap();
    let cfg = Config::new_from_toml_cfg(&raw_config).expect("config should parse");
    let resolved = resolve_model_list(&cfg, None);
    let model = resolved.get("my-model").expect("model should exist");
    assert_eq!(model.info.api_backend, ApiBackend::ChatCompletions);
}
#[test]
fn parses_model_use_concise_true() {
    let raw_config: toml::Value = toml::from_str(
        r#"
            [model.my-concise-model]
            model = "my-concise-model"
            base_url = "https://api.example.com/v1"
            context_window = 200000
            use_concise = true
            "#,
    )
    .unwrap();
    let cfg = Config::new_from_toml_cfg(&raw_config).expect("config should parse");
    let resolved = resolve_model_list(&cfg, None);
    let model = resolved
        .get("my-concise-model")
        .expect("model should exist");
    assert!(model.info.use_concise);
}
#[test]
fn model_use_concise_defaults_to_false() {
    let raw_config: toml::Value = toml::from_str(
        r#"
            [model.my-model]
            model = "my-model"
            base_url = "https://api.example.com/v1"
            context_window = 200000
            "#,
    )
    .unwrap();
    let cfg = Config::new_from_toml_cfg(&raw_config).expect("config should parse");
    let resolved = resolve_model_list(&cfg, None);
    let model = resolved.get("my-model").expect("model should exist");
    assert!(!model.info.use_concise);
}
#[test]
fn model_info_from_config_propagates_use_concise() {
    let entry = ModelEntryConfig {
        model: "test".to_string(),
        base_url: "https://test.api/v1".to_string(),
        context_window: NonZeroU64::new(200_000).unwrap(),
        use_concise: true,
        ..Default::default()
    };
    let info = ModelInfo::from_config(&entry);
    assert!(info.use_concise);
}
#[test]
fn deprecated_toolset_use_concise_is_ignored_in_model_config() {
    let raw_config: toml::Value = toml::from_str(
        r#"
            [toolset]
            use_concise = true

            [model.my-model]
            model = "my-model"
            base_url = "https://api.example.com/v1"
            context_window = 200000
            "#,
    )
    .unwrap();
    let cfg = Config::new_from_toml_cfg(&raw_config).expect("config should parse");
    let resolved = resolve_model_list(&cfg, None);
    let model = resolved.get("my-model").expect("model should exist");
    assert!(
        !model.info.use_concise,
        "old [toolset] use_concise should not affect per-model use_concise"
    );
}
#[test]
fn agent_selection_config_defaults_to_none() {
    let cfg = Config::default();
    assert!(cfg.agent.name.is_none());
    assert!(cfg.agent.definition.is_none());
}
#[test]
fn parses_agent_selection_name() {
    let raw_config: toml::Value = toml::from_str(
        r#"
            [agent]
            name = "my-custom-agent"
            "#,
    )
    .unwrap();
    let cfg = Config::new_from_toml_cfg(&raw_config).expect("config should parse");
    assert_eq!(cfg.agent.name.as_deref(), Some("my-custom-agent"));
    assert!(cfg.agent.definition.is_none());
}
#[test]
fn parses_agent_selection_definition_path() {
    let raw_config: toml::Value = toml::from_str(
        r#"
            [agent]
            definition = "/path/to/my-agent.md"
            "#,
    )
    .unwrap();
    let cfg = Config::new_from_toml_cfg(&raw_config).expect("config should parse");
    assert!(cfg.agent.name.is_none());
    assert_eq!(
        cfg.agent.definition.as_deref(),
        Some(std::path::Path::new("/path/to/my-agent.md"))
    );
}
#[test]
fn parses_agent_selection_both_name_and_definition() {
    let raw_config: toml::Value = toml::from_str(
        r#"
            [agent]
            name = "fallback-agent"
            definition = "/path/to/primary-agent.md"
            "#,
    )
    .unwrap();
    let cfg = Config::new_from_toml_cfg(&raw_config).expect("config should parse");
    assert_eq!(cfg.agent.name.as_deref(), Some("fallback-agent"));
    assert_eq!(
        cfg.agent.definition.as_deref(),
        Some(std::path::Path::new("/path/to/primary-agent.md"))
    );
}
#[test]
fn agent_selection_not_specified_uses_defaults() {
    let raw_config: toml::Value = toml::from_str(
        r#"
            [toolset.bash]
            timeout_secs = 123
            "#,
    )
    .unwrap();
    let cfg = Config::new_from_toml_cfg(&raw_config).expect("config should parse");
    assert!(cfg.agent.name.is_none());
    assert!(cfg.agent.definition.is_none());
}
#[test]
fn parses_model_with_agent_type() {
    let raw_config: toml::Value = toml::from_str(
        r#"
            [model.my-agent-model]
            model = "my-agent-model"
            base_url = "https://api.example.com/v1"
            context_window = 200000
            agent_type = "codex"
            "#,
    )
    .unwrap();
    let cfg = Config::new_from_toml_cfg(&raw_config).expect("config should parse");
    let resolved = resolve_model_list(&cfg, None);
    let model = resolved.get("my-agent-model").expect("model should exist");
    assert_eq!(model.info.agent_type, "codex");
}
#[test]
fn model_agent_type_defaults_to_codel_build() {
    let raw_config: toml::Value = toml::from_str(
        r#"
            [model.my-model]
            model = "my-model"
            base_url = "https://api.example.com/v1"
            context_window = 200000
            "#,
    )
    .unwrap();
    let cfg = Config::new_from_toml_cfg(&raw_config).expect("config should parse");
    let resolved = resolve_model_list(&cfg, None);
    let model = resolved.get("my-model").expect("model should exist");
    assert_eq!(model.info.agent_type, DEFAULT_AGENT_TYPE);
}
#[test]
fn model_info_from_config_propagates_agent_type() {
    let entry = ModelEntryConfig {
        model: "test".to_string(),
        base_url: "https://test.api/v1".to_string(),
        context_window: NonZeroU64::new(200_000).unwrap(),
        agent_type: "codex".to_string(),
        ..Default::default()
    };
    let info = ModelInfo::from_config(&entry);
    assert_eq!(info.agent_type, "codex");
}
#[test]
fn hidden_model_excluded_from_acp_but_kept_in_catalog() {
    use crate::agent::remote_config::{available_models, resolve_model_catalog};
    let raw_config: toml::Value = toml::from_str(
        r#"
            [model.visible-model]
            model = "visible-model"
            base_url = "https://api.codel.dev/v1"
            context_window = 200000

            [model.hidden-model]
            model = "hidden-model"
            base_url = "https://api.codel.dev/v1"
            context_window = 200000
            hidden = true
            "#,
    )
    .unwrap();
    let cfg = Config::new_from_toml_cfg(&raw_config).unwrap();
    let catalog = resolve_model_catalog(&cfg, None);
    let available = available_models(&catalog, true);
    assert!(
        catalog.contains_key("visible-model"),
        "visible model missing from catalog"
    );
    assert!(
        catalog.contains_key("hidden-model"),
        "hidden model missing from catalog"
    );
    assert!(
        available.values().any(|m| m.name == "visible-model"),
        "visible model missing from ACP"
    );
    assert!(
        !available.values().any(|m| m.name == "hidden-model"),
        "hidden model should NOT appear in ACP"
    );
}
#[test]
fn disabled_models_removed_from_catalog() {
    use crate::agent::remote_config::resolve_model_catalog;
    let raw: toml::Value = toml::from_str(
        r#"
            [models]
            disabled_models = ["to-disable"]
            [model.to-disable]
            model = "to-disable"
            base_url = "https://api.codel.dev/v1"
            context_window = 200000
            "#,
    )
    .unwrap();
    let catalog = resolve_model_catalog(&Config::new_from_toml_cfg(&raw).unwrap(), None);
    assert!(!catalog.contains_key("to-disable"));
}
#[test]
fn hidden_models_kept_in_catalog_but_not_in_acp() {
    use crate::agent::remote_config::{available_models, resolve_model_catalog};
    let raw: toml::Value = toml::from_str(
        r#"
            [models]
            hidden_models = ["to-hide"]
            [model.to-hide]
            model = "to-hide"
            base_url = "https://api.codel.dev/v1"
            context_window = 200000
            "#,
    )
    .unwrap();
    let catalog = resolve_model_catalog(&Config::new_from_toml_cfg(&raw).unwrap(), None);
    let available = available_models(&catalog, true);
    assert!(catalog.contains_key("to-hide"));
    assert!(catalog.get("to-hide").is_some_and(|c| c.info.hidden));
    assert!(!available.values().any(|m| m.name == "to-hide"));
}
#[test]
fn allowed_models_marks_selectable_by_wildcard_key_or_model() {
    use crate::agent::remote_config::resolve_model_catalog;
    let raw: toml::Value = toml::from_str(
        r#"
            [models]
            allowed_models = ["keep-*", "explicit-key", "explicit-model-id"]
            [model.to-drop]
            model = "to-drop"
            base_url = "https://api.codel.dev/v1"
            context_window = 256000
            [model.keep-one]
            model = "keep-one"
            base_url = "https://api.codel.dev/v1"
            context_window = 256000
            [model.explicit-key]
            model = "explicit-model-id"
            base_url = "https://api.codel.dev/v1"
            context_window = 256000
            "#,
    )
    .unwrap();
    let catalog = resolve_model_catalog(&Config::new_from_toml_cfg(&raw).unwrap(), None);
    assert!(
        catalog
            .get("keep-one")
            .is_some_and(|c| c.info.user_selectable),
        "wildcard match"
    );
    assert!(
        catalog
            .get("explicit-key")
            .is_some_and(|c| c.info.user_selectable),
        "matched by catalog key or model id"
    );
    assert!(
        catalog
            .get("to-drop")
            .is_some_and(|c| !c.info.user_selectable),
        "kept but not selectable"
    );
}
#[test]
fn allowed_models_empty_is_unrestricted() {
    use crate::agent::remote_config::resolve_model_catalog;
    let raw: toml::Value = toml::from_str(
        r#"
            [models]
            allowed_models = []
            [model.foo]
            model = "foo"
            base_url = "https://api.codel.dev/v1"
            context_window = 256000
            "#,
    )
    .unwrap();
    let catalog = resolve_model_catalog(&Config::new_from_toml_cfg(&raw).unwrap(), None);
    assert!(
        catalog.get("foo").is_some_and(|c| c.info.user_selectable),
        "empty allowed_models must not restrict"
    );
}
#[test]
fn invalid_glob_is_rejected_by_validation() {
    use crate::agent::remote_config::ModelGlobSet;
    assert!(ModelGlobSet::compile(Some(["codel[".to_string()].as_slice())).is_err());
    let raw: toml::Value = toml::from_str(
        r#"
            [models]
            allowed_models = ["codel["]
            "#,
    )
    .unwrap();
    let err = Config::new_from_toml_cfg(&raw)
        .unwrap()
        .validate_model_filters()
        .unwrap_err();
    assert!(
        err.contains("allowed_models"),
        "error should name the offending field: {err}"
    );
}
#[test]
fn supported_in_api_false_hides_from_api_key_users() {
    use crate::agent::remote_config::{available_models, resolve_model_catalog};
    let raw: toml::Value = toml::from_str(
        r#"
            [model.oauth-only-model]
            model = "oauth-only-model"
            base_url = "https://api.codel.dev/v1"
            context_window = 200000
            supported_in_api = false

            [model.public-model]
            model = "public-model"
            base_url = "https://api.codel.dev/v1"
            context_window = 200000
            "#,
    )
    .unwrap();
    let cfg = Config::new_from_toml_cfg(&raw).unwrap();
    let catalog = resolve_model_catalog(&cfg, None);
    assert!(catalog.contains_key("oauth-only-model"));
    assert!(catalog.contains_key("public-model"));
    let api_available = available_models(&catalog, false);
    assert!(!api_available.values().any(|m| m.name == "oauth-only-model"));
    assert!(api_available.values().any(|m| m.name == "public-model"));
    let oauth_available = available_models(&catalog, true);
    assert!(
        oauth_available
            .values()
            .any(|m| m.name == "oauth-only-model")
    );
    assert!(oauth_available.values().any(|m| m.name == "public-model"));
}
#[test]
fn inference_idle_timeout_secs_round_trip() {
    let raw_config: toml::Value = toml::from_str(
        r#"
            [model.slow-model]
            model = "codel-4.5"
            base_url = "https://api.codel.dev/v1"
            context_window = 200000
            inference_idle_timeout_secs = 600
            "#,
    )
    .unwrap();
    let cfg = Config::new_from_toml_cfg(&raw_config).expect("config should parse");
    let resolved = resolve_model_list(&cfg, None);
    let model = resolved.get("slow-model").expect("model should exist");
    assert_eq!(model.info.inference_idle_timeout_secs, Some(600));
}
#[test]
fn inference_idle_timeout_secs_absent_defaults_to_none() {
    let raw_config: toml::Value = toml::from_str(
        r#"
            [model.default-model]
            model = "codel-fast"
            base_url = "https://api.codel.dev/v1"
            context_window = 200000
            "#,
    )
    .unwrap();
    let cfg = Config::new_from_toml_cfg(&raw_config).expect("config should parse");
    let resolved = resolve_model_list(&cfg, None);
    let model = resolved.get("default-model").expect("model should exist");
    assert_eq!(model.info.inference_idle_timeout_secs, None);
}
#[test]
fn inference_idle_timeout_propagates_to_model_info() {
    let entry = ModelEntryConfig {
        model: "test".to_string(),
        base_url: "https://test.api/v1".to_string(),
        context_window: NonZeroU64::new(200_000).unwrap(),
        inference_idle_timeout_secs: Some(120),
        ..Default::default()
    };
    let info = ModelInfo::from_config(&entry);
    assert_eq!(info.inference_idle_timeout_secs, Some(120));
}
#[test]
fn telemetry_config_parses_custom_values_from_toml() {
    let raw: toml::Value = toml::from_str(
        r#"
            [telemetry]
            events_url     = "https://custom.example.com/events"
            events_api_key = "custom-key"
            mixpanel_token = "custom-token"
            mixpanel_enabled = false
            "#,
    )
    .unwrap();
    let cfg = Config::new_from_toml_cfg(&raw).expect("should parse");
    assert_eq!(
        cfg.telemetry.events_url.as_deref(),
        Some("https://custom.example.com/events")
    );
    assert_eq!(cfg.telemetry.events_api_key.as_deref(), Some("custom-key"));
    assert_eq!(
        cfg.telemetry.mixpanel_token.as_deref(),
        Some("custom-token")
    );
    assert!(!cfg.telemetry.mixpanel_enabled);
}
/// Empty/whitespace values must become `None`, not reach the HTTP client as empty strings.
#[test]
fn telemetry_empty_string_disables_sink() {
    let raw: toml::Value = toml::from_str(
        r#"
            [telemetry]
            events_url     = ""
            events_api_key = "  "
            mixpanel_token = "\t"
            "#,
    )
    .unwrap();
    let cfg = Config::new_from_toml_cfg(&raw).expect("should parse");
    assert!(cfg.telemetry.events_url.is_none());
    assert!(cfg.telemetry.events_api_key.is_none());
    assert!(cfg.telemetry.mixpanel_token.is_none());
}
#[test]
fn telemetry_partial_override_retains_defaults() {
    let raw: toml::Value = toml::from_str(
        r#"
            [telemetry]
            events_url = "https://my-proxy/events"
            "#,
    )
    .unwrap();
    let cfg = Config::new_from_toml_cfg(&raw).expect("should parse");
    assert_eq!(
        cfg.telemetry.events_url.as_deref(),
        Some("https://my-proxy/events")
    );
    let defaults = TelemetryConfig::default();
    assert_eq!(cfg.telemetry.events_api_key, defaults.events_api_key);
    assert_eq!(cfg.telemetry.mixpanel_token, defaults.mixpanel_token);
    assert_eq!(cfg.telemetry.mixpanel_enabled, defaults.mixpanel_enabled);
}
/// `disable_api_key_auth` parses through the `[auth]` alias, and absent means None (opt-in knob, zero impact by default).
#[test]
fn disable_api_key_auth_parses_from_auth_alias() {
    let absent = Config::new_from_toml_cfg(&toml::from_str("").unwrap()).unwrap();
    assert_eq!(absent.codel_com_config.disable_api_key_auth, None);
    let raw: toml::Value = toml::from_str(
        r#"
            [auth]
            disable_api_key_auth = true
            "#,
    )
    .unwrap();
    let cfg = Config::new_from_toml_cfg(&raw).expect("config should parse");
    assert_eq!(cfg.codel_com_config.disable_api_key_auth, Some(true));
}
/// `login_device_flow` reaches `Config::login_device_flow` via both `[codel_com_config]` and the `[auth]` alias, without warning as unrecognized.
#[test]
fn login_device_flow_reads_from_config() {
    let absent = Config::new_from_toml_cfg(&toml::from_str("").unwrap()).unwrap();
    assert_eq!(absent.login_device_flow, None);
    for section in ["codel_com_config", "auth"] {
        let raw: toml::Value =
            toml::from_str(&format!("[{section}]\nlogin_device_flow = true\n")).unwrap();
        let cfg = Config::new_from_toml_cfg(&raw).expect("config should parse");
        assert_eq!(
            cfg.login_device_flow,
            Some(true),
            "`[{section}] login_device_flow` must reach Config::login_device_flow"
        );
        assert!(
            !cfg.config_warnings
                .iter()
                .any(|w| w.target.label().contains("login_device_flow")),
            "`[{section}] login_device_flow` must not warn as unrecognized: {:?}",
            cfg.config_warnings
        );
    }
}
fn resolve_models_from_toml(
    toml_str: &str,
    prefetched: Option<IndexMap<String, ModelEntry>>,
) -> (Config, IndexMap<String, ModelEntry>) {
    let raw: toml::Value = toml::from_str(toml_str).expect("test TOML should parse");
    let cfg = Config::new_from_toml_cfg(&raw).expect("config should parse");
    let resolved = resolve_model_list(&cfg, prefetched);
    (cfg, resolved)
}
fn resolve_sampling(model: &ModelEntry, session_key: Option<&str>) -> SamplerConfig {
    let credentials = resolve_credentials(model, session_key);
    sampling_config_for_model(model, credentials, None, None, None, None)
}
#[test]
#[serial]
fn e2e_user_overrides_default_model_key_with_custom_endpoint() {
    let dm = crate::models::default_model();
    let (_, models) = resolve_models_from_toml(
        &format!(
            r#"
            [model."{dm}"]
            model = "{dm}"
            base_url = "https://inference.example.com/v1"
            context_window = 200000
            env_key = "ENTERPRISE_AUTH_TOKEN"
            "#,
        ),
        None,
    );
    let model = models.get(dm).expect("model should exist");
    assert_eq!(model.info.base_url, "https://inference.example.com/v1");
    assert_eq!(
        model.env_key.as_ref().and_then(|k| k.primary()),
        Some("ENTERPRISE_AUTH_TOKEN")
    );
    unsafe { std::env::set_var("ENTERPRISE_AUTH_TOKEN", "enterprise-secret-key") };
    let sampling = resolve_sampling(model, None);
    assert_eq!(
        sampling.api_key.as_deref(),
        Some("enterprise-secret-key"),
        "should use the user's env_key, not fall through to session/external"
    );
    assert_eq!(
        sampling.base_url, "https://inference.example.com/v1",
        "should route to the user's custom endpoint, not api.codel.dev"
    );
    unsafe { std::env::remove_var("ENTERPRISE_AUTH_TOKEN") };
}
#[test]
#[serial]
fn e2e_config_toml_model_overrides_default() {
    let dm = crate::models::default_model();
    let (_, models) = resolve_models_from_toml(
        &format!(
            r#"
            [model."{dm}"]
            base_url = "https://inference.example.com/v1"
            "#,
        ),
        None,
    );
    let model = models.get(dm).expect("model should exist");
    let sampling = resolve_sampling(model, Some("session-tok"));
    assert_eq!(sampling.base_url, "https://inference.example.com/v1");
    unsafe { std::env::set_var("CODEL_API_KEY", "codel-key") };
    let sampling = resolve_sampling(model, None);
    assert_eq!(sampling.base_url, "https://inference.example.com/v1");
    unsafe { std::env::remove_var("CODEL_API_KEY") };
    let sampling = resolve_sampling(model, None);
    assert_eq!(sampling.base_url, "https://inference.example.com/v1");
}
#[test]
fn e2e_user_overrides_default_model_with_api_key() {
    let dm = crate::models::default_model();
    let (_, models) = resolve_models_from_toml(
        &format!(
            r#"
            [model."{dm}"]
            model = "{dm}"
            base_url = "https://my-proxy.example.com/v1"
            context_window = 200000
            api_key = "my-custom-api-key"
            "#,
        ),
        None,
    );
    let model = models.get(dm).expect("model should exist");
    assert_eq!(model.info.base_url, "https://my-proxy.example.com/v1");
    assert_eq!(model.api_key.as_deref(), Some("my-custom-api-key"));
    assert!(model.env_key.is_none());
    let sampling = resolve_sampling(model, Some("session-token"));
    assert_eq!(
        sampling.api_key.as_deref(),
        Some("my-custom-api-key"),
        "model's own api_key must beat session token"
    );
    assert_eq!(
        sampling.base_url, "https://my-proxy.example.com/v1",
        "should route to user's custom endpoint"
    );
}
#[test]
fn parsed_config_has_models_config() {
    let raw: toml::Value = toml::from_str(
        r#"
            [models]
            default = "my-enterprise-model"
            web_search = "enterprise-search"
            session_summary = "title-model"
            "#,
    )
    .unwrap();
    let cfg = Config::new_from_toml_cfg(&raw).expect("config should parse");
    assert_eq!(cfg.models.default.as_deref(), Some("my-enterprise-model"));
    assert_eq!(cfg.models.web_search.as_deref(), Some("enterprise-search"));
    assert_eq!(cfg.models.session_summary.as_deref(), Some("title-model"));
}
#[test]
fn config_models_default_is_not_overwritten_by_default_models_json() {
    let config_default = Some("custom-byok-model");
    let remote_settings_default = Some("remote-settings-model");
    let resolved = resolve_string_flag(
        None,
        "CODEL_DEFAULT_MODEL_TEST_NONEXISTENT",
        config_default,
        remote_settings_default,
    );
    let resolved = resolved.expect("should resolve to a value");
    assert_eq!(resolved.value, "custom-byok-model");
    assert_eq!(
        resolved.source,
        ConfigSource::Config,
        "[models] default from config.toml must beat remote settings and compiled-in defaults"
    );
}
#[test]
fn config_models_default_custom_model_is_in_resolved_model_list() {
    let (_, models) = resolve_models_from_toml(
        r#"
            [model.acme-codel]
            model = "codel-4.5"
            base_url = "https://inference.example.com/v1"
            context_window = 256000
            env_key = "ENTERPRISE_AUTH_TOKEN"
            "#,
        None,
    );
    assert!(
        models.contains_key("acme-codel"),
        "user-defined model must be in the resolved model list"
    );
    let model = models.get("acme-codel").unwrap();
    assert_eq!(model.info.model, "codel-4.5");
    assert_eq!(model.info.base_url, "https://inference.example.com/v1");
}
#[test]
fn e2e_default_model_with_session_routes_to_proxy() {
    let (_, models) = resolve_models_from_toml("", None);
    let model = models
        .get(crate::models::default_model())
        .expect("default model should exist");
    let sampling = resolve_sampling(model, Some("session-token-123"));
    assert_eq!(sampling.api_key.as_deref(), Some("session-token-123"));
    assert_eq!(
        sampling.base_url, "https://cli-chat-proxy.codel.dev/v1",
        "session auth should route to cli-chat-proxy, not api.codel.dev"
    );
}
#[test]
#[serial]
fn e2e_default_model_with_external_api_key_routes_to_api_codel() {
    let (_, models) = resolve_models_from_toml("", None);
    let model = models
        .get(crate::models::default_model())
        .expect("default model should exist");
    unsafe { std::env::set_var("CODEL_API_KEY", "codel-external-key") };
    let sampling = resolve_sampling(model, None);
    assert_eq!(sampling.api_key.as_deref(), Some("codel-external-key"));
    assert_eq!(
        sampling.base_url, "https://api.codel.dev/v1",
        "external API key should route to api.codel.dev via api_base_url"
    );
    unsafe { std::env::remove_var("CODEL_API_KEY") };
}
#[test]
fn e2e_duplicate_model_field_both_entries_survive() {
    let dm = crate::models::default_model();
    let (_, models) = resolve_models_from_toml(
        &format!(
            r#"
            [model.acme-codel]
            model = "{dm}"
            base_url = "https://inference.example.com/v1"
            context_window = 200000
            api_key = "enterprise-key"
            "#,
        ),
        None,
    );
    assert!(models.contains_key(dm), "default entry should still exist");
    assert!(
        models.contains_key("acme-codel"),
        "user entry with different key should also exist"
    );
    let default = models.get(dm).unwrap();
    let user = models.get("acme-codel").unwrap();
    assert_eq!(default.info.model, user.info.model, "same model field");
    assert_ne!(
        default.info.base_url, user.info.base_url,
        "different base_urls"
    );
    let sampling = resolve_sampling(user, None);
    assert_eq!(sampling.api_key.as_deref(), Some("enterprise-key"));
    assert_eq!(sampling.base_url, "https://inference.example.com/v1");
    let sampling = resolve_sampling(default, Some("session-key"));
    assert_eq!(sampling.api_key.as_deref(), Some("session-key"));
    assert_eq!(sampling.base_url, "https://cli-chat-proxy.codel.dev/v1",);
}
#[test]
fn e2e_default_endpoint_still_injects_defaults() {
    let cfg = Config::default();
    let resolved = resolve_model_list(&cfg, None);
    assert!(
        resolved.contains_key(crate::models::default_model()),
        "default model should be present when using default endpoint"
    );
}
#[test]
fn e2e_enterprise_endpoints_plus_partial_model_override() {
    let dm = crate::models::default_model();
    let (_, models) = resolve_models_from_toml(
        &format!(
            r#"
            [endpoints]
            cli_chat_proxy_base_url = "https://enterprise-proxy.acme.com/v1"
            codel_api_base_url = "https://enterprise-api.acme.com/v1"

            [model."{dm}"]
            api_key = "acme-api-key"
            "#,
        ),
        None,
    );
    let model = models.get(dm).expect("model should exist");
    assert_eq!(
        model.info.base_url, "https://enterprise-proxy.acme.com/v1",
        "base_url must inherit from [endpoints], not stale default"
    );
    assert_eq!(model.api_key.as_deref(), Some("acme-api-key"));
    assert_eq!(
        model.api_base_url.as_deref(),
        Some("https://enterprise-api.acme.com/v1"),
    );
    let sampling = resolve_sampling(model, Some("session-token"));
    assert_eq!(
        sampling.api_key.as_deref(),
        Some("acme-api-key"),
        "model's own api_key must beat session token"
    );
    assert_eq!(
        sampling.base_url, "https://enterprise-proxy.acme.com/v1",
        "sampling must route to enterprise proxy"
    );
}
#[test]
fn e2e_enterprise_endpoints_only_no_model_override() {
    let (_, models) = resolve_models_from_toml(
        r#"
            [endpoints]
            cli_chat_proxy_base_url = "https://enterprise-proxy.acme.com/v1"
            codel_api_base_url = "https://enterprise-api.acme.com/v1"
            "#,
        None,
    );
    let model = models
        .get(crate::models::default_model())
        .expect("model should exist");
    assert_eq!(
        model.info.base_url, "https://enterprise-proxy.acme.com/v1",
        "default model should use enterprise cli_chat_proxy_base_url"
    );
    assert_eq!(
        model.api_base_url.as_deref(),
        Some("https://enterprise-api.acme.com/v1"),
        "default model should use enterprise codel_api_base_url"
    );
}
/// Unset every env var that `EndpointsConfig::default()` reads for endpoints.
/// The cli-chat-proxy resolver tests below are then deterministic regardless of the ambient environment.
/// Gated behind `#[serial]`.
fn unset_endpoint_env_vars() {
    for k in [
        "CODEL_CLI_CHAT_PROXY_BASE_URL",
        "CODEL_CODEL_API_BASE_URL",
        "CODEL_FEEDBACK_BASE_URL",
        "CODEL_TRACE_UPLOAD_URL",
        "CODEL_MANAGED_CONFIG_URL",
        "CODEL_MODELS_BASE_URL",
        "CODEL_MODELS_LIST_URL",
        "OTEL_EXPORTER_OTLP_ENDPOINT",
        "OTEL_EXPORTER_OTLP_TRACES_ENDPOINT",
        "OTEL_EXPORTER_OTLP_HEADERS",
        "CODEL_INTERNAL_OTLP_TRACES_ENDPOINT",
        "CODEL_INTERNAL_OTLP_HEADERS",
        "CODEL_EXTERNAL_OTEL",
    ] {
        unsafe { std::env::remove_var(k) };
    }
}
#[test]
fn e2e_user_override_explicit_base_url_wins_over_endpoints() {
    let dm = crate::models::default_model();
    let (_, models) = resolve_models_from_toml(
        &format!(
            r#"
            [endpoints]
            cli_chat_proxy_base_url = "https://enterprise-proxy.acme.com/v1"

            [model."{dm}"]
            base_url = "https://my-special-proxy.example.com/v1"
            "#,
        ),
        None,
    );
    let model = models.get(dm).expect("model should exist");
    assert_eq!(
        model.info.base_url, "https://my-special-proxy.example.com/v1",
        "explicit base_url in [model.*] must win over [endpoints]"
    );
}
#[test]
fn e2e_models_endpoint_serde_alias_parses_as_models_list_url() {
    let raw: toml::Value = toml::from_str(
        r#"
            [endpoints]
            models_endpoint = "https://old-style.acme.com/v1/models"
            "#,
    )
    .unwrap();
    let cfg = Config::new_from_toml_cfg(&raw).expect("config should parse");
    assert_eq!(
        cfg.endpoints.models_list_url.as_deref(),
        Some("https://old-style.acme.com/v1/models"),
        "models_endpoint alias should parse into models_list_url"
    );
    assert!(cfg.endpoints.has_custom_endpoint());
}
#[test]
fn e2e_config_models_parsed_directly_not_via_deep_merge() {
    let raw: toml::Value = toml::from_str(
        r#"
            [model.custom-model]
            model = "my-custom-llm"
            api_key = "custom-key"
            "#,
    )
    .unwrap();
    let cfg = Config::new_from_toml_cfg(&raw).expect("config should parse");
    assert!(cfg.config_models.contains_key("custom-model"));
    let model_override = cfg.config_models.get("custom-model").unwrap();
    assert_eq!(model_override.model.as_deref(), Some("my-custom-llm"));
    assert_eq!(model_override.api_key.as_deref(), Some("custom-key"));
    assert!(
        model_override.base_url.is_none(),
        "base_url should be None when user didn't set it"
    );
}
#[test]
fn model_mtls_cert_dir_flows_from_config_toml_to_sampler() {
    let (_, models) = resolve_models_from_toml(
        r#"
            [model.secure-model]
            model = "secure-upstream"
            base_url = "https://inference.example.com/v1"
            context_window = 200000
            api_key = "test-key"
            mtls_cert_dir = "/run/secrets/secure-model"
            "#,
        None,
    );
    let model = models
        .get("secure-model")
        .expect("configured model should resolve");
    assert_eq!(
        model.mtls_cert_dir.as_deref(),
        Some(std::path::Path::new("/run/secrets/secure-model"))
    );
    let sampling = resolve_sampling(model, None);
    assert_eq!(
        sampling.mtls_cert_dir.as_deref(),
        Some(std::path::Path::new("/run/secrets/secure-model"))
    );
    assert_eq!(sampling.base_url, "https://inference.example.com/v1");
}
#[test]
fn model_mtls_configuration_requires_one_explicit_https_destination() {
    for (config, expected_error) in [
        (
            r#"[model.secure]
               mtls_cert_dir = "/run/secrets/secure-model""#,
            "mtls_cert_dir requires base_url in the same model table",
        ),
        (
            r#"[model.secure]
               base_url = "http://inference.example.com/v1"
               mtls_cert_dir = "/run/secrets/secure-model""#,
            "base_url must be an HTTPS URL with a host",
        ),
        (
            r#"[model.secure]
               base_url = "not a URL"
               mtls_cert_dir = "/run/secrets/secure-model""#,
            "base_url is invalid",
        ),
        (
            r#"[model.secure]
               base_url = "https://inference.example.com/v1"
               mtls_cert_dir = """#,
            "mtls_cert_dir must not be empty",
        ),
        (
            r#"[model.secure]
               base_url = "https://inference.example.com/v1"
               api_base_url = "https://api.example.com/v1"
               mtls_cert_dir = "/run/secrets/secure-model""#,
            "cannot set both mtls_cert_dir and api_base_url",
        ),
        (
            r#"[model_providers.gateway]
               api_base_url = "https://api.example.com/v1"

               [model.secure]
               base_url = "https://inference.example.com/v1"
               model_provider = "gateway"
               mtls_cert_dir = "/run/secrets/secure-model""#,
            "cannot use model_providers.gateway.api_base_url with mtls_cert_dir",
        ),
    ] {
        let raw: toml::Value = toml::from_str(config).expect("test config should parse as TOML");
        let error = Config::new_from_toml_cfg(&raw).expect_err("invalid mTLS config must fail");
        assert!(
            error.contains(expected_error),
            "expected {expected_error:?} in {error:?}"
        );
    }
}
/// A field holding a registered key is read as of whenever it was written, and these three are built before the value's last writer runs.
/// `auto_wake` shipped that way and lost every pin.
/// Catches the spelling, not the class: a mirror under another name still gets through.
#[test]
fn no_registered_feature_is_mirrored_by_a_config_field() {
    const SRC: &str = include_str!("config.rs");
    const AGENT: &str = include_str!("mvp_agent/mod.rs");
    for (src, decl) in [
        (SRC, "pub struct Config {"),
        (SRC, "pub struct Features {"),
        (AGENT, "pub struct MvpAgent {"),
    ] {
        let body = src
            .split_once(decl)
            .and_then(|(_, rest)| rest.split_once("\n}\n"))
            .map(|(body, _)| body)
            .unwrap_or_else(|| panic!("{decl} moved; this test needs its new shape"));
        for spec in FEATURES {
            for field in [
                format!("{}: bool", spec.key),
                format!("{}_enabled: bool", spec.key),
                format!("{}: Option<bool>", spec.key),
                format!("{}_enabled: Option<bool>", spec.key),
            ] {
                assert!(
                    !body.contains(&field),
                    "`{field}` mirrors the {} row; read the registry at use time \
                     instead, so a pin applied after this field was written still counts",
                    spec.key,
                );
            }
        }
    }
}
/// The tamper-resistance `25-enterprise.md` sells to administrators, for every key an administrator can pin.
#[test]
#[serial]
fn requirement_pin_outranks_a_hostile_environment() {
    for spec in FEATURES {
        let pinned = !spec.default_enabled;
        let _env = EnvGuard::set(spec.env, if pinned { "0" } else { "1" });
        let mut cfg = Config::default();
        cfg.requirements
            .pin_feature(spec.id, pinned, crate::config::RequirementSource::Unknown);
        let r = cfg.feature(spec.id);
        assert_eq!(r.value, pinned, "{} lost to {}", spec.key, spec.env);
        assert_eq!(r.source, ConfigSource::Requirement, "{}", spec.key);
    }
}
/// A registered key is a `&'static str` matched against the `[features]` table, not a serde field name, so every one of them is read back here.
#[test]
#[serial]
fn every_registered_key_parses_out_of_the_features_table() {
    for spec in FEATURES {
        let configured = !spec.default_enabled;
        let raw: toml::Value =
            toml::from_str(&format!("[features]\n{} = {configured}\n", spec.key)).unwrap();
        let cfg = Config::new_from_toml_cfg(&raw).unwrap();
        {
            let _env = EnvGuard::unset(spec.env);
            let r = cfg.feature(spec.id);
            assert_eq!(
                r.value, configured,
                "{} never reached the registry",
                spec.key
            );
            assert_eq!(r.source, ConfigSource::Config, "{}", spec.key);
        }
        let _env = EnvGuard::set(spec.env, if configured { "0" } else { "1" });
        let r = cfg.feature(spec.id);
        assert_eq!(r.value, !configured, "config.toml outranked {}", spec.env);
        assert_eq!(r.source, ConfigSource::Env, "{}", spec.key);
    }
}
/// The keys the list names are read from the raw layers, so each must be one no field claims.
/// A quoted `remote_fetch` used to read as absent and leave the egress gate open.
#[test]
fn non_boolean_value_fails_the_load_for_a_key_with_no_field() {
    let features = include_str!("config.rs")
        .split_once("pub struct Features {")
        .and_then(|(_, rest)| rest.split_once("\n}\n"))
        .map(|(body, _)| body)
        .expect("`Features` moved; this test needs its new shape");
    for key in UNMIRRORED_BOOLEAN_FEATURES {
        assert!(
            !FEATURES.iter().any(|spec| spec.key == *key),
            "{key} is a registry row, which is already known"
        );
        assert!(
            !features.contains(&format!("pub {key}:")),
            "{key} has a `Features` field, so it is read through that instead"
        );
        let raw: toml::Value = toml::from_str(&format!("[features]\n{key} = \"false\"\n")).unwrap();
        let err = Config::new_from_toml_cfg(&raw)
            .expect_err(&format!("{key}: a quoted value must not read as absent"));
        assert!(
            err.contains(key) && err.contains("true or false"),
            "{key}: the error names the key and the spelling that works: {err}"
        );
    }
}
/// What no list could cover: a key this build has never heard of is typed all the same.
/// That means the next boolean added to `[features]` is checked before anyone writes it down.
/// `image_edit` takes the same path, which is why it is kept out of the list that only suppresses the unrecognized-key warning.
#[test]
fn non_boolean_value_fails_the_load_for_an_unregistered_key() {
    for key in ["image_edit", "a_key_no_build_has_ever_had"] {
        let raw: toml::Value = toml::from_str(&format!("[features]\n{key} = \"false\"\n")).unwrap();
        let err = Config::new_from_toml_cfg(&raw)
            .expect_err(&format!("{key}: a quoted value must not read as absent"));
        assert!(
            err.contains(key) && err.contains("true or false"),
            "{key}: the error names the key and the spelling that works: {err}"
        );
    }
}
/// The other half of the rule.
/// A later release can add a `[features]` key that holds something other than a boolean.
/// The build that predates its field has to start on that config rather than strand a fleet mid-rollout.
#[test]
fn a_value_that_reads_as_nothing_like_a_boolean_still_loads() {
    for value in ["\"aggressive\"", "42", "[\"a\", \"b\"]", "1.5"] {
        let entry = format!("[features]\na_later_release_key = {value}\n");
        let raw: toml::Value = toml::from_str(&entry).unwrap();
        Config::new_from_toml_cfg(&raw)
            .unwrap_or_else(|e| panic!("{value} must not stop an older build: {e}"));
        let unused = unused_keys_from_toml(&entry);
        assert!(
            unused
                .iter()
                .any(|key| key == "features.a_later_release_key"),
            "{value}: ignored, and the operator still hears about it: {unused:?}"
        );
    }
}
/// Managed policy keys are workspace-resolved, not Config serde: every spelling must pass the unrecognized-key scan.
#[test]
fn managed_policy_keys_do_not_warn_as_unrecognized() {
    let entry = r#"
allow_managed_mcp_servers_only = true
allowManagedMcpServersOnly = true
enable_all_project_mcp_servers = false
enableAllProjectMcpServers = false
plugin_auto_update = false
pluginAutoUpdate = false
allow_managed_hooks_only = true
allowManagedHooksOnly = true

[[allowed_mcp_servers]]
server_url = "https://mcp.example.com/*"

[[allowedMcpServers]]
serverCommand = ["npx", "@corp/mcp"]

[[denied_mcp_servers]]
server_name = "blocked"

[[deniedMcpServers]]
serverUrl = "https://evil.example.com/*"

[[strict_known_marketplaces]]
source = "git"
url = "https://github.com/corp/approved.git"

[[strictKnownMarketplaces]]
source = "github"
repo = "corp/more"

[extra_known_marketplaces.corp]
source = { source = "git", url = "https://github.com/corp/approved.git" }

[extraKnownMarketplaces.corp2]
source = { source = "git", url = "https://github.com/corp/other.git" }
"#;
    let unused = unused_keys_from_toml(entry);
    let leaked: Vec<&String> = unused
        .iter()
        .filter(|key| {
            codel_workspace::permission::resolution::MANAGED_POLICY_CONFIG_KEYS
                .iter()
                .any(|policy_key| {
                    key.as_str() == *policy_key || key.starts_with(&format!("{policy_key}."))
                })
        })
        .collect();
    assert!(
        leaked.is_empty(),
        "policy keys must not warn as unrecognized: {leaked:?}"
    );
}
/// The non-row keys that do have a field are turned away by serde, whatever it words the failure as.
#[test]
fn non_boolean_value_fails_the_load_for_a_key_with_a_field() {
    for key in ["title_refresh", "image_gen", "video_gen"] {
        let raw: toml::Value = toml::from_str(&format!("[features]\n{key} = \"false\"\n")).unwrap();
        Config::new_from_toml_cfg(&raw)
            .expect_err(&format!("{key}: a quoted value must not read as absent"));
    }
}
#[test]
fn non_boolean_feature_value_fails_the_load() {
    let raw: toml::Value = toml::from_str("[features]\nsession_search = \"no\"\n").unwrap();
    let err = Config::new_from_toml_cfg(&raw)
        .expect_err("a quoted value must not read as off and leave the index on");
    assert!(
        err.contains("features.session_search") && err.contains("true or false"),
        "the error names the key and the spelling that works: {err}"
    );
}
/// Title refresh defaults to the resolved `turn_summary` value, but each knob (env / config / remote) can flip it independently of turn summary.
#[test]
#[serial]
fn resolve_title_refresh_defaults_to_turn_summary_but_decouples() {
    unsafe { std::env::remove_var("CODEL_TITLE_REFRESH") };
    unsafe { std::env::remove_var("CODEL_TURN_SUMMARY") };
    let r = Config::default().resolve_title_refresh();
    assert!(r.value, "title_refresh defaults to turn_summary (on)");
    let ts_off = Config {
        remote_settings: Some(crate::util::config::RemoteSettings {
            turn_summary: Some(false),
            ..Default::default()
        }),
        ..Default::default()
    };
    assert!(
        !ts_off.resolve_title_refresh().value,
        "default follows turn_summary"
    );
    let decoupled = Config {
        features: Features {
            title_refresh: Some(true),
            ..Default::default()
        },
        ..ts_off
    };
    let r = decoupled.resolve_title_refresh();
    assert!(
        r.value,
        "title_refresh config overrides the turn_summary default"
    );
    assert_eq!(r.source, ConfigSource::Config);
    unsafe { std::env::set_var("CODEL_TITLE_REFRESH", "0") };
    let r = decoupled.resolve_title_refresh();
    assert!(!r.value, "CODEL_TITLE_REFRESH env wins");
    assert_eq!(r.source, ConfigSource::Env);
    unsafe { std::env::remove_var("CODEL_TITLE_REFRESH") };
}
/// A `turn_summary` pin lands in the title's default slot, so it moves the title with it.
/// Only the default slot, so `CODEL_TITLE_REFRESH` still outranks it and a user can turn the title back on.
/// Pinning `title_refresh` is what closes that.
#[test]
#[serial]
fn a_turn_summary_pin_moves_the_title_default_and_the_environment_lifts_it() {
    let _env = EnvGuard::set("CODEL_TURN_SUMMARY", "1");
    let mut cfg = Config::default();
    cfg.requirements.pin_feature(
        Feature::TurnSummary,
        false,
        crate::config::RequirementSource::Unknown,
    );
    {
        let _title = EnvGuard::unset("CODEL_TITLE_REFRESH");
        assert!(
            !cfg.resolve_title_refresh().value,
            "the pin outranks CODEL_TURN_SUMMARY, and the title default follows the pin"
        );
    }
    let _title = EnvGuard::set("CODEL_TITLE_REFRESH", "1");
    let r = cfg.resolve_title_refresh();
    assert!(r.value, "the environment outranks a derived default");
    assert_eq!(r.source, ConfigSource::Env);
}
/// The tier that closes it.
/// Not a registry row, so the sweep in `requirement_pin_outranks_a_hostile_environment` never reaches this key.
#[test]
#[serial]
fn a_title_refresh_pin_outranks_the_environment() {
    let _env = EnvGuard::set("CODEL_TITLE_REFRESH", "1");
    let mut cfg = Config::default();
    cfg.requirements
        .title_refresh
        .pin(false, crate::config::RequirementSource::Unknown);
    let r = cfg.resolve_title_refresh();
    assert!(!r.value, "the pin lost to CODEL_TITLE_REFRESH");
    assert_eq!(r.source, ConfigSource::Requirement);
}
#[test]
#[serial]
fn resolve_long_reasoning_reminder_precedence() {
    use crate::session::long_reasoning_reminder::LongReasoningReminder;
    use crate::util::config::LongReasoningReminderSettings;
    let _env = EnvGuard::unset("CODEL_LONG_REASONING_REMINDER");
    assert_eq!(
        LongReasoningReminder {
            enabled: false,
            tokens: 1000,
            delay: 1
        },
        Config::default().resolve_long_reasoning_reminder(),
        "default is OFF with default tuning"
    );
    let remote_on = Config {
        remote_settings: Some(crate::util::config::RemoteSettings {
            long_reasoning_reminder: Some(LongReasoningReminderSettings {
                enabled: Some(true),
                tokens: Some(3000),
                ..Default::default()
            }),
            ..Default::default()
        }),
        ..Default::default()
    };
    assert_eq!(
        LongReasoningReminder {
            enabled: true,
            tokens: 3000,
            delay: 1
        },
        remote_on.resolve_long_reasoning_reminder(),
        "remote gate enables; remote tokens apply, delay falls to the default"
    );
    let toml_off = Config {
        long_reasoning_reminder: LongReasoningReminderSettings {
            enabled: Some(false),
            ..Default::default()
        },
        ..remote_on.clone()
    };
    assert_eq!(
        LongReasoningReminder {
            enabled: false,
            tokens: 3000,
            delay: 1
        },
        toml_off.resolve_long_reasoning_reminder(),
        "TOML false beats a remote true; remote tuning still resolves for telemetry"
    );
    let toml_on = Config {
        long_reasoning_reminder: LongReasoningReminderSettings {
            enabled: Some(true),
            tokens: Some(500),
            ..Default::default()
        },
        remote_settings: Some(crate::util::config::RemoteSettings {
            long_reasoning_reminder: Some(LongReasoningReminderSettings {
                enabled: Some(false),
                tokens: Some(3000),
                delay: Some(4),
            }),
            ..Default::default()
        }),
        ..Default::default()
    };
    assert_eq!(
        LongReasoningReminder {
            enabled: true,
            tokens: 500,
            delay: 4
        },
        toml_on.resolve_long_reasoning_reminder(),
        "TOML true beats a remote false; TOML tokens beat remote, remote delay fills in"
    );
    let _env = EnvGuard::set("CODEL_LONG_REASONING_REMINDER", "0");
    assert!(
        !toml_on.resolve_long_reasoning_reminder().enabled,
        "env kill switch wins over TOML + remote"
    );
    let _env = EnvGuard::set(
        "CODEL_LONG_REASONING_REMINDER",
        r#"{"enabled": true, "tokens": 9000}"#,
    );
    assert_eq!(
        LongReasoningReminder {
            enabled: true,
            tokens: 9000,
            delay: 1
        },
        toml_off.resolve_long_reasoning_reminder(),
        "env JSON enables over a TOML false and its tokens win; delay falls through"
    );
}
/// Gate precedence: env > `[doom_loop_recovery]` > remote settings > default(ON).
/// The remote layer merges PER-FIELD from the nested `doom_loop_recovery` object, and each layer's `false` is an independent kill switch.
/// One test covers the full ladder.
#[test]
#[serial]
fn resolve_doom_loop_recovery_precedence() {
    use crate::util::config::DoomLoopRecoverySettings;
    unsafe { std::env::remove_var("CODEL_DOOM_LOOP_RECOVERY") };
    let default_cfg = Config::default();
    let p = default_cfg
        .resolve_doom_loop_recovery()
        .expect("default is ON");
    assert_eq!(p.max_threshold, 64, "default tunables unchanged");
    assert_eq!(p.max_retries, 2, "default tunables unchanged");
    assert_eq!(p.window_tokens, 1024, "default tunables unchanged");
    let toml_off = Config {
        doom_loop_recovery: DoomLoopRecoverySettings {
            enabled: Some(false),
            ..Default::default()
        },
        ..Default::default()
    };
    assert!(
        toml_off.resolve_doom_loop_recovery().is_none(),
        "TOML kill switch"
    );
    let remote_off = Config {
        remote_settings: Some(crate::util::config::RemoteSettings {
            doom_loop_recovery: Some(DoomLoopRecoverySettings {
                enabled: Some(false),
                ..Default::default()
            }),
            ..Default::default()
        }),
        ..Default::default()
    };
    assert!(
        remote_off.resolve_doom_loop_recovery().is_none(),
        "remote settings kill switch"
    );
    unsafe { std::env::set_var("CODEL_DOOM_LOOP_RECOVERY", "0") };
    assert!(
        default_cfg.resolve_doom_loop_recovery().is_none(),
        "env kill switch"
    );
    unsafe { std::env::remove_var("CODEL_DOOM_LOOP_RECOVERY") };
    let remote_on = Config {
        remote_settings: Some(crate::util::config::RemoteSettings {
            doom_loop_recovery: Some(DoomLoopRecoverySettings {
                enabled: Some(true),
                max_threshold: Some(16),
                max_retries: Some(1),
                ..Default::default()
            }),
            ..Default::default()
        }),
        ..Default::default()
    };
    let p = remote_on.resolve_doom_loop_recovery().expect("remote on");
    assert_eq!(p.max_threshold, 16);
    assert_eq!(p.max_retries, 1);
    assert_eq!(p.window_tokens, 1024);
    let partial_remote = Config {
        remote_settings: Some(crate::util::config::RemoteSettings {
            doom_loop_recovery: Some(DoomLoopRecoverySettings {
                max_threshold: Some(16),
                ..Default::default()
            }),
            ..Default::default()
        }),
        ..Default::default()
    };
    let p = partial_remote
        .resolve_doom_loop_recovery()
        .expect("default-on gate despite remote object omitting enabled");
    assert_eq!(p.max_threshold, 16, "remote tunable applies");
    assert_eq!(p.max_retries, 2, "unset field falls to the default");
    assert_eq!(p.window_tokens, 1024, "unset field falls to the default");
    let config_over_remote = Config {
        doom_loop_recovery: DoomLoopRecoverySettings {
            enabled: Some(true),
            max_threshold: Some(4),
            max_retries: Some(3),
            ..Default::default()
        },
        remote_settings: Some(crate::util::config::RemoteSettings {
            doom_loop_recovery: Some(DoomLoopRecoverySettings {
                enabled: Some(false),
                max_threshold: Some(16),
                max_retries: Some(1),
                ..Default::default()
            }),
            ..Default::default()
        }),
        ..Default::default()
    };
    let p = config_over_remote
        .resolve_doom_loop_recovery()
        .expect("config on beats remote kill-switch");
    assert_eq!(p.max_threshold, 4);
    assert_eq!(p.max_retries, 3);
    unsafe { std::env::set_var("CODEL_DOOM_LOOP_RECOVERY", "0") };
    assert!(
        config_over_remote.resolve_doom_loop_recovery().is_none(),
        "env wins over config + remote"
    );
    unsafe { std::env::remove_var("CODEL_DOOM_LOOP_RECOVERY") };
}
/// The `[doom_loop_recovery]` TOML section deserializes through the standard config path (no bespoke parser).
#[test]
#[serial]
fn doom_loop_recovery_section_parses_from_toml() {
    unsafe { std::env::remove_var("CODEL_DOOM_LOOP_RECOVERY") };
    let raw: toml::Value = toml::from_str(
        r#"
            [doom_loop_recovery]
            enabled = true
            max_threshold = 12
            max_retries = 1
            "#,
    )
    .unwrap();
    let cfg = Config::new_from_toml_cfg(&raw).unwrap();
    assert_eq!(cfg.doom_loop_recovery.enabled, Some(true));
    let p = cfg.resolve_doom_loop_recovery().expect("enabled via toml");
    assert_eq!(p.max_threshold, 12);
    assert_eq!(p.max_retries, 1);
}
/// `[worktree.auto_gc]` deserializes through Config and resolve honors it.
#[test]
#[serial]
fn worktree_auto_gc_section_parses_from_toml() {
    unsafe { codel_fast_worktree::clear_auto_gc_env_for_test() };
    let raw: toml::Value = toml::from_str(
        r#"
            [worktree.auto_gc]
            enabled = true
            max_age_secs = 7200
            min_interval_secs = 120
            dry_run = true
            [worktree.auto_gc.max_age_by_kind]
            subagent = 3600
            manual = "never"
            "#,
    )
    .unwrap();
    let cfg = Config::new_from_toml_cfg(&raw).unwrap();
    assert_eq!(cfg.worktree.auto_gc.enabled, Some(true));
    assert_eq!(cfg.worktree.auto_gc.max_age_secs, Some(7200));
    let p = cfg.resolve_worktree_auto_gc();
    assert!(p.enabled);
    assert_eq!(p.max_age_secs, 7200);
    assert_eq!(p.min_interval_secs, 120);
    assert!(p.dry_run);
    assert_eq!(
        p.max_age_by_kind
            .get(&codel_fast_worktree::WorktreeKind::Subagent),
        Some(&Some(3600))
    );
    assert_eq!(
        p.max_age_by_kind
            .get(&codel_fast_worktree::WorktreeKind::Manual),
        Some(&None)
    );
}
/// Out-of-range tunables clamp instead of being honored or dropped.
#[test]
#[serial]
fn resolve_doom_loop_recovery_clamps_tunables() {
    use crate::util::config::DoomLoopRecoverySettings;
    unsafe { std::env::remove_var("CODEL_DOOM_LOOP_RECOVERY") };
    let cfg = Config {
        doom_loop_recovery: DoomLoopRecoverySettings {
            enabled: Some(true),
            max_threshold: Some(1_000),
            max_retries: Some(99),
            ..Default::default()
        },
        ..Default::default()
    };
    let p = cfg.resolve_doom_loop_recovery().expect("enabled");
    assert_eq!(p.max_threshold, 64);
    assert_eq!(p.max_retries, 5);
    let cfg = Config {
        doom_loop_recovery: DoomLoopRecoverySettings {
            enabled: Some(true),
            max_threshold: Some(0),
            max_retries: Some(0),
            ..Default::default()
        },
        ..Default::default()
    };
    let p = cfg.resolve_doom_loop_recovery().expect("enabled");
    assert_eq!(p.max_threshold, 2);
    assert_eq!(p.max_retries, 0, "0 retries is valid (observe-only)");
    for (raw, expected) in [
        (0, 4096),
        (100, 4096),
        (256, 4096),
        (512, 512),
        (1024, 1024),
        (4096, 4096),
        (99999, 4096),
    ] {
        let cfg = Config {
            doom_loop_recovery: DoomLoopRecoverySettings {
                enabled: Some(true),
                window_tokens: Some(raw),
                ..Default::default()
            },
            ..Default::default()
        };
        let p = cfg.resolve_doom_loop_recovery().expect("enabled");
        assert_eq!(p.window_tokens, expected, "window_tokens={raw}");
    }
}
#[test]
#[serial]
fn resolve_trace_upload_disabled_when_telemetry_off_despite_remote_flag() {
    unsafe { std::env::remove_var("CODEL_TELEMETRY_ENABLED") };
    unsafe { std::env::remove_var("CODEL_TELEMETRY_TRACE_UPLOAD") };
    let mut cfg = Config::default();
    cfg.features.telemetry = Some(TelemetryMode::Disabled);
    cfg.remote_settings = Some(crate::util::config::RemoteSettings {
        trace_upload_enabled: Some(true),
        ..Default::default()
    });
    let r = cfg.resolve_trace_upload();
    assert!(!r.value, "telemetry off must force trace upload off");
    assert!(!cfg.is_trace_upload_enabled());
}
#[test]
#[serial]
fn resolve_trace_upload_explicit_config_wins_over_telemetry_off() {
    unsafe { std::env::remove_var("CODEL_TELEMETRY_ENABLED") };
    unsafe { std::env::remove_var("CODEL_TELEMETRY_TRACE_UPLOAD") };
    let mut cfg = Config::default();
    cfg.features.telemetry = Some(TelemetryMode::Disabled);
    cfg.telemetry.trace_upload = Some(true);
    let r = cfg.resolve_trace_upload();
    assert!(
        r.value,
        "explicit trace_upload config wins over telemetry off"
    );
    assert_eq!(r.source, ConfigSource::Config);
    cfg.telemetry.trace_upload = None;
    cfg.requirements
        .trace_upload
        .pin(true, crate::config::RequirementSource::Unknown);
    assert!(cfg.resolve_trace_upload().value);
}
#[test]
#[serial]
fn trace_upload_decision_debug_reports_winning_source() {
    unsafe { std::env::remove_var("CODEL_TELEMETRY_ENABLED") };
    unsafe { std::env::remove_var("CODEL_TELEMETRY_TRACE_UPLOAD") };
    let mut cfg = Config::default();
    cfg.features.telemetry = Some(TelemetryMode::Disabled);
    cfg.remote_settings = Some(crate::util::config::RemoteSettings {
        trace_upload_enabled: Some(true),
        ..Default::default()
    });
    let d = cfg.trace_upload_decision_debug();
    assert_eq!(
        d.get("trace_upload"),
        Some(&serde_json::json!(serde_json::json!(false)))
    );
    assert_eq!(
        d.get("trace_upload_source"),
        Some(&serde_json::json!(serde_json::json!("default")))
    );
    assert_eq!(
        d.get("telemetry_mode"),
        Some(&serde_json::json!(serde_json::json!("false")))
    );
    assert_eq!(
        d.get("in_remote_trace_upload_enabled"),
        Some(&serde_json::json!(serde_json::json!(true)))
    );
    assert_eq!(
        d.get("has_remote_settings"),
        Some(&serde_json::json!(serde_json::json!(true)))
    );
    cfg.telemetry.trace_upload = Some(true);
    let d = cfg.trace_upload_decision_debug();
    assert_eq!(
        d.get("trace_upload"),
        Some(&serde_json::json!(serde_json::json!(true)))
    );
    assert_eq!(
        d.get("trace_upload_source"),
        Some(&serde_json::json!(serde_json::json!("config")))
    );
    assert_eq!(
        d.get("in_cfg_telemetry_trace_upload"),
        Some(&serde_json::json!(serde_json::json!(true)))
    );
}
#[test]
#[serial]
fn resolve_trace_upload_honors_config_when_telemetry_on() {
    unsafe { std::env::remove_var("CODEL_TELEMETRY_ENABLED") };
    unsafe { std::env::remove_var("DISABLE_TELEMETRY") };
    unsafe { std::env::remove_var("CODEL_TELEMETRY_TRACE_UPLOAD") };
    let mut cfg = Config::default();
    cfg.features.telemetry = Some(TelemetryMode::Enabled);
    cfg.telemetry.trace_upload = Some(false);
    let r = cfg.resolve_trace_upload();
    assert!(!r.value);
    assert_eq!(r.source, ConfigSource::Config);
    cfg.telemetry.trace_upload = None;
    let r = cfg.resolve_trace_upload();
    assert!(r.value, "defaults on when telemetry fully enabled");
}
#[test]
#[serial]
fn resolve_goal_defaults_to_true_when_unset() {
    unsafe { std::env::remove_var("CODEL_GOAL") };
    let cfg = Config::default();
    let r = cfg.resolve_goal();
    assert!(r.value, "goal should be on by default");
    assert_eq!(r.source, ConfigSource::Default);
}
#[test]
#[serial]
fn resolve_goal_env_overrides_config_without_remote_kill_switch() {
    unsafe { std::env::set_var("CODEL_GOAL", "1") };
    let mut cfg = Config::default();
    cfg.goal.enabled = Some(false);
    let r = cfg.resolve_goal();
    assert_eq!(r.source, ConfigSource::Env);
    assert!(r.value);
    unsafe { std::env::remove_var("CODEL_GOAL") };
}
#[test]
#[serial]
fn resolve_goal_remote_false_kills_local_opt_in() {
    unsafe { std::env::set_var("CODEL_GOAL", "1") };
    let mut cfg = Config::default();
    cfg.goal.enabled = Some(true);
    cfg.remote_settings = Some(crate::util::config::RemoteSettings {
        goal_enabled: Some(false),
        ..Default::default()
    });
    let r = cfg.resolve_goal();
    assert_eq!(r.source, ConfigSource::Remote);
    assert!(!r.value);
    unsafe { std::env::remove_var("CODEL_GOAL") };
}
#[test]
#[serial]
fn resolve_goal_remote_settings_used_when_no_local() {
    unsafe { std::env::remove_var("CODEL_GOAL") };
    let cfg = Config {
        remote_settings: Some(crate::util::config::RemoteSettings {
            goal_enabled: Some(true),
            ..Default::default()
        }),
        ..Default::default()
    };
    let r = cfg.resolve_goal();
    assert_eq!(r.source, ConfigSource::Remote);
    assert!(r.value);
}
/// The remote settings `goal_enabled: false` kill-switch must still win over the default-on fallback.
#[test]
#[serial]
fn resolve_goal_remote_settings_kill_switch_overrides_default_on() {
    unsafe { std::env::remove_var("CODEL_GOAL") };
    let cfg = Config {
        remote_settings: Some(crate::util::config::RemoteSettings {
            goal_enabled: Some(false),
            ..Default::default()
        }),
        ..Default::default()
    };
    let r = cfg.resolve_goal();
    assert_eq!(r.source, ConfigSource::Remote);
    assert!(!r.value);
}
#[test]
#[serial]
fn background_workflows_default_on_without_affecting_goal() {
    unsafe { std::env::remove_var("CODEL_WORKFLOWS") };
    let cfg = Config::default();
    let r = cfg.resolve_workflows();
    assert!(r.value);
    assert_eq!(r.source, ConfigSource::Default);
    assert!(cfg.resolve_goal().value);
}
#[test]
#[serial]
fn resolve_workflows_remote_settings_enables() {
    unsafe { std::env::remove_var("CODEL_WORKFLOWS") };
    let cfg = Config {
        remote_settings: Some(crate::util::config::RemoteSettings {
            workflows_enabled: Some(true),
            ..Default::default()
        }),
        ..Default::default()
    };
    let r = cfg.resolve_workflows();
    assert_eq!(r.source, ConfigSource::Remote);
    assert!(r.value);
}
#[test]
#[serial]
fn resolve_workflows_remote_false_kills_local_opt_in() {
    unsafe { std::env::set_var("CODEL_WORKFLOWS", "1") };
    let mut cfg = Config::default();
    cfg.workflows.enabled = Some(true);
    cfg.remote_settings = Some(crate::util::config::RemoteSettings {
        workflows_enabled: Some(false),
        ..Default::default()
    });
    let r = cfg.resolve_workflows();
    assert_eq!(r.source, ConfigSource::Remote);
    assert!(!r.value);
    unsafe { std::env::remove_var("CODEL_WORKFLOWS") };
}
#[test]
#[serial]
fn resolve_workflows_env_wins() {
    unsafe { std::env::set_var("CODEL_WORKFLOWS", "0") };
    let cfg = Config::default();
    let r = cfg.resolve_workflows();
    assert_eq!(r.source, ConfigSource::Env);
    assert!(
        !r.value,
        "env must be able to kill the default-on workflows"
    );
    unsafe { std::env::remove_var("CODEL_WORKFLOWS") };
}
#[test]
#[serial]
fn resolve_image_gen_model_override_remote_settings_or_config() {
    unsafe { std::env::remove_var("CODEL_IMAGE_GEN_MODEL_OVERRIDE") };
    let with = |config: Option<&str>, gb: Option<&str>| Config {
        features: Features {
            image_gen_model_override: config.map(String::from),
            ..Default::default()
        },
        remote_settings: Some(crate::util::config::RemoteSettings {
            image_gen_model_override: gb.map(String::from),
            ..Default::default()
        }),
        ..Default::default()
    };
    assert_eq!(Config::default().resolve_image_gen_model_override(), None);
    assert_eq!(
        with(None, Some("codel-imagine-image")).resolve_image_gen_model_override(),
        Some("codel-imagine-image".to_owned())
    );
    assert_eq!(
        with(Some("codel-imagine-image-pro"), Some("codel-imagine-image"))
            .resolve_image_gen_model_override(),
        Some("codel-imagine-image-pro".to_owned())
    );
}
#[test]
#[serial]
fn resolve_image_edit_model_override_remote_settings_or_config() {
    unsafe { std::env::remove_var("CODEL_IMAGE_EDIT_MODEL_OVERRIDE") };
    let with = |config: Option<&str>, gb: Option<&str>| Config {
        features: Features {
            image_edit_model_override: config.map(String::from),
            ..Default::default()
        },
        remote_settings: Some(crate::util::config::RemoteSettings {
            image_edit_model_override: gb.map(String::from),
            ..Default::default()
        }),
        ..Default::default()
    };
    assert_eq!(Config::default().resolve_image_edit_model_override(), None);
    assert_eq!(
        with(None, Some("codel-imagine-image")).resolve_image_edit_model_override(),
        Some("codel-imagine-image".to_owned())
    );
    assert_eq!(
        with(Some("codel-imagine-image-pro"), Some("codel-imagine-image"))
            .resolve_image_edit_model_override(),
        Some("codel-imagine-image-pro".to_owned())
    );
    let gen_only = Config {
        remote_settings: Some(crate::util::config::RemoteSettings {
            image_gen_model_override: Some("codel-imagine-image".to_owned()),
            ..Default::default()
        }),
        ..Default::default()
    };
    assert_eq!(gen_only.resolve_image_edit_model_override(), None);
}
#[test]
#[serial]
fn imagine_tools_disabled_gates_image_edit() {
    unsafe { std::env::remove_var("CODEL_IMAGE_EDIT") };
    let with_list = |tools: Vec<&str>| Config {
        remote_settings: Some(crate::util::config::RemoteSettings {
            imagine_tools_disabled: Some(tools.into_iter().map(String::from).collect()),
            ..Default::default()
        }),
        ..Default::default()
    };
    unsafe { std::env::set_var("CODEL_IMAGE_EDIT", "1") };
    let off = with_list(vec!["image_edit"]).resolve_image_edit();
    assert!(!off.value);
    assert_eq!(off.source, ConfigSource::Remote);
    unsafe { std::env::remove_var("CODEL_IMAGE_EDIT") };
    assert!(with_list(vec!["image_to_video"]).resolve_image_edit().value);
    assert!(Config::default().resolve_image_edit().value);
}
#[test]
#[serial]
fn resolve_image_gen_gates() {
    unsafe { std::env::remove_var("CODEL_IMAGE_GEN") };
    assert!(Config::default().resolve_image_gen().value);
    assert!(
        !Config {
            features: Features {
                image_gen: Some(false),
                ..Default::default()
            },
            ..Default::default()
        }
        .resolve_image_gen()
        .value
    );
    assert!(
        !Config {
            remote_settings: Some(crate::util::config::RemoteSettings {
                image_gen_enabled: Some(false),
                ..Default::default()
            }),
            ..Default::default()
        }
        .resolve_image_gen()
        .value
    );
    unsafe { std::env::set_var("CODEL_IMAGE_GEN", "1") };
    let denied = Config {
        remote_settings: Some(crate::util::config::RemoteSettings {
            imagine_tools_disabled: Some(vec!["image_gen".into()]),
            ..Default::default()
        }),
        ..Default::default()
    }
    .resolve_image_gen();
    assert!(!denied.value);
    assert_eq!(denied.source, ConfigSource::Remote);
    unsafe { std::env::remove_var("CODEL_IMAGE_GEN") };
}
#[test]
#[serial]
fn resolve_video_gen_gates() {
    unsafe { std::env::remove_var("CODEL_VIDEO_GEN") };
    assert!(Config::default().resolve_video_gen().value);
    assert!(
        !Config {
            features: Features {
                video_gen: Some(false),
                ..Default::default()
            },
            ..Default::default()
        }
        .resolve_video_gen()
        .value
    );
    assert!(
        !Config {
            remote_settings: Some(crate::util::config::RemoteSettings {
                video_gen_enabled: Some(false),
                ..Default::default()
            }),
            ..Default::default()
        }
        .resolve_video_gen()
        .value
    );
    assert!(
        !Config {
            remote_settings: Some(crate::util::config::RemoteSettings {
                imagine_tools_disabled: Some(vec!["image_to_video".into()]),
                ..Default::default()
            }),
            ..Default::default()
        }
        .resolve_video_gen()
        .value
    );
}
/// Clear every env var the goal/companion resolvers read so tests start from a known baseline regardless of run order.
fn clear_goal_envs() {
    unsafe {
        std::env::remove_var("CODEL_GOAL");
        std::env::remove_var("CODEL_GOAL_CLASSIFIER");
        std::env::remove_var("CODEL_GOAL_PLANNER");
        std::env::remove_var("CODEL_GOAL_SUMMARY");
        std::env::remove_var("CODEL_GOAL_VERIFIER_N");
        std::env::remove_var("CODEL_GOAL_CLASSIFIER_MAX");
        std::env::remove_var("CODEL_GOAL_STRATEGIST_EVERY");
        std::env::remove_var("CODEL_GOAL_REVERIFY_AFTER");
    }
}
fn cfg_with_goal(goal: bool) -> Config {
    Config {
        goal: GoalConfig {
            enabled: Some(goal),
            ..Default::default()
        },
        ..Default::default()
    }
}
fn cfg_with_goal_and_remote(goal: bool, remote: crate::util::config::RemoteSettings) -> Config {
    Config {
        goal: GoalConfig {
            enabled: Some(goal),
            ..Default::default()
        },
        remote_settings: Some(remote),
        ..Default::default()
    }
}
fn remote_classifier(v: bool) -> crate::util::config::RemoteSettings {
    crate::util::config::RemoteSettings {
        goal_classifier_enabled: Some(v),
        ..Default::default()
    }
}
fn remote_planner(v: bool) -> crate::util::config::RemoteSettings {
    crate::util::config::RemoteSettings {
        goal_planner_enabled: Some(v),
        ..Default::default()
    }
}
fn remote_summary(v: bool) -> crate::util::config::RemoteSettings {
    crate::util::config::RemoteSettings {
        goal_summary_enabled: Some(v),
        ..Default::default()
    }
}
fn cfg_with_goal_config(goal: GoalConfig) -> Config {
    Config {
        goal,
        ..Default::default()
    }
}
fn cfg_with_goal_config_and_remote(
    goal: GoalConfig,
    remote: crate::util::config::RemoteSettings,
) -> Config {
    Config {
        goal,
        remote_settings: Some(remote),
        ..Default::default()
    }
}
#[test]
#[serial]
fn resolve_goal_classifier_default_tracks_goal_enabled() {
    clear_goal_envs();
    assert!(
        !cfg_with_goal(false)
            .resolve_goal_classifier_enabled(false)
            .value
    );
    let on = cfg_with_goal(true).resolve_goal_classifier_enabled(true);
    assert!(on.value);
    assert_eq!(on.source, ConfigSource::Default);
    clear_goal_envs();
}
#[test]
#[serial]
fn resolve_goal_classifier_remote_forces_either_way() {
    clear_goal_envs();
    let off = cfg_with_goal_and_remote(true, remote_classifier(false))
        .resolve_goal_classifier_enabled(true);
    assert!(!off.value);
    assert_eq!(off.source, ConfigSource::Remote);
    let on = cfg_with_goal_and_remote(false, remote_classifier(true))
        .resolve_goal_classifier_enabled(false);
    assert!(on.value);
    assert_eq!(on.source, ConfigSource::Remote);
    clear_goal_envs();
}
#[test]
#[serial]
fn resolve_goal_classifier_env_overrides_default_and_remote() {
    clear_goal_envs();
    unsafe { std::env::set_var("CODEL_GOAL_CLASSIFIER", "0") };
    let r = cfg_with_goal_and_remote(true, remote_classifier(true))
        .resolve_goal_classifier_enabled(true);
    assert!(!r.value);
    assert_eq!(r.source, ConfigSource::Env);
    unsafe { std::env::set_var("CODEL_GOAL_CLASSIFIER", "1") };
    let r = cfg_with_goal_and_remote(false, remote_classifier(false))
        .resolve_goal_classifier_enabled(false);
    assert!(r.value);
    assert_eq!(r.source, ConfigSource::Env);
    clear_goal_envs();
}
#[test]
#[serial]
fn resolve_goal_planner_default_tracks_goal_enabled() {
    clear_goal_envs();
    assert!(
        !cfg_with_goal(false)
            .resolve_goal_planner_enabled(false)
            .value
    );
    let on = cfg_with_goal(true).resolve_goal_planner_enabled(true);
    assert!(on.value);
    assert_eq!(on.source, ConfigSource::Default);
    clear_goal_envs();
}
#[test]
#[serial]
fn resolve_goal_planner_remote_forces_either_way() {
    clear_goal_envs();
    let off =
        cfg_with_goal_and_remote(true, remote_planner(false)).resolve_goal_planner_enabled(true);
    assert!(!off.value);
    assert_eq!(off.source, ConfigSource::Remote);
    let on =
        cfg_with_goal_and_remote(false, remote_planner(true)).resolve_goal_planner_enabled(false);
    assert!(on.value);
    assert_eq!(on.source, ConfigSource::Remote);
    clear_goal_envs();
}
#[test]
#[serial]
fn resolve_goal_planner_env_overrides_default_and_remote() {
    clear_goal_envs();
    unsafe { std::env::set_var("CODEL_GOAL_PLANNER", "0") };
    let r = cfg_with_goal_and_remote(true, remote_planner(true)).resolve_goal_planner_enabled(true);
    assert!(!r.value);
    assert_eq!(r.source, ConfigSource::Env);
    unsafe { std::env::set_var("CODEL_GOAL_PLANNER", "1") };
    let r =
        cfg_with_goal_and_remote(false, remote_planner(false)).resolve_goal_planner_enabled(false);
    assert!(r.value);
    assert_eq!(r.source, ConfigSource::Env);
    clear_goal_envs();
}
#[test]
#[serial]
fn resolve_goal_summary_default_tracks_goal_enabled() {
    clear_goal_envs();
    assert!(
        !cfg_with_goal(false)
            .resolve_goal_summary_enabled(false)
            .value
    );
    let on = cfg_with_goal(true).resolve_goal_summary_enabled(true);
    assert!(on.value);
    assert_eq!(on.source, ConfigSource::Default);
    clear_goal_envs();
}
#[test]
#[serial]
fn resolve_goal_summary_remote_forces_either_way() {
    clear_goal_envs();
    let off =
        cfg_with_goal_and_remote(true, remote_summary(false)).resolve_goal_summary_enabled(true);
    assert!(!off.value);
    assert_eq!(off.source, ConfigSource::Remote);
    let on =
        cfg_with_goal_and_remote(false, remote_summary(true)).resolve_goal_summary_enabled(false);
    assert!(on.value);
    assert_eq!(on.source, ConfigSource::Remote);
    clear_goal_envs();
}
#[test]
#[serial]
fn resolve_goal_summary_env_overrides_default_and_remote() {
    clear_goal_envs();
    unsafe { std::env::set_var("CODEL_GOAL_SUMMARY", "0") };
    let r = cfg_with_goal_and_remote(true, remote_summary(true)).resolve_goal_summary_enabled(true);
    assert!(!r.value);
    assert_eq!(r.source, ConfigSource::Env);
    clear_goal_envs();
}
#[test]
#[serial]
fn resolve_goal_classifier_config_honored_when_env_unset() {
    clear_goal_envs();
    let r = cfg_with_goal_config(GoalConfig {
        classifier_enabled: Some(true),
        ..Default::default()
    })
    .resolve_goal_classifier_enabled(false);
    assert_eq!(r.source, ConfigSource::Config);
    assert!(r.value);
    clear_goal_envs();
}
#[test]
#[serial]
fn resolve_goal_classifier_env_beats_config() {
    clear_goal_envs();
    unsafe { std::env::set_var("CODEL_GOAL_CLASSIFIER", "0") };
    let r = cfg_with_goal_config(GoalConfig {
        classifier_enabled: Some(true),
        ..Default::default()
    })
    .resolve_goal_classifier_enabled(false);
    assert_eq!(r.source, ConfigSource::Env);
    assert!(!r.value);
    clear_goal_envs();
}
#[test]
#[serial]
fn resolve_goal_classifier_config_beats_remote() {
    clear_goal_envs();
    let r = cfg_with_goal_config_and_remote(
        GoalConfig {
            classifier_enabled: Some(true),
            ..Default::default()
        },
        remote_classifier(false),
    )
    .resolve_goal_classifier_enabled(false);
    assert_eq!(r.source, ConfigSource::Config);
    assert!(r.value);
    clear_goal_envs();
}
#[test]
#[serial]
fn resolve_goal_classifier_config_beats_default() {
    clear_goal_envs();
    let r = cfg_with_goal_config(GoalConfig {
        enabled: Some(true),
        classifier_enabled: Some(false),
        ..Default::default()
    })
    .resolve_goal_classifier_enabled(false);
    assert_eq!(r.source, ConfigSource::Config);
    assert!(!r.value);
    clear_goal_envs();
}
#[test]
#[serial]
fn resolve_goal_planner_config_honored_when_env_unset() {
    clear_goal_envs();
    let r = cfg_with_goal_config(GoalConfig {
        planner_enabled: Some(true),
        ..Default::default()
    })
    .resolve_goal_planner_enabled(false);
    assert_eq!(r.source, ConfigSource::Config);
    assert!(r.value);
    clear_goal_envs();
}
#[test]
#[serial]
fn resolve_goal_planner_env_beats_config() {
    clear_goal_envs();
    unsafe { std::env::set_var("CODEL_GOAL_PLANNER", "0") };
    let r = cfg_with_goal_config(GoalConfig {
        planner_enabled: Some(true),
        ..Default::default()
    })
    .resolve_goal_planner_enabled(false);
    assert_eq!(r.source, ConfigSource::Env);
    assert!(!r.value);
    clear_goal_envs();
}
#[test]
#[serial]
fn resolve_goal_planner_config_beats_remote() {
    clear_goal_envs();
    let r = cfg_with_goal_config_and_remote(
        GoalConfig {
            planner_enabled: Some(true),
            ..Default::default()
        },
        remote_planner(false),
    )
    .resolve_goal_planner_enabled(false);
    assert_eq!(r.source, ConfigSource::Config);
    assert!(r.value);
    clear_goal_envs();
}
#[test]
#[serial]
fn resolve_goal_planner_config_beats_default() {
    clear_goal_envs();
    let r = cfg_with_goal_config(GoalConfig {
        enabled: Some(true),
        planner_enabled: Some(false),
        ..Default::default()
    })
    .resolve_goal_planner_enabled(false);
    assert_eq!(r.source, ConfigSource::Config);
    assert!(!r.value);
    clear_goal_envs();
}
#[test]
#[serial]
fn resolve_goal_summary_config_honored_when_env_unset() {
    clear_goal_envs();
    let r = cfg_with_goal_config(GoalConfig {
        summary_enabled: Some(true),
        ..Default::default()
    })
    .resolve_goal_summary_enabled(false);
    assert_eq!(r.source, ConfigSource::Config);
    assert!(r.value);
    clear_goal_envs();
}
#[test]
#[serial]
fn resolve_goal_summary_env_beats_config() {
    clear_goal_envs();
    unsafe { std::env::set_var("CODEL_GOAL_SUMMARY", "0") };
    let r = cfg_with_goal_config(GoalConfig {
        summary_enabled: Some(true),
        ..Default::default()
    })
    .resolve_goal_summary_enabled(false);
    assert_eq!(r.source, ConfigSource::Env);
    assert!(!r.value);
    clear_goal_envs();
}
#[test]
#[serial]
fn resolve_goal_summary_config_beats_remote() {
    clear_goal_envs();
    let r = cfg_with_goal_config_and_remote(
        GoalConfig {
            summary_enabled: Some(true),
            ..Default::default()
        },
        remote_summary(false),
    )
    .resolve_goal_summary_enabled(false);
    assert_eq!(r.source, ConfigSource::Config);
    assert!(r.value);
    clear_goal_envs();
}
#[test]
#[serial]
fn resolve_goal_summary_config_beats_default() {
    clear_goal_envs();
    let r = cfg_with_goal_config(GoalConfig {
        enabled: Some(true),
        summary_enabled: Some(false),
        ..Default::default()
    })
    .resolve_goal_summary_enabled(false);
    assert_eq!(r.source, ConfigSource::Config);
    assert!(!r.value);
    clear_goal_envs();
}
#[test]
fn goal_keys_round_trip_from_toml() {
    let raw: toml::Value = toml::from_str(
        r#"
[goal]
enabled = true
classifier_enabled = true
planner_enabled = false
summary_enabled = true
verifier_count = 4
classifier_max_runs = 7
strategist_every = 3
reverify_after = 6
"#,
    )
    .expect("test TOML should parse");
    let cfg = Config::new_from_toml_cfg(&raw).expect("config should parse");
    assert_eq!(cfg.goal.enabled, Some(true));
    assert_eq!(cfg.goal.classifier_enabled, Some(true));
    assert_eq!(cfg.goal.planner_enabled, Some(false));
    assert_eq!(cfg.goal.summary_enabled, Some(true));
    assert_eq!(cfg.goal.verifier_count, Some(4));
    assert_eq!(cfg.goal.classifier_max_runs, Some(7));
    assert_eq!(cfg.goal.strategist_every, Some(3));
    assert_eq!(cfg.goal.reverify_after, Some(6));
    let empty = Config::new_from_toml_cfg(&toml::from_str("").unwrap()).unwrap();
    assert_eq!(empty.goal.classifier_enabled, None);
    assert_eq!(empty.goal.verifier_count, None);
}
const GOAL_USE_CURRENT_ENV: &str = "CODEL_GOAL_USE_CURRENT_MODEL_ONLY";
fn clear_goal_model_env() {
    unsafe { std::env::remove_var(GOAL_USE_CURRENT_ENV) };
}
fn planner_pair() -> crate::util::config::GoalRoleModel {
    crate::util::config::GoalRoleModel {
        model: "codel-4".to_string(),
        agent_type: "general-purpose".to_string(),
    }
}
fn strategist_pair() -> crate::util::config::GoalRoleModel {
    crate::util::config::GoalRoleModel {
        model: "codel-4.5".to_string(),
        agent_type: "cursor".to_string(),
    }
}
#[test]
#[serial]
fn goal_use_current_model_only_env_true() {
    clear_goal_model_env();
    unsafe { std::env::set_var(GOAL_USE_CURRENT_ENV, "1") };
    let r = Config::default().resolve_goal_use_current_model_only();
    assert!(r.value);
    assert_eq!(r.source, ConfigSource::Env);
    clear_goal_model_env();
}
#[test]
#[serial]
fn goal_use_current_model_only_config_true() {
    clear_goal_model_env();
    let cfg = cfg_with_goal_config(GoalConfig {
        use_current_model_only: Some(true),
        ..Default::default()
    });
    let r = cfg.resolve_goal_use_current_model_only();
    assert!(r.value);
    assert_eq!(r.source, ConfigSource::Config);
    clear_goal_model_env();
}
#[test]
#[serial]
fn goal_use_current_model_only_default_false() {
    clear_goal_model_env();
    let r = Config::default().resolve_goal_use_current_model_only();
    assert!(!r.value);
    assert_eq!(r.source, ConfigSource::Default);
    clear_goal_model_env();
}
#[test]
#[serial]
fn goal_use_current_model_only_env_overrides_config_false() {
    clear_goal_model_env();
    unsafe { std::env::set_var(GOAL_USE_CURRENT_ENV, "1") };
    let cfg = cfg_with_goal_config(GoalConfig {
        use_current_model_only: Some(false),
        ..Default::default()
    });
    let r = cfg.resolve_goal_use_current_model_only();
    assert!(r.value);
    assert_eq!(r.source, ConfigSource::Env);
    clear_goal_model_env();
}
fn remote_planner_model(
    p: crate::util::config::GoalRoleModel,
) -> crate::util::config::RemoteSettings {
    crate::util::config::RemoteSettings {
        goal_planner_model: Some(p),
        ..Default::default()
    }
}
fn remote_strategist_model(
    p: crate::util::config::GoalRoleModel,
) -> crate::util::config::RemoteSettings {
    crate::util::config::RemoteSettings {
        goal_strategist_model: Some(p),
        ..Default::default()
    }
}
#[test]
fn resolve_goal_planner_model_kill_switch_inherits() {
    let cfg = cfg_with_goal_config_and_remote(
        GoalConfig::default(),
        remote_planner_model(planner_pair()),
    );
    let r = cfg.resolve_goal_planner_model(true);
    assert_eq!(r.value, GoalRoleModelChoice::InheritCurrent);
    assert_eq!(r.source, ConfigSource::Config);
}
#[test]
fn resolve_goal_planner_model_remote_pair_explicit() {
    let cfg = cfg_with_goal_config_and_remote(
        GoalConfig::default(),
        remote_planner_model(planner_pair()),
    );
    let r = cfg.resolve_goal_planner_model(false);
    assert_eq!(r.value, GoalRoleModelChoice::Explicit(planner_pair()));
    assert_eq!(r.source, ConfigSource::Remote);
}
#[test]
fn resolve_goal_planner_model_config_overrides_remote() {
    let cfg = cfg_with_goal_config_and_remote(
        GoalConfig {
            planner_model: Some(planner_pair()),
            ..Default::default()
        },
        remote_planner_model(strategist_pair()),
    );
    let r = cfg.resolve_goal_planner_model(false);
    assert_eq!(r.value, GoalRoleModelChoice::Explicit(planner_pair()));
    assert_eq!(r.source, ConfigSource::Config);
}
#[test]
fn resolve_goal_planner_model_default_inherits() {
    let r = Config::default().resolve_goal_planner_model(false);
    assert_eq!(r.value, GoalRoleModelChoice::InheritCurrent);
    assert_eq!(r.source, ConfigSource::Default);
}
#[test]
fn resolve_goal_planner_model_remote_present_but_field_absent_inherits() {
    let cfg = cfg_with_goal_config_and_remote(
        GoalConfig::default(),
        remote_strategist_model(strategist_pair()),
    );
    let r = cfg.resolve_goal_planner_model(false);
    assert_eq!(r.value, GoalRoleModelChoice::InheritCurrent);
    assert_eq!(r.source, ConfigSource::Default);
}
#[test]
fn resolve_goal_strategist_model_remote_pair_explicit() {
    let cfg = cfg_with_goal_config_and_remote(
        GoalConfig::default(),
        remote_strategist_model(strategist_pair()),
    );
    let r = cfg.resolve_goal_strategist_model(false);
    assert_eq!(r.value, GoalRoleModelChoice::Explicit(strategist_pair()));
    assert_eq!(r.source, ConfigSource::Remote);
}
#[test]
fn resolve_goal_strategist_model_config_overrides_remote() {
    let cfg = cfg_with_goal_config_and_remote(
        GoalConfig {
            strategist_model: Some(strategist_pair()),
            ..Default::default()
        },
        remote_strategist_model(planner_pair()),
    );
    let r = cfg.resolve_goal_strategist_model(false);
    assert_eq!(r.value, GoalRoleModelChoice::Explicit(strategist_pair()));
    assert_eq!(r.source, ConfigSource::Config);
}
#[test]
fn resolve_goal_skeptic_models_kill_switch_inherits() {
    let cfg = cfg_with_goal_config(GoalConfig {
        skeptic_models: vec![planner_pair(), strategist_pair()],
        ..Default::default()
    });
    let r = cfg.resolve_goal_skeptic_models(true);
    assert!(r.value.is_empty(), "kill-switch ⇒ all skeptics inherit");
    assert_eq!(r.source, ConfigSource::Config);
}
#[test]
fn resolve_goal_skeptic_models_remote_pool_explicit() {
    let remote = crate::util::config::RemoteSettings {
        goal_skeptic_models: vec![planner_pair(), strategist_pair()],
        ..Default::default()
    };
    let r = cfg_with_goal_config_and_remote(GoalConfig::default(), remote)
        .resolve_goal_skeptic_models(false);
    assert_eq!(
        r.value,
        vec![
            GoalRoleModelChoice::Explicit(planner_pair()),
            GoalRoleModelChoice::Explicit(strategist_pair()),
        ]
    );
    assert_eq!(r.source, ConfigSource::Remote);
}
#[test]
fn resolve_goal_skeptic_models_config_pool_overrides_remote_pool() {
    let remote = crate::util::config::RemoteSettings {
        goal_skeptic_models: vec![strategist_pair(), strategist_pair()],
        ..Default::default()
    };
    let cfg = cfg_with_goal_config_and_remote(
        GoalConfig {
            skeptic_models: vec![planner_pair(), strategist_pair()],
            ..Default::default()
        },
        remote,
    );
    let r = cfg.resolve_goal_skeptic_models(false);
    assert_eq!(
        r.value,
        vec![
            GoalRoleModelChoice::Explicit(planner_pair()),
            GoalRoleModelChoice::Explicit(strategist_pair()),
        ]
    );
    assert_eq!(r.source, ConfigSource::Config);
}
#[test]
fn resolve_goal_skeptic_models_no_pool_inherits() {
    let r = Config::default().resolve_goal_skeptic_models(false);
    assert!(r.value.is_empty());
    assert_eq!(r.source, ConfigSource::Default);
}
/// `[goal]` model pins parse from both the inline-table and `[[...]]` array forms.
#[test]
fn goal_model_pins_parse_from_toml() {
    let toml_str = r#"
[goal]
enabled = true
planner_model = { model = "codel-build", agent_type = "codel-build-plan" }

[goal.strategist_model]
model = "test-model-fast"
agent_type = "cursor"

[[goal.skeptic_models]]
model = "codel-build"
agent_type = "codel-build-plan"

[[goal.skeptic_models]]
model = "test-model-fast"
agent_type = "cursor"
"#;
    let raw: toml::Value = toml::from_str(toml_str).unwrap();
    let cfg = Config::new_from_toml_cfg(&raw).unwrap();
    assert_eq!(cfg.goal.planner_model.as_ref().unwrap().model, "codel-build");
    assert_eq!(
        cfg.goal.strategist_model.as_ref().unwrap().agent_type,
        "cursor"
    );
    assert_eq!(cfg.goal.skeptic_models.len(), 2);
    assert_eq!(
        cfg.goal.skeptic_models.first().map(|m| m.model.as_str()),
        Some("codel-build")
    );
    assert_eq!(
        cfg.resolve_goal_planner_model(false).source,
        ConfigSource::Config
    );
}
/// A malformed pin must drop to `None`, not fail the whole parse (which would silently wipe every other setting).
#[test]
fn goal_model_pin_malformed_is_dropped_not_fatal() {
    let toml_str = r#"
[goal]
enabled = true
classifier_max_runs = 6
planner_model = { agent_type = "codel-build-plan" }
"#;
    let raw: toml::Value = toml::from_str(toml_str).unwrap();
    let cfg = Config::new_from_toml_cfg(&raw)
        .expect("malformed planner_model must not fail the whole parse");
    assert!(cfg.goal.planner_model.is_none());
    assert_eq!(cfg.goal.classifier_max_runs, Some(6));
}
#[test]
fn goal_skeptic_models_drop_malformed_entry_keep_rest() {
    let toml_str = r#"
[goal]
enabled = true

[[goal.skeptic_models]]
model = "codel-build"
agent_type = "codel-build-plan"

[[goal.skeptic_models]]
agent_type = "cursor"

[[goal.skeptic_models]]
model = "test-model-fast"
agent_type = "cursor"
"#;
    let raw: toml::Value = toml::from_str(toml_str).unwrap();
    let cfg = Config::new_from_toml_cfg(&raw).unwrap();
    assert_eq!(cfg.goal.skeptic_models.len(), 2);
    assert_eq!(
        cfg.goal.skeptic_models.first().map(|m| m.model.as_str()),
        Some("codel-build")
    );
    assert_eq!(
        cfg.goal.skeptic_models.get(1).map(|m| m.model.as_str()),
        Some("test-model-fast")
    );
}
/// Acceptance test: a full managed-config `[goal]` block resolves end-to-end, every value sourced from config (not remote/default).
#[test]
#[serial]
fn full_goal_managed_config_resolves_end_to_end() {
    clear_goal_envs();
    clear_goal_model_env();
    let raw: toml::Value = toml::from_str(
        r#"
[goal]
enabled = true
classifier_enabled = true
planner_enabled = true
verifier_count = 3
classifier_max_runs = 6
planner_model = { model = "codel-build", agent_type = "codel-build-plan" }
strategist_model = { model = "test-model-fast", agent_type = "cursor" }

[[goal.skeptic_models]]
model = "codel-build"
agent_type = "codel-build-plan"

[[goal.skeptic_models]]
model = "test-model-fast"
agent_type = "cursor"
"#,
    )
    .unwrap();
    let cfg = Config::new_from_toml_cfg(&raw).expect("[goal] config must parse");
    let codel_build = crate::util::config::GoalRoleModel {
        model: "codel-build".into(),
        agent_type: "codel-build-plan".into(),
    };
    let composer = crate::util::config::GoalRoleModel {
        model: "test-model-fast".into(),
        agent_type: "cursor".into(),
    };
    let goal_enabled = cfg.resolve_goal().value;
    assert!(goal_enabled);
    assert!(cfg.resolve_goal_classifier_enabled(goal_enabled).value);
    assert!(cfg.resolve_goal_planner_enabled(goal_enabled).value);
    assert_eq!(cfg.resolve_goal_verifier_count().value, 3);
    assert_eq!(cfg.resolve_goal_classifier_max_runs().value, 6);
    let use_current = cfg.resolve_goal_use_current_model_only().value;
    assert!(!use_current);
    let planner = cfg.resolve_goal_planner_model(use_current);
    assert_eq!(
        planner.value,
        GoalRoleModelChoice::Explicit(codel_build.clone())
    );
    assert_eq!(planner.source, ConfigSource::Config);
    assert_eq!(
        cfg.resolve_goal_strategist_model(use_current).value,
        GoalRoleModelChoice::Explicit(composer.clone())
    );
    assert_eq!(
        cfg.resolve_goal_skeptic_models(use_current).value,
        vec![
            GoalRoleModelChoice::Explicit(codel_build),
            GoalRoleModelChoice::Explicit(composer),
        ]
    );
    clear_goal_envs();
    clear_goal_model_env();
}
/// Run the production scan (`deserialize_collecting_unrecognized`) on a TOML string.
/// Mirrors the [model] removal and default-merge in `new_from_toml_cfg`.
fn unused_keys_from_toml(toml_str: &str) -> Vec<String> {
    let raw: toml::Value = toml::from_str(toml_str).unwrap();
    let raw_without_models = {
        let mut r = raw.clone();
        if let toml::Value::Table(ref mut t) = r {
            t.remove("model");
        }
        r
    };
    let mut base = toml::Value::try_from(Config::default()).unwrap();
    if let toml::Value::Table(ref mut t) = base {
        t.remove("model");
    }
    crate::config::deep_merge_toml(&mut base, &raw_without_models);
    let (_config, unused) = Config::deserialize_collecting_unrecognized(base, &raw_without_models)
        .expect("config should deserialize");
    unused
}
#[test]
fn config_warns_on_section_typo() {
    let raw: toml::Value = toml::from_str(
        r#"
            [endpoint]
            deployment_key = "codel-token-test"
        "#,
    )
    .unwrap();
    let config = Config::new_from_toml_cfg(&raw).expect("should parse");
    assert!(config.endpoints.deployment_key.is_none());
    let unused = unused_keys_from_toml(
        r#"
            [endpoint]
            deployment_key = "codel-token-test"
        "#,
    );
    assert!(unused.iter().any(|k| k == "endpoint"), "got: {unused:?}");
}
#[test]
fn known_non_serde_config_paths_are_not_reported_unused() {
    let unused = unused_keys_from_toml(
        r#"
            [features]
            remote_fetch = false
            session_search = false
            image_edit = true
            not_a_real_feature = true
            [slash_command_tags]
            workflows = "new"
            [marketplace]
            plugin_cta_marketplace = "Acme Marketplace"
            [cli]
            grove = true
            grove_worktree = "grove"
            nfs_worktree = true
        "#,
    );
    assert!(
        !unused.iter().any(|k| k == "features.remote_fetch"),
        "features.remote_fetch must not be treated as a typo: {unused:?}"
    );
    assert!(
        !unused
            .iter()
            .any(|k| k == "marketplace.plugin_cta_marketplace"),
        "the pager-read CTA marketplace override must not warn: {unused:?}"
    );
    assert!(
        !unused.iter().any(|k| k == "features.session_search"),
        "a registered feature has no typed field and must not look like a typo: {unused:?}"
    );
    assert!(
        !unused.iter().any(|k| k == "slash_command_tags"),
        "slash_command_tags is a real table: {unused:?}"
    );
    assert!(
        unused.iter().any(|k| k == "features.image_edit"),
        "only a pin sets image_edit, so a config entry stays unrecognized: {unused:?}"
    );
    assert!(
        unused.iter().any(|k| k == "features.not_a_real_feature"),
        "real typos still surface: {unused:?}"
    );
    for path in ["cli.grove", "cli.grove_worktree", "cli.nfs_worktree"] {
        assert!(
            !unused.iter().any(|k| k == path),
            "{path} is a raw Grove CLI key and must not look like a typo: {unused:?}"
        );
    }
}
/// `[toolset.web_search]`'s domain keys are read from the raw layers, not from `ShellToolsetConfig::web_search` (a `SamplerConfig`).
/// The scan must therefore not call the documented settings typos.
#[test]
fn web_search_domain_keys_are_not_reported_unused() {
    for key in ["allowed_domains", "excluded_domains"] {
        let unused = unused_keys_from_toml(&format!(
            r#"
                [toolset.web_search]
                {key} = ["docs.codel.dev"]
                not_a_real_key = true
            "#
        ));
        assert!(
            !unused
                .iter()
                .any(|k| k == &format!("toolset.web_search.{key}")),
            "toolset.web_search.{key} must not be treated as a typo: {unused:?}"
        );
        assert!(
            unused
                .iter()
                .any(|k| k == "toolset.web_search.not_a_real_key"),
            "real typos in the same section still surface: {unused:?}"
        );
    }
}
#[test]
fn config_warns_on_field_typos() {
    let unused = unused_keys_from_toml(
        r#"
            [endpoints]
            deplomyent_key = "test"
            [ui]
            yoloo = true
            [features]
            telmetry = true
        "#,
    );
    assert!(
        unused.iter().any(|k| k == "endpoints.deplomyent_key"),
        "got: {unused:?}"
    );
    assert!(unused.iter().any(|k| k == "ui.yoloo"), "got: {unused:?}");
    assert!(
        unused.iter().any(|k| k == "features.telmetry"),
        "got: {unused:?}"
    );
}
#[test]
fn config_accepts_all_known_sections() {
    let unused = unused_keys_from_toml(
        r#"
            disabled_mcp_servers = ["old-server"]
            [cli]
            auto_update = false
            [features]
            feedback = true
            [endpoints]
            deployment_key = "test"
            management_api_key = "mgmt-key"
            gcs_service_account_key = "gcs-key"
            [models]
            default = "codel-3"
            [ui]
            yolo = true
            theme = "dark"
            approval_mode = "ask"
            [session]
            auto_compact_threshold_percent = 85
            [telemetry]
            enabled = true
            trace_upload = true
            [agent]
            name = "custom"
            [skills]
            paths = ["~/skills"]
            [plugins]
            paths = ["~/plugins"]
            [subagents]
            enabled = true
            [memory]
            enabled = true
            [compaction]
            [compaction.pruning]
            enabled = true
            [harness]
            block_for_upload = true
            [feedback.user]
            name = ["os_user"]
            email = ["git_email", "team@example.com"]
            email_domain = "example.com"
            command = "/opt/bin/codel-identity"
            [repo_changes_dedup]
            enabled = false
            [relay]
            enabled = false
            [worktree_pool]
            pool_size = 4
            [managed_mcps]
            enabled = true
            [mcp_servers.test]
            url = "https://mcp.test.com"
            [toolset.bash]
            timeout_secs = 120
            login_shell_capture = true
            [codel_com_config]
            token_header = "test"
            [auth.oidc]
            issuer = "https://sso.corp.com"
            client_id = "abc123"
            [storage]
            cleanup_ttl_days = 7
            [[marketplace.sources]]
            name = "Local Dev"
            path = "/tmp/plugins"
            [permission]
            [[permission.rules]]
            action = "allow"
            tool = "bash"
            [tools]
            respect_gitignore = false
            [desktop]
            some_key = "value"
        "#,
    );
    assert!(
        unused.is_empty(),
        "false positive on valid config: {unused:?}"
    );
}
#[test]
fn config_accepts_compact_permission_section() {
    let unused = unused_keys_from_toml(
        r#"
            [permission]
            allow = ["Read(//tmp/**)"]
            deny = ["Bash(rm *)"]
            ask = ["WebFetch"]
        "#,
    );
    assert!(
        unused.is_empty(),
        "false positive on [permission] keys: {unused:?}"
    );
}
/// `prompt_policy` is not consumed from any TOML permission section, so it must warn rather than be a silent no-op.
/// The verbose loader keeps only `rules`; prompt policy comes from .claude settings `defaultMode`.
#[test]
fn permission_prompt_policy_warns_as_unconsumed() {
    let unused = unused_keys_from_toml(
        r#"
            [permission]
            deny = ["Bash(rm *)"]
            prompt_policy = "deny"
        "#,
    );
    assert_eq!(
        unused,
        vec!["permission.prompt_policy".to_string()],
        "an unconsumed key in a security section must be flagged"
    );
}
/// A typo'd `[permission]` sub-key must still warn: silently dropping a misspelled security rule would leave the user believing it's in force.
#[test]
fn permission_unknown_subkey_still_warns() {
    let unused = unused_keys_from_toml(
        r#"
            [permission]
            denny = ["Bash(rm *)"]
            ask = ["WebFetch"]
        "#,
    );
    assert_eq!(
        unused,
        vec!["permission.denny".to_string()],
        "exactly the typo'd sub-key must be flagged"
    );
}
/// Permission *values* are opaque: a malformed `[[permission.rules]]` entry neither warns nor fails Config load.
/// The out-of-band loaders parse it tolerantly and warn per item.
#[test]
fn malformed_permission_rules_do_not_fail_config_load() {
    let toml_str = r#"
            [[permission.rules]]
            pattern = 5
        "#;
    let raw: toml::Value = toml::from_str(toml_str).unwrap();
    Config::new_from_toml_cfg(&raw)
        .expect("malformed rule values are the permission loaders' concern");
    let unused = unused_keys_from_toml(toml_str);
    assert!(unused.is_empty(), "got: {unused:?}");
}
/// A non-table `[permission]` value still fails Config load: a fundamentally broken security section should be loud.
#[test]
fn non_table_permission_value_fails_config_load() {
    let raw: toml::Value = toml::from_str(r#"permission = "foo""#).unwrap();
    assert!(
        Config::new_from_toml_cfg(&raw).is_err(),
        "non-table [permission] must fail loudly"
    );
}
/// Wrong-typed values for the opaque passthrough keys must neither warn nor fail config load.
/// An admin typo in a managed layer must not brick startup fleet-wide; the out-of-band consumers degrade gracefully.
#[test]
fn wrong_typed_passthrough_values_neither_warn_nor_fail() {
    let toml_str = r#"
            [marketplace]
            official_marketplace_auto_installed = "yes"
            default_skills_installs_purged = "yes"
        "#;
    let unused = unused_keys_from_toml(toml_str);
    assert!(unused.is_empty(), "got: {unused:?}");
    let raw: toml::Value = toml::from_str(toml_str).unwrap();
    Config::new_from_toml_cfg(&raw)
        .expect("wrong-typed passthrough values must not fail config load");
}
/// Exempting `[permission]` and friends must not swallow warnings for genuinely unknown keys.
#[test]
fn unknown_key_still_warns_next_to_exempt_sections() {
    let unused = unused_keys_from_toml(
        r#"
            [permission]
            deny = ["Bash(rm *)"]
            [marketplace]
            official_marketplace_auto_installed = true
            default_skills_installs_purged = true
            [ui]
            yollo = true
        "#,
    );
    assert_eq!(
        unused,
        vec!["ui.yollo".to_string()],
        "exactly the typo'd key must be flagged"
    );
}
/// Regression: a deployment key with no OAuth token must resolve to Proxy.
#[test]
fn resolve_upload_method_accepts_deployment_key_without_oauth() {
    use crate::session::repo_changes::UploadMethod;
    let endpoints = EndpointsConfig {
        deployment_key: Some("enterprise-key".to_string()),
        ..Default::default()
    };
    match endpoints.resolve_upload_method(None) {
        Some(UploadMethod::Proxy {
            deployment_key,
            user_token,
            ..
        }) => {
            assert_eq!(deployment_key.as_deref(), Some("enterprise-key"));
            assert_eq!(user_token, "");
        }
        other => panic!("expected Proxy upload method, got {other:?}"),
    }
}
/// Base config for the internal-OTLP tests: pinned proxy, every OTLP knob explicitly unset so ambient env (via `Default`) can't leak in.
fn internal_otlp_test_config() -> EndpointsConfig {
    EndpointsConfig {
        cli_chat_proxy_base_url: Some("https://proxy.example/v1".to_string()),
        ..Default::default()
    }
}
fn empty_config() -> toml::Value {
    toml::Value::Table(toml::map::Map::new())
}
fn clear_runtime_env_vars() {
    unsafe {
        std::env::remove_var("CODEL_SUBAGENTS");
        std::env::remove_var("CODEL_RESPECT_GITIGNORE");
        std::env::remove_var("CODEL_WEB_SEARCH_MODEL");
        std::env::remove_var("CODEL_SESSION_SUMMARY_MODEL");
        std::env::remove_var("CODEL_CURSOR_SKILLS_ENABLED");
        std::env::remove_var("CODEL_CURSOR_RULES_ENABLED");
        std::env::remove_var("CODEL_CURSOR_AGENTS_ENABLED");
        std::env::remove_var("CODEL_CLAUDE_SKILLS_ENABLED");
        std::env::remove_var("CODEL_CLAUDE_RULES_ENABLED");
        std::env::remove_var("CODEL_CLAUDE_AGENTS_ENABLED");
    }
}
fn clear_managed_mcp_env_vars() {
    unsafe {
        std::env::remove_var("CODEL_MANAGED_MCPS_ENABLED");
        std::env::remove_var("CODEL_MANAGED_MCP_GATEWAY_TOOLS_ENABLED");
    }
}
fn isolate_compat_env() -> Vec<EnvGuard> {
    COMPAT_CELLS
        .into_iter()
        .map(|cell| EnvGuard::unset(cell.env_var()))
        .collect()
}
fn parse_compat(source: &str) -> CompatConfigToml {
    let raw: toml::Value = toml::from_str(source).unwrap();
    raw.get("compat").unwrap().clone().try_into().unwrap()
}
fn assert_session_one_disabled(config: CompatConfig, expected: CompatVendor) {
    for cell in COMPAT_CELLS {
        if cell.surface() == CompatSurface::Sessions {
            assert_eq!(
                config.value(cell),
                cell.vendor() != expected,
                "{}.sessions",
                Into::<&'static str>::into(cell.vendor())
            );
        }
    }
}
fn remote_settings_with(key: CompatRemoteKey, value: bool) -> crate::util::config::RemoteSettings {
    let mut remote = crate::util::config::RemoteSettings::default();
    match key {
        CompatRemoteKey::CursorSkills => remote.cursor_skills_enabled = Some(value),
        CompatRemoteKey::CursorRules => remote.cursor_rules_enabled = Some(value),
        CompatRemoteKey::CursorAgents => remote.cursor_agents_enabled = Some(value),
        CompatRemoteKey::CursorMcps => remote.cursor_mcps_enabled = Some(value),
        CompatRemoteKey::CursorHooks => remote.cursor_hooks_enabled = Some(value),
        CompatRemoteKey::CursorSessions => remote.cursor_sessions_enabled = Some(value),
        CompatRemoteKey::ClaudeSkills => remote.claude_skills_enabled = Some(value),
        CompatRemoteKey::ClaudeRules => remote.claude_rules_enabled = Some(value),
        CompatRemoteKey::ClaudeAgents => remote.claude_agents_enabled = Some(value),
        CompatRemoteKey::ClaudeMcps => remote.claude_mcps_enabled = Some(value),
        CompatRemoteKey::ClaudeHooks => remote.claude_hooks_enabled = Some(value),
        CompatRemoteKey::ClaudeSessions => remote.claude_sessions_enabled = Some(value),
        CompatRemoteKey::CodexSessions => remote.codex_sessions_enabled = Some(value),
    }
    remote
}
#[test]
#[serial]
fn resolve_compat_defaults_match_registry() {
    let _env = isolate_compat_env();
    assert_eq!(
        resolve_compat_config(&CompatConfigToml::default(), None),
        CompatConfig::default()
    );
}
#[test]
#[serial]
fn resolve_raw_compat_sessions_valid_empty_uses_remote_and_defaults() {
    let _env = isolate_compat_env();
    let raw = toml::Value::Table(Default::default());
    let remote = crate::util::config::RemoteSettings {
        claude_sessions_enabled: Some(false),
        ..Default::default()
    };
    let resolved = resolve_compat_sessions_from_raw(Ok(&raw), Some(&remote));
    assert!(resolved.cursor.sessions);
    assert!(!resolved.claude.sessions);
    assert!(resolved.codex.sessions);
}
#[test]
#[serial]
fn remote_keys_are_one_hot_and_false_overrides_default() {
    let _env = isolate_compat_env();
    for key in COMPAT_CELLS
        .into_iter()
        .filter_map(|cell| cell.remote_key())
    {
        let remote = remote_settings_with(key, false);
        for cell in COMPAT_CELLS {
            assert_eq!(
                remote_compat_value(Some(&remote), cell.remote_key()),
                (cell.remote_key() == Some(key)).then_some(false),
                "{key:?} mapped to {}.{}",
                Into::<&'static str>::into(cell.vendor()),
                Into::<&'static str>::into(cell.surface())
            );
        }
    }
    let remote = remote_settings_with(CompatRemoteKey::CursorSkills, false);
    assert!(CompatConfig::default().cursor.skills);
    assert!(
        !resolve_compat_config(&CompatConfigToml::default(), Some(&remote))
            .cursor
            .skills
    );
}
#[test]
#[serial]
fn resolve_compat_env_sessions_disable_independently() {
    let _env = isolate_compat_env();
    for (vendor, env_var) in [
        (CompatVendor::Cursor, "CODEL_CURSOR_SESSIONS_ENABLED"),
        (CompatVendor::Claude, "CODEL_CLAUDE_SESSIONS_ENABLED"),
        (CompatVendor::Codex, "CODEL_CODEX_SESSIONS_ENABLED"),
    ] {
        let _disabled = EnvGuard::set(env_var, "false");
        assert_session_one_disabled(
            resolve_compat_config(&CompatConfigToml::default(), None),
            vendor,
        );
    }
}
#[test]
#[serial]
fn resolve_compat_precedence_and_reserved_codex_hook() {
    let _env = isolate_compat_env();
    let config = parse_compat("[compat.cursor]\nsessions = false\n[compat.codex]\nhooks = false");
    let remote = crate::util::config::RemoteSettings {
        cursor_sessions_enabled: Some(true),
        ..Default::default()
    };
    let resolved = resolve_compat_config(&config, Some(&remote));
    assert!(!resolved.cursor.sessions);
    assert!(!resolved.codex.hooks);
    assert!(resolved.cursor.hooks);
    assert!(resolved.claude.hooks);
    let _session = EnvGuard::set("CODEL_CURSOR_SESSIONS_ENABLED", "true");
    let _hook = EnvGuard::set("CODEL_CODEX_HOOKS_ENABLED", "true");
    let resolved = resolve_compat_config(&config, Some(&remote));
    assert!(resolved.cursor.sessions);
    assert!(resolved.codex.hooks);
}
#[test]
#[serial]
fn resolve_runtime_fields_compat_asymmetric_sources() {
    let _env = isolate_compat_env();
    let _cursor = EnvGuard::set("CODEL_CURSOR_SESSIONS_ENABLED", "false");
    let raw: toml::Value =
        toml::from_str("[compat.cursor]\nsessions = true\n[compat.claude]\nsessions = false")
            .unwrap();
    let remote = crate::util::config::RemoteSettings {
        cursor_sessions_enabled: Some(true),
        claude_sessions_enabled: Some(true),
        codex_sessions_enabled: Some(false),
        ..Default::default()
    };
    let mut config = Config::new_from_toml_cfg(&raw).unwrap();
    config.resolve_runtime_fields(&RuntimeResolutionContext {
        raw_config: &raw,
        remote_settings: Some(&remote),
        is_headless: false,
        cli_subagents: None,
        cli_web_search_model: None,
        cli_session_summary_model: None,
        memory_enabled_override: None,
        disable_web_search: false,
        todo_gate: false,
        laziness_debug_log: None,
        storage_mode: None,
    });
    assert!(!config.compat_resolved.cursor.sessions);
    assert!(!config.compat_resolved.claude.sessions);
    assert!(!config.compat_resolved.codex.sessions);
}
#[test]
#[serial]
fn resolve_runtime_fields_interactive_defaults() {
    clear_runtime_env_vars();
    clear_managed_mcp_env_vars();
    let raw = empty_config();
    let mut cfg = Config::new_from_toml_cfg(&raw).unwrap();
    cfg.resolve_runtime_fields(&RuntimeResolutionContext {
        raw_config: &raw,
        remote_settings: None,
        is_headless: false,
        cli_subagents: None,
        cli_web_search_model: None,
        cli_session_summary_model: None,
        memory_enabled_override: None,
        disable_web_search: false,
        todo_gate: false,
        laziness_debug_log: None,
        storage_mode: None,
    });
    assert!(cfg.subagents_enabled);
    assert!(!cfg.respect_gitignore);
    assert!(cfg.managed_mcps_enabled);
    assert!(!cfg.managed_mcp_gateway_tools_enabled);
    assert_eq!(
        cfg.web_search_model,
        crate::models::default_web_search_model()
    );
    assert_eq!(
        cfg.session_summary_model,
        Some(crate::models::default_session_summary_model().to_owned())
    );
    assert!(!cfg.path_not_found_hints);
}
#[test]
#[serial]
fn resolve_runtime_fields_headless_defaults() {
    clear_runtime_env_vars();
    clear_managed_mcp_env_vars();
    let raw = empty_config();
    let mut cfg = Config::new_from_toml_cfg(&raw).unwrap();
    cfg.resolve_runtime_fields(&RuntimeResolutionContext {
        raw_config: &raw,
        remote_settings: None,
        is_headless: true,
        cli_subagents: None,
        cli_web_search_model: None,
        cli_session_summary_model: None,
        memory_enabled_override: None,
        disable_web_search: false,
        todo_gate: false,
        laziness_debug_log: None,
        storage_mode: None,
    });
    assert!(
        !cfg.managed_mcps_enabled,
        "headless should default managed_mcps to false"
    );
    assert!(!cfg.managed_mcp_gateway_tools_enabled);
}
#[test]
#[serial]
fn resolve_runtime_fields_managed_gateway_tools_from_remote() {
    clear_runtime_env_vars();
    clear_managed_mcp_env_vars();
    let raw = empty_config();
    let remote = crate::util::config::RemoteSettings {
        managed_mcp_gateway_tools_enabled: Some(true),
        ..Default::default()
    };
    let mut cfg = Config::new_from_toml_cfg(&raw).unwrap();
    cfg.resolve_runtime_fields(&RuntimeResolutionContext {
        raw_config: &raw,
        remote_settings: Some(&remote),
        is_headless: false,
        cli_subagents: None,
        cli_web_search_model: None,
        cli_session_summary_model: None,
        memory_enabled_override: None,
        disable_web_search: false,
        todo_gate: false,
        laziness_debug_log: None,
        storage_mode: None,
    });
    assert!(cfg.managed_mcp_gateway_tools_enabled);
}
#[test]
#[serial]
fn resolve_runtime_fields_subagents_from_config() {
    clear_runtime_env_vars();
    let raw: toml::Value = toml::from_str("[subagents]\nenabled = true").unwrap();
    let mut cfg = Config::new_from_toml_cfg(&raw).unwrap();
    cfg.resolve_runtime_fields(&RuntimeResolutionContext {
        raw_config: &raw,
        remote_settings: None,
        is_headless: false,
        cli_subagents: None,
        cli_web_search_model: None,
        cli_session_summary_model: None,
        memory_enabled_override: None,
        disable_web_search: false,
        todo_gate: false,
        laziness_debug_log: None,
        storage_mode: None,
    });
    assert!(cfg.subagents_enabled);
}
#[test]
#[serial]
fn resolve_runtime_fields_cli_subagents_override() {
    clear_runtime_env_vars();
    let raw = empty_config();
    let mut cfg = Config::new_from_toml_cfg(&raw).unwrap();
    cfg.resolve_runtime_fields(&RuntimeResolutionContext {
        raw_config: &raw,
        remote_settings: None,
        is_headless: false,
        cli_subagents: Some(true),
        cli_web_search_model: None,
        cli_session_summary_model: None,
        memory_enabled_override: None,
        disable_web_search: false,
        todo_gate: false,
        laziness_debug_log: None,
        storage_mode: None,
    });
    assert!(cfg.subagents_enabled);
}
#[test]
#[serial]
fn resolve_runtime_fields_cli_no_subagents_disables_over_config() {
    clear_runtime_env_vars();
    let raw: toml::Value = toml::from_str("[subagents]\nenabled = true").unwrap();
    let mut cfg = Config::new_from_toml_cfg(&raw).unwrap();
    cfg.resolve_runtime_fields(&RuntimeResolutionContext {
        raw_config: &raw,
        remote_settings: None,
        is_headless: false,
        cli_subagents: Some(false),
        cli_web_search_model: None,
        cli_session_summary_model: None,
        memory_enabled_override: None,
        disable_web_search: false,
        todo_gate: false,
        laziness_debug_log: None,
        storage_mode: None,
    });
    assert!(!cfg.subagents_enabled);
    assert_eq!(Some(false), cfg.cli_subagents);
}
#[test]
#[serial]
fn resolve_runtime_fields_partial_subagents_table_stays_enabled() {
    clear_runtime_env_vars();
    let raw: toml::Value = toml::from_str("[subagents]\nmax_depth = 2\n").unwrap();
    let mut cfg = Config::new_from_toml_cfg(&raw).unwrap();
    cfg.resolve_runtime_fields(&RuntimeResolutionContext {
        raw_config: &raw,
        remote_settings: None,
        is_headless: true,
        cli_subagents: None,
        cli_web_search_model: None,
        cli_session_summary_model: None,
        memory_enabled_override: None,
        disable_web_search: false,
        todo_gate: false,
        laziness_debug_log: None,
        storage_mode: None,
    });
    assert!(cfg.subagents_enabled);
    assert_eq!(2, cfg.subagents_max_depth);
}
#[test]
#[serial]
fn resolve_runtime_fields_gitignore_from_env() {
    clear_runtime_env_vars();
    unsafe { std::env::set_var("CODEL_RESPECT_GITIGNORE", "0") };
    let raw = empty_config();
    let mut cfg = Config::new_from_toml_cfg(&raw).unwrap();
    cfg.resolve_runtime_fields(&RuntimeResolutionContext {
        raw_config: &raw,
        remote_settings: None,
        is_headless: false,
        cli_subagents: None,
        cli_web_search_model: None,
        cli_session_summary_model: None,
        memory_enabled_override: None,
        disable_web_search: false,
        todo_gate: false,
        laziness_debug_log: None,
        storage_mode: None,
    });
    assert!(!cfg.respect_gitignore);
    clear_runtime_env_vars();
}
#[test]
#[serial]
fn resolve_runtime_fields_model_overrides_from_cli() {
    clear_runtime_env_vars();
    let raw = empty_config();
    let mut cfg = Config::new_from_toml_cfg(&raw).unwrap();
    cfg.resolve_runtime_fields(&RuntimeResolutionContext {
        raw_config: &raw,
        remote_settings: None,
        is_headless: false,
        cli_subagents: None,
        cli_web_search_model: Some("custom-ws"),
        cli_session_summary_model: Some("custom-ss"),
        memory_enabled_override: None,
        disable_web_search: false,
        todo_gate: false,
        laziness_debug_log: None,
        storage_mode: None,
    });
    assert_eq!(cfg.web_search_model, "custom-ws");
    assert_eq!(cfg.session_summary_model, Some("custom-ss".to_owned()));
}
#[test]
#[serial]
fn resolve_runtime_fields_path_hints_from_remote() {
    clear_runtime_env_vars();
    let raw = empty_config();
    let remote = crate::util::config::RemoteSettings {
        path_not_found_hints: Some(true),
        ..Default::default()
    };
    let mut cfg = Config::new_from_toml_cfg(&raw).unwrap();
    cfg.resolve_runtime_fields(&RuntimeResolutionContext {
        raw_config: &raw,
        remote_settings: Some(&remote),
        is_headless: false,
        cli_subagents: None,
        cli_web_search_model: None,
        cli_session_summary_model: None,
        memory_enabled_override: None,
        disable_web_search: false,
        todo_gate: false,
        laziness_debug_log: None,
        storage_mode: None,
    });
    assert!(cfg.path_not_found_hints);
}
#[test]
#[serial]
fn resolve_runtime_fields_idempotent() {
    clear_runtime_env_vars();
    let raw: toml::Value = toml::from_str("[subagents]\nenabled = true").unwrap();
    let mut cfg = Config::new_from_toml_cfg(&raw).unwrap();
    let ctx = RuntimeResolutionContext {
        raw_config: &raw,
        remote_settings: None,
        is_headless: false,
        cli_subagents: None,
        cli_web_search_model: None,
        cli_session_summary_model: None,
        memory_enabled_override: None,
        disable_web_search: false,
        todo_gate: false,
        laziness_debug_log: None,
        storage_mode: None,
    };
    cfg.resolve_runtime_fields(&ctx);
    let first_subagents = cfg.subagents_enabled;
    let first_gitignore = cfg.respect_gitignore;
    let first_mcps = cfg.managed_mcps_enabled;
    let first_ws = cfg.web_search_model.clone();
    cfg.resolve_runtime_fields(&ctx);
    assert_eq!(cfg.subagents_enabled, first_subagents);
    assert_eq!(cfg.respect_gitignore, first_gitignore);
    assert_eq!(cfg.managed_mcps_enabled, first_mcps);
    assert_eq!(cfg.web_search_model, first_ws);
}
#[test]
fn telemetry_mode_toml_roundtrip() {
    let cfg: Features = toml::from_str("telemetry = true").unwrap();
    assert_eq!(cfg.telemetry, Some(TelemetryMode::Enabled));
    let cfg: Features = toml::from_str("telemetry = false").unwrap();
    assert_eq!(cfg.telemetry, Some(TelemetryMode::Disabled));
    let cfg: Features = toml::from_str(r#"telemetry = "session_metrics""#).unwrap();
    assert_eq!(cfg.telemetry, Some(TelemetryMode::SessionMetrics));
    let cfg: Features =
        toml::from_str(r#"telemetry = "metrics_v3""#).expect("unknown string must not error");
    assert_eq!(cfg.telemetry, Some(TelemetryMode::Disabled));
    assert!(toml::from_str::<Features>("telemetry = 42").is_err());
}
#[test]
fn telemetry_enabled_from_toml_recognizes_modes() {
    let on: toml::Value = toml::from_str("[features]\ntelemetry = true\n").unwrap();
    assert_eq!(telemetry_enabled_from_toml(&on), Some(true));
    let session: toml::Value = toml::from_str(
        r#"[features]
telemetry = "session_metrics"
"#,
    )
    .unwrap();
    assert_eq!(telemetry_enabled_from_toml(&session), Some(true));
    let unknown: toml::Value = toml::from_str(
        r#"[features]
telemetry = "garbage"
"#,
    )
    .unwrap();
    assert_eq!(telemetry_enabled_from_toml(&unknown), None);
}
#[test]
fn global_extra_headers_apply_to_model_without_override() {
    let dm = crate::models::default_model();
    let (_, models) = resolve_models_from_toml(
        r#"
            [models]
            extra_headers = { "X-Request-Tags" = "team=example,env=prod" }
            "#,
        None,
    );
    let model = models.get(dm).expect("default model should exist");
    assert_eq!(
        model
            .info
            .extra_headers
            .get("X-Request-Tags")
            .map(String::as_str),
        Some("team=example,env=prod"),
        "global [models].extra_headers must apply to a model with no per-model override"
    );
}
#[test]
fn per_model_extra_headers_override_global_per_key() {
    let dm = crate::models::default_model();
    let (_, models) = resolve_models_from_toml(
        &format!(
            r#"
                [models]
                extra_headers = {{ "X-Request-Tags" = "team=example,env=staging", "X-Team" = "platform" }}

                [model."{dm}"]
                extra_headers = {{ "X-Request-Tags" = "team=example,env=prod" }}
                "#,
        ),
        None,
    );
    let model = models.get(dm).expect("default model should exist");
    assert_eq!(
        model
            .info
            .extra_headers
            .get("X-Request-Tags")
            .map(String::as_str),
        Some("team=example,env=prod"),
        "per-model extra_headers must override the global value for that key"
    );
    assert_eq!(
        model.info.extra_headers.get("X-Team").map(String::as_str),
        Some("platform"),
        "a global-only key must still be inherited when a model overrides a different key"
    );
}
#[test]
fn per_model_extra_headers_override_global_case_insensitively() {
    let dm = crate::models::default_model();
    let (_, models) = resolve_models_from_toml(
        &format!(
            r#"
                [models]
                extra_headers = {{ "X-Request-Tags" = "global" }}

                [model."{dm}"]
                extra_headers = {{ "x-request-tags" = "permodel" }}
                "#,
        ),
        None,
    );
    let model = models.get(dm).expect("default model should exist");
    let cost_tags: Vec<&str> = model
        .info
        .extra_headers
        .iter()
        .filter(|(k, _)| k.eq_ignore_ascii_case("x-request-tags"))
        .map(|(_, v)| v.as_str())
        .collect();
    assert_eq!(
        cost_tags,
        vec!["permodel"],
        "per-model value must win case-insensitively, with no global case-variant duplicate"
    );
    assert!(
        !model.info.extra_headers.contains_key("X-Request-Tags"),
        "global \"X-Request-Tags\" must not co-exist with per-model \"x-request-tags\""
    );
}
#[test]
fn config_model_reasoning_efforts_parses_inline_tables_and_bare_strings() {
    let raw_config: toml::Value = toml::from_str(
        r#"
            [model.custom]
            model = "custom"
            base_url = "https://api.example.com/v1"
            context_window = 200000
            reasoning_efforts = [
                { value = "high", label = "High", default = true },
                { id = "deep", value = "xhigh", label = "Deep", description = "Max" },
            ]

            [model.shorthand]
            model = "shorthand"
            base_url = "https://api.example.com/v1"
            context_window = 200000
            reasoning_efforts = ["low", "high"]
            "#,
    )
    .unwrap();
    let cfg = Config::new_from_toml_cfg(&raw_config).expect("config should parse");
    let resolved = resolve_model_list(&cfg, None);
    let custom = &resolved.get("custom").expect("custom model").info;
    let [e0, e1] = custom.reasoning_efforts.as_slice() else {
        panic!(
            "expected two reasoning efforts: {:?}",
            custom.reasoning_efforts
        );
    };
    assert_eq!(e0.label, "High");
    assert!(e0.default);
    assert_eq!(e1.id, "deep");
    assert_eq!(e1.value, ReasoningEffort::Xhigh);
    let shorthand = &resolved.get("shorthand").expect("shorthand model").info;
    let ids: Vec<_> = shorthand
        .reasoning_efforts
        .iter()
        .map(|o| o.id.as_str())
        .collect();
    assert_eq!(ids, ["low", "high"]);
    assert_eq!(
        shorthand
            .reasoning_efforts
            .first()
            .map(|e| e.label.as_str()),
        Some("Low")
    );
}
/// Resolves `config_toml` (rows pointing at `model = "codel-4.6-build"`) against `donor` prefetched under the wire id.
/// A custom models endpoint keeps the built-in `codel-4.6` catalog row (which has its own menu) out of Layer 1.
fn resolve_with_menu_donor(config_toml: &str, donor: ModelEntry) -> IndexMap<String, ModelEntry> {
    let raw: toml::Value = toml::from_str(config_toml).unwrap();
    let mut cfg = Config::new_from_toml_cfg(&raw).expect("config should parse");
    cfg.endpoints.models_base_url = Some("https://test.example.com/v1".to_owned());
    let mut prefetched = IndexMap::new();
    prefetched.insert("codel-4.6-build".to_owned(), donor);
    resolve_model_list(&cfg, Some(prefetched))
}
/// Resolves a single `[model."{key}"]` row (`model = "codel-4.6-build"`, no menu) against `donor`; `key` is the
/// wire id itself or an alias of it.
fn resolve_row_with_menu_donor(key: &str, extra_toml: &str, donor: ModelEntry) -> ModelEntry {
    let config_toml = format!(
        r#"
            [model."{key}"]
            model = "codel-4.6-build"
            base_url = "https://test.example.com/v1"
            {extra_toml}
            "#
    );
    resolve_with_menu_donor(&config_toml, donor)
        .shift_remove(key)
        .expect("config key must exist")
}
fn effort_ids(info: &ModelInfo) -> Vec<&str> {
    info.reasoning_efforts
        .iter()
        .map(|o| o.id.as_str())
        .collect()
}
/// Inheriting an unmarked `capabilities` menu must carry the server-default flag along, or the alias would
/// derive `.first()` (`low`) where the same-key row sends nothing.
#[test]
fn slug_inherited_unmarked_capabilities_menu_keeps_no_default_effort() {
    let row = serde_json::json!({
        "id": "codel-4.6-build",
        "capabilities": { "reasoning_effort": ["low", "medium", "high", "xhigh"] }
    });
    let parsed =
        crate::remote::client::parse_remote_model_value(&row, "https://test.example.com/v1")
            .expect("row parses");
    let entry = resolve_row_with_menu_donor("codel-4.6", "", ModelEntry::from_config_entry(&parsed));
    assert_eq!(effort_ids(&entry.info), ["low", "medium", "high", "xhigh"]);
    assert!(entry.info.supports_reasoning_effort);
    assert!(entry.info.reasoning_effort_server_default);
    assert_eq!(entry.info.reasoning_effort, None);
    assert_eq!(resolve_sampling(&entry, None).reasoning_effort, None);
}
/// A `/v1/models` row whose `capabilities` names no default keeps the menu but sends no effort, so the
/// server applies its own instead of the lowest listed tier.
#[test]
fn capabilities_menu_without_default_resolves_to_no_reasoning_effort() {
    let mut cfg = Config::default();
    cfg.endpoints.models_base_url = Some("https://test.example.com/v1".to_owned());
    let row = serde_json::json!({
        "id": "codel-4.6-build",
        "capabilities": { "reasoning_effort": ["low", "medium", "high", "xhigh"] }
    });
    let parsed =
        crate::remote::client::parse_remote_model_value(&row, "https://test.example.com/v1")
            .expect("row parses");
    let mut prefetched = IndexMap::new();
    prefetched.insert(
        "codel-4.6-build".to_owned(),
        ModelEntry::from_config_entry(&parsed),
    );
    let info = resolve_model_list(&cfg, Some(prefetched))
        .shift_remove("codel-4.6-build")
        .expect("codel-4.6-build key must exist")
        .info;
    assert_eq!(info.reasoning_efforts.len(), 4);
    assert!(info.supports_reasoning_effort);
    assert_eq!(info.reasoning_effort, None);
}
#[test]
fn hub_config_default_has_no_url() {
    assert!(HubConfig::default().url.is_none());
    assert!(!HubConfig::default().is_enabled());
}
#[test]
fn hub_config_is_enabled_only_for_nonempty_url() {
    assert!(
        HubConfig {
            url: Some("wss://hub.example/ws".into()),
        }
        .is_enabled()
    );
    assert!(
        !HubConfig {
            url: Some("   ".into()),
        }
        .is_enabled()
    );
}
#[test]
fn resolve_model_list_prunes_bundled_entries_not_in_prefetch() {
    let cfg = Config::default();
    let dm = crate::models::default_model();
    let mut defs = default_model_entries(&EndpointsConfig::default());
    let mut p = IndexMap::new();
    if let Some(e) = defs.shift_remove(dm) {
        p.insert(dm.to_string(), e);
    }
    let resolved = resolve_model_list(&cfg, Some(p));
    assert!(resolved.contains_key(dm));
    let no_p = resolve_model_list(&cfg, None);
    assert!(no_p.contains_key(dm));
}
#[test]
fn resolve_model_list_prefetch_visibility_matches_auth_and_server_list() {
    let cfg = Config::default();
    let dm = crate::models::default_model();
    let mut defs = default_model_entries(&EndpointsConfig::default());
    let mut p = IndexMap::new();
    if let Some(e) = defs.shift_remove(dm) {
        p.insert(dm.to_string(), e);
    }
    let resolved = resolve_model_list(&cfg, Some(p));
    let sess: Vec<_> = resolved
        .values()
        .filter(|e| e.visible_for_auth(true))
        .collect();
    let api: Vec<_> = resolved
        .values()
        .filter(|e| e.visible_for_auth(false))
        .collect();
    assert_eq!(sess.len(), 1);
    assert_eq!(api.len(), 1);
}
#[test]
fn resolve_model_list_empty_prefetch_yields_empty_base() {
    let cfg = Config::default();
    let resolved = resolve_model_list(&cfg, Some(IndexMap::new()));
    assert!(resolved.is_empty());
}
/// Guard: a config overlay WITHOUT credentials must NOT flip the bundled supported_in_api flag.
/// Only BYOK triggers that override.
#[test]
fn plain_config_overlay_preserves_bundled_visibility() {
    let dm = crate::models::default_model();
    let bundled = default_model_entries(&EndpointsConfig::default())
        .get(dm)
        .expect("bundled default must exist")
        .clone();
    let raw: toml::Value = toml::from_str(&format!(
        r#"
            [model."{dm}"]
            context_window = 300000
            "#
    ))
    .unwrap();
    let cfg = Config::new_from_toml_cfg(&raw).expect("config should parse");
    let resolved = resolve_model_list(&cfg, None);
    let entry = resolved.get(dm).expect("bundled default must exist");
    assert_eq!(
        entry.visible_for_auth(false),
        bundled.visible_for_auth(false),
        "non-BYOK config overlay must preserve bundled supported_in_api"
    );
    assert_eq!(
        entry.visible_for_auth(true),
        bundled.visible_for_auth(true),
        "non-BYOK config overlay must preserve bundled OAuth visibility"
    );
}
#[test]
#[serial]
fn mcp_liveness_watchers_default_is_true() {
    unsafe { std::env::remove_var("CODEL_MCP_LIVENESS_WATCHERS") };
    let r = resolve_mcp_liveness_watchers(None, None, None, None, None);
    assert!(r.value, "default-on by spec");
    assert_eq!(r.source, ConfigSource::Default);
}
#[test]
#[serial]
fn mcp_liveness_watchers_requirement_wins_over_everything() {
    unsafe { std::env::set_var("CODEL_MCP_LIVENESS_WATCHERS", "true") };
    let r =
        resolve_mcp_liveness_watchers(Some(false), Some(true), Some(true), Some(true), Some(true));
    unsafe { std::env::remove_var("CODEL_MCP_LIVENESS_WATCHERS") };
    assert!(!r.value, "requirement overrides every other layer");
    assert_eq!(r.source, ConfigSource::Requirement);
}
#[test]
#[serial]
fn mcp_liveness_watchers_cli_wins_over_env_and_below() {
    unsafe { std::env::set_var("CODEL_MCP_LIVENESS_WATCHERS", "true") };
    let r = resolve_mcp_liveness_watchers(None, Some(false), Some(true), Some(true), Some(true));
    unsafe { std::env::remove_var("CODEL_MCP_LIVENESS_WATCHERS") };
    assert!(!r.value);
    assert_eq!(r.source, ConfigSource::Cli);
}
#[test]
#[serial]
fn mcp_liveness_watchers_env_wins_over_config_and_below() {
    unsafe { std::env::set_var("CODEL_MCP_LIVENESS_WATCHERS", "false") };
    let r = resolve_mcp_liveness_watchers(None, None, Some(true), Some(true), Some(true));
    unsafe { std::env::remove_var("CODEL_MCP_LIVENESS_WATCHERS") };
    assert!(!r.value);
    assert_eq!(r.source, ConfigSource::Env);
}
#[test]
#[serial]
fn mcp_liveness_watchers_config_wins_over_managed_and_feature_flag() {
    unsafe { std::env::remove_var("CODEL_MCP_LIVENESS_WATCHERS") };
    let r = resolve_mcp_liveness_watchers(None, None, Some(false), Some(true), Some(true));
    assert!(!r.value);
    assert_eq!(r.source, ConfigSource::Config);
}
#[test]
#[serial]
fn mcp_liveness_watchers_managed_wins_over_feature_flag() {
    unsafe { std::env::remove_var("CODEL_MCP_LIVENESS_WATCHERS") };
    let r = resolve_mcp_liveness_watchers(None, None, None, Some(false), Some(true));
    assert!(!r.value);
    assert_eq!(r.source, ConfigSource::ManagedConfig);
}
#[test]
#[serial]
fn mcp_liveness_watchers_feature_flag_used_when_no_higher_layer() {
    unsafe { std::env::remove_var("CODEL_MCP_LIVENESS_WATCHERS") };
    let r = resolve_mcp_liveness_watchers(None, None, None, None, Some(false));
    assert!(!r.value);
    assert_eq!(r.source, ConfigSource::Remote);
}
#[test]
#[serial]
fn mcp_auto_restart_default_is_true() {
    unsafe { std::env::remove_var("CODEL_MCP_AUTO_RESTART") };
    let r = resolve_mcp_auto_restart(None, None, None, None, None);
    assert!(r.value, "recovery is on by default");
    assert_eq!(r.source, ConfigSource::Default);
}
#[test]
#[serial]
fn mcp_auto_restart_requirement_wins_over_everything() {
    unsafe { std::env::set_var("CODEL_MCP_AUTO_RESTART", "false") };
    let r = resolve_mcp_auto_restart(
        Some(true),
        Some(false),
        Some(false),
        Some(false),
        Some(false),
    );
    unsafe { std::env::remove_var("CODEL_MCP_AUTO_RESTART") };
    assert!(r.value);
    assert_eq!(r.source, ConfigSource::Requirement);
}
#[test]
#[serial]
fn mcp_auto_restart_env_wins_over_config_and_below() {
    unsafe { std::env::set_var("CODEL_MCP_AUTO_RESTART", "true") };
    let r = resolve_mcp_auto_restart(None, None, Some(false), Some(false), Some(false));
    unsafe { std::env::remove_var("CODEL_MCP_AUTO_RESTART") };
    assert!(r.value);
    assert_eq!(r.source, ConfigSource::Env);
}
#[test]
#[serial]
fn turn_transient_retry_default_is_true() {
    unsafe { std::env::remove_var("CODEL_TURN_TRANSIENT_RETRY") };
    let r = resolve_turn_transient_retry(None, None, None, None, None);
    assert!(r.value, "transient retry is on by default");
    assert_eq!(r.source, ConfigSource::Default);
}
#[test]
#[serial]
fn turn_transient_retry_config_kill_switch() {
    unsafe { std::env::remove_var("CODEL_TURN_TRANSIENT_RETRY") };
    let r = resolve_turn_transient_retry(None, None, Some(false), None, None);
    assert!(
        !r.value,
        "config `[features] turn_transient_retry = false` disables"
    );
    assert_eq!(r.source, ConfigSource::Config);
}
#[test]
#[serial]
fn turn_transient_retry_remote_flag_disables_below_config() {
    unsafe { std::env::remove_var("CODEL_TURN_TRANSIENT_RETRY") };
    let r = resolve_turn_transient_retry(None, None, None, None, Some(false));
    assert!(!r.value);
    assert_eq!(r.source, ConfigSource::Remote);
    let r = resolve_turn_transient_retry(None, None, Some(true), None, Some(false));
    assert!(r.value);
    assert_eq!(r.source, ConfigSource::Config);
    let r = resolve_turn_transient_retry(None, None, Some(false), None, Some(true));
    assert!(!r.value);
    assert_eq!(r.source, ConfigSource::Config);
}
#[test]
#[serial]
fn turn_transient_retry_env_wins_over_config() {
    unsafe { std::env::set_var("CODEL_TURN_TRANSIENT_RETRY", "false") };
    let r = resolve_turn_transient_retry(None, None, Some(true), None, None);
    unsafe { std::env::remove_var("CODEL_TURN_TRANSIENT_RETRY") };
    assert!(!r.value);
    assert_eq!(r.source, ConfigSource::Env);
}
#[test]
#[serial]
fn mcp_push_server_status_default_is_true() {
    unsafe { std::env::remove_var("CODEL_MCP_PUSH_SERVER_STATUS") };
    let r = resolve_mcp_push_server_status(None, None, None, None, None);
    assert!(r.value, "default-on by spec");
    assert_eq!(r.source, ConfigSource::Default);
}
#[test]
#[serial]
fn mcp_push_server_status_requirement_wins_over_everything() {
    unsafe { std::env::set_var("CODEL_MCP_PUSH_SERVER_STATUS", "true") };
    let r =
        resolve_mcp_push_server_status(Some(false), Some(true), Some(true), Some(true), Some(true));
    unsafe { std::env::remove_var("CODEL_MCP_PUSH_SERVER_STATUS") };
    assert!(!r.value, "requirement overrides every other layer");
    assert_eq!(r.source, ConfigSource::Requirement);
}
#[test]
#[serial]
fn mcp_push_server_status_cli_wins_over_env_and_below() {
    unsafe { std::env::set_var("CODEL_MCP_PUSH_SERVER_STATUS", "true") };
    let r = resolve_mcp_push_server_status(None, Some(false), Some(true), Some(true), Some(true));
    unsafe { std::env::remove_var("CODEL_MCP_PUSH_SERVER_STATUS") };
    assert!(!r.value);
    assert_eq!(r.source, ConfigSource::Cli);
}
#[test]
#[serial]
fn mcp_push_server_status_env_wins_over_config_and_below() {
    unsafe { std::env::set_var("CODEL_MCP_PUSH_SERVER_STATUS", "false") };
    let r = resolve_mcp_push_server_status(None, None, Some(true), Some(true), Some(true));
    unsafe { std::env::remove_var("CODEL_MCP_PUSH_SERVER_STATUS") };
    assert!(!r.value);
    assert_eq!(r.source, ConfigSource::Env);
}
#[test]
#[serial]
fn mcp_push_server_status_config_wins_over_managed_and_feature_flag() {
    unsafe { std::env::remove_var("CODEL_MCP_PUSH_SERVER_STATUS") };
    let r = resolve_mcp_push_server_status(None, None, Some(false), Some(true), Some(true));
    assert!(!r.value);
    assert_eq!(r.source, ConfigSource::Config);
}
#[test]
#[serial]
fn mcp_push_server_status_managed_wins_over_feature_flag() {
    unsafe { std::env::remove_var("CODEL_MCP_PUSH_SERVER_STATUS") };
    let r = resolve_mcp_push_server_status(None, None, None, Some(false), Some(true));
    assert!(!r.value);
    assert_eq!(r.source, ConfigSource::ManagedConfig);
}
#[test]
#[serial]
fn mcp_push_server_status_feature_flag_used_when_no_higher_layer() {
    unsafe { std::env::remove_var("CODEL_MCP_PUSH_SERVER_STATUS") };
    let r = resolve_mcp_push_server_status(None, None, None, None, Some(false));
    assert!(!r.value);
    assert_eq!(r.source, ConfigSource::Remote);
}
#[test]
#[serial]
fn mcp_recursive_config_watch_default_is_true() {
    unsafe { std::env::remove_var("CODEL_MCP_RECURSIVE_CONFIG_WATCH") };
    let r = resolve_mcp_recursive_config_watch(None, None, None, None, None);
    assert!(r.value, "default-on by spec");
    assert_eq!(r.source, ConfigSource::Default);
}
#[test]
#[serial]
fn mcp_recursive_config_watch_requirement_wins_over_everything() {
    unsafe { std::env::set_var("CODEL_MCP_RECURSIVE_CONFIG_WATCH", "true") };
    let r = resolve_mcp_recursive_config_watch(
        Some(false),
        Some(true),
        Some(true),
        Some(true),
        Some(true),
    );
    unsafe { std::env::remove_var("CODEL_MCP_RECURSIVE_CONFIG_WATCH") };
    assert!(!r.value, "requirement overrides every other layer");
    assert_eq!(r.source, ConfigSource::Requirement);
}
#[test]
#[serial]
fn mcp_recursive_config_watch_cli_wins_over_env_and_below() {
    unsafe { std::env::set_var("CODEL_MCP_RECURSIVE_CONFIG_WATCH", "true") };
    let r =
        resolve_mcp_recursive_config_watch(None, Some(false), Some(true), Some(true), Some(true));
    unsafe { std::env::remove_var("CODEL_MCP_RECURSIVE_CONFIG_WATCH") };
    assert!(!r.value);
    assert_eq!(r.source, ConfigSource::Cli);
}
#[test]
#[serial]
fn mcp_recursive_config_watch_env_wins_over_config_and_below() {
    unsafe { std::env::set_var("CODEL_MCP_RECURSIVE_CONFIG_WATCH", "false") };
    let r = resolve_mcp_recursive_config_watch(None, None, Some(true), Some(true), Some(true));
    unsafe { std::env::remove_var("CODEL_MCP_RECURSIVE_CONFIG_WATCH") };
    assert!(!r.value);
    assert_eq!(r.source, ConfigSource::Env);
}
#[test]
#[serial]
fn mcp_recursive_config_watch_config_wins_over_managed_and_feature_flag() {
    unsafe { std::env::remove_var("CODEL_MCP_RECURSIVE_CONFIG_WATCH") };
    let r = resolve_mcp_recursive_config_watch(None, None, Some(false), Some(true), Some(true));
    assert!(!r.value);
    assert_eq!(r.source, ConfigSource::Config);
}
#[test]
#[serial]
fn mcp_recursive_config_watch_managed_wins_over_feature_flag() {
    unsafe { std::env::remove_var("CODEL_MCP_RECURSIVE_CONFIG_WATCH") };
    let r = resolve_mcp_recursive_config_watch(None, None, None, Some(false), Some(true));
    assert!(!r.value);
    assert_eq!(r.source, ConfigSource::ManagedConfig);
}
#[test]
#[serial]
fn mcp_recursive_config_watch_feature_flag_used_when_no_higher_layer() {
    unsafe { std::env::remove_var("CODEL_MCP_RECURSIVE_CONFIG_WATCH") };
    let r = resolve_mcp_recursive_config_watch(None, None, None, None, Some(false));
    assert!(!r.value);
    assert_eq!(r.source, ConfigSource::Remote);
}
#[test]
#[serial_test::serial(remote_sig_disarm)]
fn remote_settings_disarm_managed_config_signatures() {
    let prod = crate::env::PROD_CLI_CHAT_PROXY_BASE_URL;
    let _env = crate::env::EnvVarGuard::remove("CODEL_CLI_CHAT_PROXY_BASE_URL");
    codel_config::signed_policy::apply_remote_managed_config_signature_verification(
        Some(true),
        true,
    );
    assert!(codel_config::signed_policy::verification_active());
    let settings = crate::util::config::RemoteSettings {
        managed_config_signature_verification: Some(false),
        ..Default::default()
    };
    apply_remote_settings_side_effects(Some(&settings), prod);
    assert!(!codel_config::signed_policy::verification_active());
    let settings = crate::util::config::RemoteSettings {
        managed_config_signature_verification: Some(true),
        ..Default::default()
    };
    apply_remote_settings_side_effects(Some(&settings), prod);
    assert!(codel_config::signed_policy::verification_active());
    codel_config::signed_policy::apply_remote_managed_config_signature_verification(
        Some(false),
        true,
    );
    apply_remote_settings_side_effects(None, prod);
    assert!(!codel_config::signed_policy::verification_active());
    codel_config::signed_policy::apply_remote_managed_config_signature_verification(
        Some(true),
        true,
    );
    assert!(codel_config::signed_policy::verification_active());
}
/// A pass with no payload must keep the last applied policy (a cancelled
/// first bootstrap followed by an offline fallback pass must not wipe it);
/// a real payload still rewrites the caches, clearing omitted fields.
#[test]
#[serial_test::serial(remote_sig_disarm)]
fn absent_settings_keep_previously_applied_remote_policy() {
    let prod = crate::env::PROD_CLI_CHAT_PROXY_BASE_URL;
    let settings = crate::util::config::RemoteSettings {
        prompt_suggestions: Some(serde_json::json!({"enabled": true})),
        ..Default::default()
    };
    apply_remote_settings_side_effects(Some(&settings), prod);
    assert_eq!(
        crate::util::config::cached_remote_prompt_suggestions_enabled(),
        Some(true)
    );
    apply_remote_settings_side_effects(None, prod);
    assert_eq!(
        crate::util::config::cached_remote_prompt_suggestions_enabled(),
        Some(true),
        "a fetchless pass must not wipe the last applied payload"
    );
    apply_remote_settings_side_effects(Some(&crate::util::config::RemoteSettings::default()), prod);
    assert_eq!(
        crate::util::config::cached_remote_prompt_suggestions_enabled(),
        None,
        "a real payload clears the fields it omits"
    );
}
/// Keyed path: prod proxy origin can disarm; env override cannot.
#[test]
#[serial_test::serial(remote_sig_disarm)]
fn remote_settings_disarm_requires_prod_proxy_when_keys_embedded() {
    let prod = crate::env::PROD_CLI_CHAT_PROXY_BASE_URL;
    codel_config::signed_policy::apply_remote_managed_config_signature_verification(
        Some(true),
        true,
    );
    assert!(codel_config::signed_policy::verification_active());
    let settings = crate::util::config::RemoteSettings {
        managed_config_signature_verification: Some(false),
        ..Default::default()
    };
    let env = crate::env::EnvVarGuard::remove("CODEL_CLI_CHAT_PROXY_BASE_URL");
    apply_remote_settings_side_effects(Some(&settings), prod);
    assert!(
        !codel_config::signed_policy::verification_active(),
        "prod proxy origin must allow disarm when keys are embedded"
    );
    codel_config::signed_policy::apply_remote_managed_config_signature_verification(
        Some(true),
        true,
    );
    assert!(codel_config::signed_policy::verification_active());
    env.set_value("https://attacker.example/v1");
    apply_remote_settings_side_effects(Some(&settings), prod);
    assert!(
        codel_config::signed_policy::verification_active(),
        "env-overridden proxy must not be able to disarm keyed verification"
    );
    codel_config::signed_policy::apply_remote_managed_config_signature_verification(
        Some(true),
        true,
    );
}
#[test]
fn a_status_line_the_parser_could_not_read_in_full_reaches_codel_inspect() {
    use super::super::config_model_override_parse::{ConfigWarningKind, WarningTarget};
    let raw_config: toml::Value = toml::from_str(
        r#"
            [ui]
            theme = "kanagawa"

            [ui.status_line]
            type = "disabled"
            padding = "2"
            colour = "red"
            "#,
    )
    .unwrap();
    let cfg = Config::new_from_toml_cfg(&raw_config).expect("a typo must not fail the config");
    let warnings = |path: &str, kind: ConfigWarningKind| {
        cfg.config_warnings
            .iter()
            .filter(|w| {
                w.kind == kind
                    && matches!(&w.target, WarningTarget::ConfigKey { path: p } if p == path)
            })
            .count()
    };
    assert_eq!(
        warnings("ui.status_line", ConfigWarningKind::InvalidValue),
        1
    );
    assert_eq!(
        warnings("ui.status_line.colour", ConfigWarningKind::UnknownField),
        1
    );
    assert_eq!(cfg.ui.theme.as_deref(), Some("kanagawa"));
}
/// A model's `env_key` credential, installed as the process static key, is served by the shared api-key provider.
/// Lives agent-side because it resolves a model list from the full `Config`, which the auth layer never sees.
#[tokio::test]
#[serial]
async fn process_key_from_model_env_key() {
    use std::sync::Arc;
    use codel_login::{AuthManager, CodelComConfig, shared_api_key_provider};
    const ENV: &str = "TEST_MODEL_ENV_KEY";
    const TOKEN: &str = "model-env-token";
    let _codel = EnvGuard::unset("CODEL_API_KEY");
    let _legacy = EnvGuard::unset("CODEL_CODE_CODEL_API_KEY");
    let _tok = EnvGuard::set(ENV, TOKEN);
    let dm = codel_models::default_model();
    let cfg = Config::new_from_toml_cfg(
        &toml::from_str(&format!(
            r#"
            [model."{dm}"]
            model = "{dm}"
            env_key = "{ENV}"
            "#
        ))
        .unwrap(),
    )
    .unwrap();
    let key = resolve_model_list(&cfg, None)
        .get(dm)
        .and_then(|m| m.own_credential())
        .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let mgr = Arc::new(AuthManager::new(dir.path(), CodelComConfig::default()));
    assert!(mgr.current().is_none());
    mgr.set_process_static_api_key(Some(key));
    assert_eq!(
        shared_api_key_provider(mgr)
            .current_api_key_async()
            .await
            .as_deref(),
        Some(TOKEN)
    );
}
