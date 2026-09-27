use std::num::NonZeroU64;

use indexmap::IndexMap;

use super::config::{ConfigModelOverride, EnvKeys};
use super::config_model_override_parse::{ConfigWarning, ConfigWarningKind};
use crate::sampling::ApiBackend;

#[derive(Clone, Debug, Default, serde::Deserialize)]
#[serde(default)]
pub struct ModelProviderConfig {
    pub base_url: Option<String>,
    pub api_base_url: Option<String>,
    pub env_key: Option<EnvKeys>,
    pub api_key: Option<String>,
    pub api_backend: Option<ApiBackend>,
    pub extra_headers: IndexMap<String, String>,
    /// Query parameters folded into every request URL; inherited by models.
    pub query_params: IndexMap<String, String>,
    /// Header name to environment variable; inherited by models, resolved at client build.
    pub env_http_headers: IndexMap<String, String>,
    pub context_window: Option<u64>,
    /// Request-body cap of this endpoint; inherited by models that set none of their own.
    pub max_request_bytes: Option<NonZeroU64>,
}

pub(crate) fn parse_model_providers(
    raw_config: &toml::Value,
) -> (IndexMap<String, ModelProviderConfig>, Vec<ConfigWarning>) {
    let mut providers = IndexMap::new();
    let mut warnings = Vec::new();
    let Some(section) = raw_config.get("model_providers") else {
        return (providers, warnings);
    };
    let Some(table) = section.as_table() else {
        warnings.push(ConfigWarning::model_provider_section(
            ConfigWarningKind::NotATable,
            format!(
                "`model_providers` must be a table of [model_providers.<id>] entries, got {}; \
                 all model providers ignored",
                section.type_str()
            ),
        ));
        return (providers, warnings);
    };
    for (id, value) in table {
        let mut unknown = Vec::new();
        match serde_ignored::deserialize::<_, _, ModelProviderConfig>(value.clone(), |path| {
            unknown.push(path.to_string());
        }) {
            Ok(provider) => {
                for key in unknown {
                    warnings.push(ConfigWarning::model_provider(
                        id,
                        Some(key.as_str()),
                        ConfigWarningKind::UnknownField,
                        "unrecognized key; field ignored".to_owned(),
                    ));
                }
                providers.insert(id.clone(), provider);
            }
            Err(error) => {
                warnings.push(ConfigWarning::model_provider(
                    id,
                    None,
                    ConfigWarningKind::InvalidValue,
                    format!(
                        "failed to parse ({error}); provider skipped, inheriting models \
                         resolve with defaults"
                    ),
                ));
            }
        }
    }
    (providers, warnings)
}

impl ConfigModelOverride {
    pub(crate) fn with_provider_defaults(
        &self,
        provider: &ModelProviderConfig,
        provider_id: &str,
    ) -> Self {
        let ModelProviderConfig {
            base_url,
            api_base_url,
            env_key,
            api_key,
            api_backend,
            extra_headers,
            query_params,
            env_http_headers,
            context_window,
            max_request_bytes,
        } = provider;

        let mut merged = self.clone();
        merged.model_provider = None;
        merged.base_url = merged.base_url.or_else(|| base_url.clone());
        merged.api_base_url = merged.api_base_url.or_else(|| api_base_url.clone());
        merged.api_backend = merged.api_backend.or_else(|| api_backend.clone());
        merged.context_window = merged.context_window.or(*context_window);
        merged.max_request_bytes = merged.max_request_bytes.or(*max_request_bytes);
        // Inherited wholesale only when the model sets none of its own.
        if merged.extra_headers.is_empty() {
            merged.extra_headers = extra_headers.clone();
        }
        if merged.query_params.is_empty() {
            merged.query_params = query_params.clone();
        }
        if merged.env_http_headers.is_empty() {
            merged.env_http_headers = env_http_headers.clone();
        }
        let model_sets_own_api_key = self
            .api_key
            .as_deref()
            .is_some_and(|k| !k.trim().is_empty());
        let model_sets_own_env_key = self.env_key.as_ref().and_then(EnvKeys::primary).is_some();
        let model_has_own_auth = model_sets_own_api_key || model_sets_own_env_key;
        if !model_has_own_auth {
            merged.api_key = api_key.clone();
            merged.env_key = env_key.clone();
        }
        merged
    }

