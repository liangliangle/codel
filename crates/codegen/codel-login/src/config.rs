//! Configuration for the Codel authority this build talks to.
//!
//! Upstream's `CodelComConfig` also carried the OIDC/OAuth2 provider settings,
//! the external auth-provider command, the forced-login-team pin and the
//! `preferred_method` selector. The fork authenticates with an API key only, so
//! all of that is gone; what remains is the authority's origin/URL, the header
//! name the relay uses, and the API-key kill switch.

use serde::{Deserialize, Serialize};
pub use codel_config::CLI_CHAT_PROXY_BASE_URL_DEFAULT;
use codel_config::{Capability, Distribution};
use codel_shell_base::env::{PROD_RELAY_WS_URL, PROD_WS_ORIGIN};

/// `auth.json` scope key.
///
/// Upstream derived this from the OIDC issuer and client id so a personal and a
/// team session could coexist. An API key is a single credential, and the value
/// is kept verbatim so an `auth.json` written by an earlier build still
/// resolves. It is a storage key only: nothing reads the issuer.
const AUTH_SCOPE_KEY: &str = "https://auth.codel.dev::b1a00492-073a-47ea-816f-4c329264a828";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CodelComConfig {
    pub codel_ws_origin: String,
    pub codel_ws_url: String,
    /// Header name the relay uses to identify this client family.
    pub token_header: String,
    /// Admin kill switch for API-key auth (`CODEL_DISABLE_API_KEY_AUTH`, `[codel_com_config] disable_api_key_auth`).
    pub disable_api_key_auth: Option<bool>,
}


/// If `config` contains `[auth]`, copy its contents under `[codel_com_config]`.
/// `[codel_com_config]` takes precedence if both are present (explicit wins).
/// This lets customers write the shorter `[auth]` table instead of `[codel_com_config]`.
pub fn expand_auth_alias(config: &toml::Value) -> toml::Value {
    let mut config = config.clone();
    if let toml::Value::Table(ref mut table) = config
        && let Some(auth) = table.remove("auth")
    {
        if let Some(gcc) = table.get_mut("codel_com_config") {
            if let (toml::Value::Table(gcc_table), toml::Value::Table(auth_table)) = (gcc, &auth) {
                for (k, v) in auth_table {
                    gcc_table.entry(k.clone()).or_insert(v.clone());
                }
            }
        } else {
            table.insert("codel_com_config".to_owned(), auth);
        }
    }
    config
}

impl CodelComConfig {
    /// Whether API-key auth is refused.
    ///
    /// The `CODEL_DISABLE_API_KEY_AUTH` env lockdown is read at call time and
    /// OR-ed in, so a lower-trust user `config.toml` cannot turn it back off.
    /// `requirements.toml` already wins by layer precedence.
    pub fn api_key_auth_disabled(&self) -> bool {
        self.disable_api_key_auth == Some(true) || env_lockdown_forced()
    }

    /// Parse `[codel_com_config]` (alias `[auth]`) out of an effective config document.
    /// A section that omits a field keeps the value `Default` resolves (env or built-in).
    ///
    /// # Errors
    /// Returns an error when the section is present but does not deserialize.
    pub fn from_effective_config(config: &toml::Value) -> Result<CodelComConfig, toml::de::Error> {
        match config.get("codel_com_config").or_else(|| config.get("auth")) {
            Some(section) => section.clone().try_into(),
            None => Ok(CodelComConfig::default()),
        }
    }

    /// The `auth.json` scope key for this config.
    pub fn auth_scope(&self) -> String {
        AUTH_SCOPE_KEY.to_owned()
    }
}

impl Default for CodelComConfig {
    fn default() -> Self {
        Self {
            codel_ws_origin: std::env::var("CODEL_WS_ORIGIN")
                .unwrap_or_else(|_| PROD_WS_ORIGIN.to_owned()),
            codel_ws_url: std::env::var("CODEL_WS_URL")
                .unwrap_or_else(|_| PROD_RELAY_WS_URL.to_owned()),
            token_header: "codel-cli".to_owned(),
            disable_api_key_auth: std::env::var("CODEL_DISABLE_API_KEY_AUTH")
                .ok()
                .map(|v| env_flag_enabled(&v)),
        }
    }
}

/// Parses a boolean env-var value for codel's on/off flags.
/// Bare presence enables the flag, but falsy spellings (`0`, `false`, `off`, `no`, empty) count as disabled.
/// `CODEL_DISABLE_API_KEY_AUTH=false` therefore does NOT enable the flag.
fn env_flag_enabled(value: &str) -> bool {
    !matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "" | "0" | "false" | "off" | "no"
    )
}

/// True when the admin has set `CODEL_DISABLE_API_KEY_AUTH` to a truthy value in the process environment.
/// It is read at call time and OR-ed into [`CodelComConfig::api_key_auth_disabled`], so a user-layer `config.toml` cannot override the lockdown.
fn env_lockdown_forced() -> bool {
    std::env::var("CODEL_DISABLE_API_KEY_AUTH")
        .ok()
        .is_some_and(|v| env_flag_enabled(&v))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_flag_enabled_treats_falsy_spellings_as_off() {
        for v in ["", "0", "false", "FALSE", "off", "no", "  off  "] {
            assert!(!env_flag_enabled(v), "{v:?} must count as off");
        }
        for v in ["1", "true", "yes", "on", "anything"] {
            assert!(env_flag_enabled(v), "{v:?} must count as on");
        }
    }

    #[test]
    fn the_kill_switch_reads_the_config_field() {
        let cfg = CodelComConfig {
            disable_api_key_auth: Some(true),
            ..CodelComConfig::default()
        };
        assert!(cfg.api_key_auth_disabled());
    }

    #[test]
    fn the_scope_key_is_stable() {
        assert_eq!(
            CodelComConfig::default().auth_scope(),
            AUTH_SCOPE_KEY,
            "the scope key must not drift: an auth.json entry written by an earlier build has to keep resolving"
        );
    }
}
