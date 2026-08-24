//! Data APIs for `codel models`. Clients own display.

use agent_client_protocol as acp;
use anyhow::Result;
use codel_acp_lib::{AcpAgentTx, acp_send};

use crate::agent::config::Config as AgentConfig;

/// Status for the `codel models` banner (display order ≠ sampling priority; see [`AuthStatus::resolve`]).
#[derive(Debug, PartialEq, Eq)]
pub enum AuthStatus {
    ApiKey,
    /// Auth host from `codel_ws_origin` (scheme stripped).
    LoggedIn(String),
    /// Catalog key of the first model with own `api_key`/`env_key`.
    ModelCredentials(String),
    DeploymentKey,
    NotAuthenticated,
}

impl AuthStatus {
    /// Banner status: session → BYOK → deployment → none.
    ///
    /// Differs from sampling (`resolve_credentials`: BYOK → session) so a
    /// logged-in user sees the login host. BYOK uses
    /// [`crate::agent::auth_method::should_advertise_codel_api_key`] so
    /// per-model credentials are detected.
    pub fn resolve(agent_config: &AgentConfig) -> Self {
        if agent_config.create_auth_manager().current().is_some() {
            let origin = &agent_config.codel_com_config.codel_ws_origin;
            let host = origin
                .strip_prefix("https://")
                .or_else(|| origin.strip_prefix("http://"))
                .unwrap_or(origin);
            return Self::LoggedIn(host.to_owned());
        }
        let models = crate::agent::config::resolve_model_list(agent_config, None);
        if crate::agent::auth_method::should_advertise_codel_api_key(
            agent_config.codel_com_config.api_key_auth_disabled(),
            models.values(),
        ) && let Some(name) = models
            .iter()
            .find_map(|(name, entry)| entry.has_own_credentials().then(|| name.clone()))
        {
            return Self::ModelCredentials(name);
        }
        if agent_config.endpoints.deployment_key.is_some() {
            return Self::DeploymentKey;
        }
        Self::NotAuthenticated
    }
}