    pub(crate) fn with_missing_provider(&self) -> Self {
        let mut merged = self.clone();
        merged.model_provider = None;
        merged
    }
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU64;

    use crate::agent::config::{
        Config, resolve_credentials, resolve_model_list, sampling_config_for_model,
    };
    #[test]
    fn model_inherits_provider_connection_defaults() {
        let raw_config: toml::Value = toml::from_str(
            r#"
            [model_providers.gateway]
            base_url = "https://gateway.example/v1"
            context_window = 123456

            [model_providers.gateway.extra_headers]
            X-Corp = "yes"

            [model.via-gateway]
            model = "m"
            model_provider = "gateway"
            "#,
        )
        .unwrap();

        let cfg = Config::new_from_toml_cfg(&raw_config).expect("config should parse");
        assert!(cfg.model_providers.contains_key("gateway"));
        let resolved = resolve_model_list(&cfg, None);
        let model = resolved.get("via-gateway").expect("model should exist");
        assert_eq!(model.info.base_url, "https://gateway.example/v1");
        assert_eq!(model.info.context_window.get(), 123456);
        assert_eq!(
            model.info.extra_headers.get("X-Corp").map(String::as_str),
            Some("yes")
        );
        assert!(
            model.has_own_credentials(),
            "a custom endpoint without a credential is BYOK, not session-authed"
        );
        assert_eq!(
            resolve_credentials(model, Some("session-jwt")).api_key,
            None,
            "the session token must not leak to the provider's custom endpoint"
        );
    }

    #[test]
    fn model_fields_override_provider_defaults() {
        let raw_config: toml::Value = toml::from_str(
            r#"
            [model_providers.gateway]
            base_url = "https://gateway.example/v1"
            context_window = 100000

            [model.override-url]
            model = "m"
            model_provider = "gateway"
            base_url = "https://model-specific.example/v1"
            context_window = 200000
            "#,
        )
        .unwrap();

        let cfg = Config::new_from_toml_cfg(&raw_config).expect("config should parse");
        let resolved = resolve_model_list(&cfg, None);
        let model = resolved.get("override-url").expect("model should exist");
        assert_eq!(model.info.base_url, "https://model-specific.example/v1");
        assert_eq!(model.info.context_window.get(), 200000);
    }

    #[test]
    fn model_inherits_provider_max_request_bytes() {
        let raw_config: toml::Value = toml::from_str(
            r#"
            [model_providers.messages-gateway]
            base_url = "https://gateway.example/v1"
            api_backend = "messages"
            max_request_bytes = 20000000

            [model.inherits]
            model = "claude-sonnet"
            model_provider = "messages-gateway"

            [model.overrides]
            model = "claude-opus"
            model_provider = "messages-gateway"
            max_request_bytes = 10000000
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
            max_request_bytes("inherits"),
            "the provider cap reaches a model that sets none and beats the messages default"
        );
        assert_eq!(
            NonZeroU64::new(10_000_000),
            max_request_bytes("overrides"),
            "the model's own cap overrides the provider's"
        );
    }



    #[test]
    fn undefined_model_provider_fails_closed() {
        use super::super::config_model_override_parse::{ConfigWarningKind, WarningTarget};

        let raw_config: toml::Value = toml::from_str(
            r#"
            [model.dangling]
            model = "m"
            base_url = "https://third-party.example/v1"
            context_window = 200000
            model_provider = "ghost"
            "#,
        )
        .unwrap();

        let cfg = Config::new_from_toml_cfg(&raw_config).expect("config should parse");
        assert!(
            cfg.config_warnings.iter().any(|w| {
                w.kind == ConfigWarningKind::InvalidValue
                    && matches!(
                        &w.target,
                        WarningTarget::Model { field, .. }
                            if field.as_deref() == Some("model_provider")
                    )
            }),
            "an undefined provider reference warns: {:?}",
            cfg.config_warnings
        );
        let resolved = resolve_model_list(&cfg, None);
        let model = resolved.get("dangling").expect("model should exist");
        assert_eq!(
            model.info.base_url, "https://third-party.example/v1",
            "the model keeps its own connection fields"
        );
        assert!(
            model.has_own_credentials(),
            "an undefined provider leaves the model BYOK, not session-authed"
        );
        let creds = resolve_credentials(model, Some("session-jwt"));
        assert_eq!(
            creds.api_key, None,
            "no credential resolves and the session token does not leak to the model's base_url"
        );
    }