/// Fetch model state (available models + default) over an ACP channel.
pub async fn list_models(
    acp_tx: &AcpAgentTx,
    client_type: &str,
    client_version: &str,
) -> Result<acp::SessionModelState> {
    let init_resp: acp::InitializeResponse = acp_send(
        acp::InitializeRequest::new(acp::ProtocolVersion::V1)
            .client_capabilities(
                acp::ClientCapabilities::new()
                    .fs(acp::FileSystemCapabilities::new())
                    .terminal(false),
            )
            .meta(
                serde_json::json!({
                    "clientType": client_type,
                    "clientVersion": client_version,
                })
                .as_object()
                .cloned(),
            ),
        acp_tx,
    )
    .await?;

    let model_state = init_resp
        .meta
        .and_then(|m| m.get("modelState").cloned())
        .ok_or_else(|| anyhow::anyhow!("InitializeResponse missing modelState"))?;
    let state: acp::SessionModelState = serde_json::from_value(model_state)
        .map_err(|e| anyhow::anyhow!("Failed to parse modelState: {}", e))?;

    Ok(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::config::Config;
    use crate::auth::{AuthMode, CodelAuth};
    use serial_test::serial;
    use codel_test_support::EnvGuard;

    /// Isolate process-global auth sources that `AuthStatus::resolve` consults.
    ///
    /// Uses `CODEL_AUTH_PATH` (not `CODEL_HOME`) so a OnceLock-cached real home
    /// with `auth.json` cannot leak into these tests.
    fn isolate_auth_sources() -> (tempfile::TempDir, [EnvGuard; 5]) {
        let dir = tempfile::tempdir().unwrap();
        let auth_path = dir.path().join("no-auth.json");
        let guards = [
            EnvGuard::unset("CODEL_AUTH"),
            EnvGuard::set("CODEL_AUTH_PATH", auth_path.to_str().unwrap()),
            EnvGuard::unset("CODEL_DEPLOYMENT_KEY"),
            EnvGuard::unset("CODEL_WS_ORIGIN"),
            EnvGuard::unset("CODEL_DISABLE_API_KEY_AUTH"),
        ];
        (dir, guards)
    }

    fn byok_and_deployment_toml(model_id: &str) -> String {
        format!(
            r#"
            [endpoints]
            deployment_key = "deploy-key"

            [model."{model_id}"]
            model = "{model_id}"
            api_key = "sk-byok"
            "#
        )
    }

    fn config_from_toml(toml_src: &str) -> Config {
        let toml: toml::Value = toml::from_str(toml_src).unwrap();
        Config::new_from_toml_cfg(&toml).expect("config should parse")
    }

    #[test]
    #[serial]
    fn resolve_oauth_session() {
        let (_dir, _g) = isolate_auth_sources();
        let token = CodelAuth {
            key: "session-token".into(),
            auth_mode: AuthMode::ApiKey,
            ..CodelAuth::test_default()
        };
        let json = serde_json::to_string(&token).unwrap();
        let _auth = EnvGuard::set("CODEL_AUTH", &json);

        assert_eq!(
            AuthStatus::resolve(&Config::default()),
            AuthStatus::LoggedIn("codel.dev".to_owned())
        );
    }

    #[test]
    #[serial]
    fn resolve_model_api_key_byok() {
        let (_dir, _g) = isolate_auth_sources();
        let dm = "test-model";
        let cfg = config_from_toml(&format!(
            r#"
            [model."{dm}"]
            model = "{dm}"
            api_key = "sk-byok-inline"
            "#
        ));
        assert_eq!(
            AuthStatus::resolve(&cfg),
            AuthStatus::ModelCredentials(dm.to_owned())
        );
    }

    #[test]
    #[serial]
    fn resolve_model_env_key_byok() {
        let (_dir, _g) = isolate_auth_sources();
        const TEST_ENV: &str = "TEST_AUTH_STATUS_BYOK_ENV_KEY";
        let dm = "test-model";
        let cfg = config_from_toml(&format!(
            r#"
            [model."{dm}"]
            model = "{dm}"
            env_key = "{TEST_ENV}"
            "#
        ));

        {
            let _unset = EnvGuard::unset(TEST_ENV);
            assert_eq!(AuthStatus::resolve(&cfg), AuthStatus::NotAuthenticated);
        }
        {
            let _set = EnvGuard::set(TEST_ENV, "secret-token");
            assert_eq!(
                AuthStatus::resolve(&cfg),
                AuthStatus::ModelCredentials(dm.to_owned())
            );
        }
    }

    #[test]
    #[serial]
    fn resolve_deployment_key() {
        let (_dir, _g) = isolate_auth_sources();
        let mut cfg = Config::default();
        cfg.endpoints.deployment_key = Some("deploy-key".into());
        assert_eq!(AuthStatus::resolve(&cfg), AuthStatus::DeploymentKey);
    }

    #[test]
    #[serial]
    fn resolve_not_authenticated() {
        let (_dir, _g) = isolate_auth_sources();
        assert_eq!(
            AuthStatus::resolve(&Config::default()),
            AuthStatus::NotAuthenticated
        );
    }

    #[test]
    #[serial]
    fn resolve_priority_session_over_byok_and_deployment() {
        let (_dir, _g) = isolate_auth_sources();
        let token = CodelAuth {
            key: "session-token".into(),
            auth_mode: AuthMode::ApiKey,
            ..CodelAuth::test_default()
        };
        let json = serde_json::to_string(&token).unwrap();
        let _auth = EnvGuard::set("CODEL_AUTH", &json);

        let dm = "test-model";
        let cfg = config_from_toml(&byok_and_deployment_toml(dm));
        assert_eq!(
            AuthStatus::resolve(&cfg),
            AuthStatus::LoggedIn("codel.dev".to_owned())
        );
    }

    #[test]
    #[serial]
    fn resolve_priority_byok_over_deployment() {
        let (_dir, _g) = isolate_auth_sources();
        let dm = "test-model";
        let cfg = config_from_toml(&byok_and_deployment_toml(dm));
        assert_eq!(
            AuthStatus::resolve(&cfg),
            AuthStatus::ModelCredentials(dm.to_owned())
        );
    }

    #[test]
    #[serial]
    fn resolve_disable_api_key_auth_suppresses_byok_banner() {
        let (_dir, _g) = isolate_auth_sources();
        let dm = "test-model";
        let cfg = config_from_toml(&format!(
            r#"
            [codel_com_config]
            disable_api_key_auth = true

            [model."{dm}"]
            model = "{dm}"
            api_key = "sk-byok"
            "#
        ));
        assert_eq!(AuthStatus::resolve(&cfg), AuthStatus::NotAuthenticated);
    }

    #[test]
    #[serial]
    fn resolve_disable_api_key_auth_falls_through_to_deployment() {
        let (_dir, _g) = isolate_auth_sources();
        let dm = "test-model";
        let cfg = config_from_toml(&format!(
            r#"
            [codel_com_config]
            disable_api_key_auth = true

            [endpoints]
            deployment_key = "deploy-key"

            [model."{dm}"]
            model = "{dm}"
            api_key = "sk-byok"
            "#
        ));
        assert_eq!(AuthStatus::resolve(&cfg), AuthStatus::DeploymentKey);
    }

    #[test]
    #[serial]
    fn resolve_model_credentials_uses_first_catalog_key() {
        let (_dir, _g) = isolate_auth_sources();
        let cfg = config_from_toml(
            r#"
            [model."my-openai"]
            model = "gpt-4o"
            api_key = "sk-first"

            [model."my-anthropic"]
            model = "claude"
            api_key = "sk-second"
            "#,
        );
        assert_eq!(
            AuthStatus::resolve(&cfg),
            AuthStatus::ModelCredentials("my-openai".to_owned())
        );
    }
}