    #[test]
    fn undefined_model_provider_keeps_model_own_key() {
        let raw_config: toml::Value = toml::from_str(
            r#"
            [model.own-key]
            model = "m"
            base_url = "https://third-party.example/v1"
            context_window = 200000
            api_key = "sk-model-own"
            model_provider = "ghost"
            "#,
        )
        .unwrap();

        let cfg = Config::new_from_toml_cfg(&raw_config).expect("config should parse");
        let resolved = resolve_model_list(&cfg, None);
        let model = resolved.get("own-key").expect("model should exist");
        let creds = resolve_credentials(model, Some("session-jwt"));
        assert_eq!(creds.api_key.as_deref(), Some("sk-model-own"));
    }

    #[test]
    fn model_provider_parse_warnings_are_lenient_and_specific() {
        use super::super::config_model_override_parse::{ConfigWarningKind, WarningTarget};

        let raw_config: toml::Value = toml::from_str(
            r#"
            [model_providers.good]
            base_url = "https://good.example/v1"

            [model_providers.bad-type]
            context_window = "not-a-number"

            [model_providers.typo]
            base_url = "https://typo.example/v1"
            unknown_field = 5

            [model.on-broken-provider]
            model = "m"
            base_url = "https://x.example/v1"
            context_window = 200000
            model_provider = "bad-type"
            "#,
        )
        .unwrap();

        let cfg = Config::new_from_toml_cfg(&raw_config)
            .expect("one bad provider must not fail the config");
        assert!(cfg.model_providers.contains_key("good"));
        assert!(
            !cfg.model_providers.contains_key("bad-type"),
            "a malformed provider is skipped"
        );

        let has_provider = |id: &str, field: Option<&str>, kind: ConfigWarningKind| {
            cfg.config_warnings.iter().any(|w| {
                w.kind == kind
                    && matches!(
                        &w.target,
                        WarningTarget::ModelProvider { id: i, field: f }
                            if i == id && f.as_deref() == field
                    )
            })
        };
        assert!(has_provider(
            "bad-type",
            None,
            ConfigWarningKind::InvalidValue
        ));
        assert!(has_provider(
            "typo",
            Some("unknown_field"),
            ConfigWarningKind::UnknownField
        ));
        assert!(
            !cfg.config_warnings.iter().any(|w| {
                matches!(
                    &w.target,
                    WarningTarget::Model { field, .. }
                        if field.as_deref() == Some("model_provider")
                )
            }),
            "a declared-but-malformed provider must not also warn as undefined: {:?}",
            cfg.config_warnings
        );

        let raw_config: toml::Value = toml::from_str(r#"model_providers = "oops""#).unwrap();
        let cfg = Config::new_from_toml_cfg(&raw_config)
            .expect("a non-table model_providers must not fail the config");
        assert!(cfg.model_providers.is_empty());
        assert!(
            cfg.config_warnings.iter().any(|w| {
                matches!(w.target, WarningTarget::ModelProviderSection)
                    && w.kind == ConfigWarningKind::NotATable
            }),
            "non-table section warns: {:?}",
            cfg.config_warnings
        );
    }

    #[test]
    fn model_provider_conflicting_credentials_warn() {
        use super::super::config_model_override_parse::{ConfigWarningKind, WarningTarget};

        let raw_config: toml::Value = toml::from_str(
            r#"
            [model_providers.static-shadows]
            base_url = "https://a.example/v1"
            api_key = "sk-static"
            [model_providers.static-shadows.auth]
            command = "printf tok"

            [model_providers.env-shadows]
            base_url = "https://b.example/v1"
            env_key = "SOME_VAR"
            [model_providers.env-shadows.auth]
            command = "printf tok"

            [model_providers.two-helpers]
            base_url = "https://c.example/v1"
            auth_provider = "corp"
            [model_providers.two-helpers.auth]
            command = "printf tok"
            "#,
        )
        .unwrap();

        let cfg = Config::new_from_toml_cfg(&raw_config).expect("config should parse");
        let has = |id: &str, field: &str| {
            cfg.config_warnings.iter().any(|w| {
                w.kind == ConfigWarningKind::ConflictingFields
                    && matches!(
                        &w.target,
                        WarningTarget::ModelProvider { id: i, field: f }
                            if i == id && f.as_deref() == Some(field)
                    )
            })
        };
        assert!(
            has("static-shadows", "api_key"),
            "a static api_key shadows the helper: {:?}",
            cfg.config_warnings
        );
        assert!(
            has("env-shadows", "env_key"),
            "an env_key may shadow the helper: {:?}",
            cfg.config_warnings
        );
        assert!(
            has("two-helpers", "auth"),
            "auth_provider shadows the inline auth helper: {:?}",
            cfg.config_warnings
        );
    }

    #[test]
    fn model_provider_undefined_auth_provider_warns() {
        use super::super::config_model_override_parse::{ConfigWarningKind, WarningTarget};

        let raw_config: toml::Value = toml::from_str(
            r#"
            [model_providers.gateway]
            base_url = "https://gateway.example/v1"
            auth_provider = "nonexistent"
            "#,
        )
        .unwrap();

        let cfg = Config::new_from_toml_cfg(&raw_config).expect("config should parse");
        assert!(
            cfg.config_warnings.iter().any(|w| {
                w.kind == ConfigWarningKind::InvalidValue
                    && matches!(
                        &w.target,
                        WarningTarget::ModelProvider { id, field }
                            if id == "gateway" && field.as_deref() == Some("auth_provider")
                    )
            }),
            "an undefined provider auth_provider reference warns: {:?}",
            cfg.config_warnings
        );
    }



    #[test]
    fn model_inherits_provider_static_key() {
        let raw_config: toml::Value = toml::from_str(
            r#"
            [model_providers.gateway]
            base_url = "https://gateway.example/v1"
            api_key = "sk-provider"

            [model.via-gateway]
            model = "m"
            model_provider = "gateway"
            "#,
        )
        .unwrap();

        let cfg = Config::new_from_toml_cfg(&raw_config).expect("config should parse");
        let resolved = resolve_model_list(&cfg, None);
        let model = resolved.get("via-gateway").expect("model should exist");
        assert_eq!(
            resolve_credentials(model, Some("session-jwt"))
                .api_key
                .as_deref(),
            Some("sk-provider"),
            "the provider's static key resolves for the inheriting model"
        );
    }

    #[test]
    fn declared_unresolved_credential_fails_closed_on_provider_endpoint() {
        let raw_config: toml::Value = toml::from_str(
            r#"
            [model_providers.gateway]
            base_url = "https://gateway.example/v1"

            [model.via-gateway]
            model = "m"
            model_provider = "gateway"
            env_key = "DEFINITELY_UNSET_MODEL_PROVIDER_TEST_VAR"
            "#,
        )
        .unwrap();

        let cfg = Config::new_from_toml_cfg(&raw_config).expect("config should parse");
        let resolved = resolve_model_list(&cfg, None);
        let model = resolved.get("via-gateway").expect("model should exist");
        assert_eq!(
            resolve_credentials(model, Some("session-jwt")).api_key,
            None,
            "an unresolved declared credential must not fall back to the session token"
        );
    }

    #[test]
    fn model_inherits_provider_api_backend_and_base_url() {
        let raw_config: toml::Value = toml::from_str(
            r#"
            [model_providers.gateway]
            base_url = "https://gateway.example/v1"
            api_base_url = "https://gateway.example/api"
            api_backend = "responses"
            api_key = "sk-provider"

            [model.via-gateway]
            model = "m"
            model_provider = "gateway"
            "#,
        )
        .unwrap();

        let cfg = Config::new_from_toml_cfg(&raw_config).expect("config should parse");
        let resolved = resolve_model_list(&cfg, None);
        let model = resolved.get("via-gateway").expect("model should exist");
        assert_eq!(
            model.info.api_backend,
            crate::sampling::ApiBackend::Responses
        );
        assert_eq!(
            model.api_base_url.as_deref(),
            Some("https://gateway.example/api")
        );
    }



    #[test]
    fn model_headers_shadow_provider_headers() {
        let raw_config: toml::Value = toml::from_str(
            r#"
            [model_providers.gateway]
            base_url = "https://gateway.example/v1"
            api_key = "sk-provider"

            [model_providers.gateway.extra_headers]
            X-Corp = "yes"

            [model.via-gateway]
            model = "m"
            context_window = 200000
            model_provider = "gateway"

            [model.via-gateway.extra_headers]
            X-Model = "own"
            "#,
        )
        .unwrap();

        let cfg = Config::new_from_toml_cfg(&raw_config).expect("config should parse");
        let resolved = resolve_model_list(&cfg, None);
        let model = resolved.get("via-gateway").expect("model should exist");
        assert_eq!(
            model.info.extra_headers.get("X-Model").map(String::as_str),
            Some("own")
        );
        assert!(
            model.info.extra_headers.get("X-Corp").is_none(),
            "a model that sets any header inherits none of the provider's"
        );
    }

    #[test]
    fn model_provider_inline_auth_ttl_and_timeout_warn() {
        use super::super::config_model_override_parse::{ConfigWarningKind, WarningTarget};

        let raw_config: toml::Value = toml::from_str(
            r#"
            [model_providers.gateway]
            base_url = "https://gateway.example/v1"

            [model_providers.gateway.auth]
            command = "printf tok"
            token_ttl_secs = 5
            timeout_secs = 0
            "#,
        )
        .unwrap();

        let cfg = Config::new_from_toml_cfg(&raw_config).expect("config should parse");
        let has = |field: &str| {
            cfg.config_warnings.iter().any(|w| {
                w.kind == ConfigWarningKind::InvalidValue
                    && matches!(
                        &w.target,
                        WarningTarget::ModelProvider { id, field: f }
                            if id == "gateway" && f.as_deref() == Some(field)
                    )
            })
        };
        assert!(
            has("auth.token_ttl_secs"),
            "inline auth ttl below the refresh margin warns: {:?}",
            cfg.config_warnings
        );
        assert!(
            has("auth.timeout_secs"),
            "inline auth timeout out of range warns: {:?}",
            cfg.config_warnings
        );
    }


    #[test]
    fn model_inherits_provider_query_params_and_env_http_headers() {
        let toml_cfg: toml::Value = toml::from_str(
            r#"
            [model_providers.gateway]
            base_url = "https://gateway.example/v1"
            api_key = "sk-provider"

            [model_providers.gateway.query_params]
            api-version = "2026-07-22"

            [model_providers.gateway.env_http_headers]
            X-Tenant-Token = "GATEWAY_TENANT_TOKEN"

            [model.via-gateway]
            model = "m"
            model_provider = "gateway"
            "#,
        )
        .unwrap();

        let cfg = Config::new_from_toml_cfg(&toml_cfg).expect("config should parse");
        let resolved = resolve_model_list(&cfg, None);
        let model = resolved.get("via-gateway").expect("model should exist");
        assert_eq!(
            model
                .info
                .query_params
                .get("api-version")
                .map(String::as_str),
            Some("2026-07-22"),
            "the model inherits the provider's query params"
        );
        assert_eq!(
            model
                .info
                .env_http_headers
                .get("X-Tenant-Token")
                .map(String::as_str),
            Some("GATEWAY_TENANT_TOKEN"),
            "the model inherits the provider's env_http_headers mapping (unresolved names)"
        );
    }

    #[test]
    fn model_query_params_shadow_provider_query_params() {
        let toml_cfg: toml::Value = toml::from_str(
            r#"
            [model_providers.gateway]
            base_url = "https://gateway.example/v1"
            api_key = "sk-provider"

            [model_providers.gateway.query_params]
            api-version = "provider"

            [model.via-gateway]
            model = "m"
            model_provider = "gateway"

            [model.via-gateway.query_params]
            api-version = "model"
            "#,
        )
        .unwrap();

        let cfg = Config::new_from_toml_cfg(&toml_cfg).expect("config should parse");
        let resolved = resolve_model_list(&cfg, None);
        let model = resolved.get("via-gateway").expect("model should exist");
        assert_eq!(
            model
                .info
                .query_params
                .get("api-version")
                .map(String::as_str),
            Some("model"),
            "a model that sets its own query params inherits none of the provider's"
        );
    }
}
