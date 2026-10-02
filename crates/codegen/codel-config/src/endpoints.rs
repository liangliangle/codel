//! The `[endpoints]` config table, its environment variable overrides, and the URLs resolved from it.
//!
//! The auxiliary services (feedback, trace upload, managed config, telemetry) resolve to the cli-chat-proxy.
//! Only API-key inference uses `codel_api_base_url`.
use codel_env::{PROD_CLI_CHAT_PROXY_BASE_URL, env_string};
use serde::{Deserialize, Serialize};
pub const CLI_CHAT_PROXY_BASE_URL_DEFAULT: &str = PROD_CLI_CHAT_PROXY_BASE_URL;
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct EndpointsConfig {
    /// When this is `None`, `proxy_url` returns `CLI_CHAT_PROXY_BASE_URL_DEFAULT`.
    /// `Some` means someone configured it, even when the value is the default URL.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cli_chat_proxy_base_url: Option<String>,
    /// Base URL for the public Codel API.
    pub codel_api_base_url: String,
    /// An extra access header value for matching first-party hosts, used only with the optional non-production feature.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alpha_test_key: Option<String>,
    /// Env: `CODEL_MODELS_BASE_URL`. Setting it makes `has_custom_endpoint` true.
    /// The models list URL defaults to `{models_base_url}/models`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub models_base_url: Option<String>,
    /// Env: `CODEL_MODELS_LIST_URL`. Overrides the default `{base}/models` list URL.
    #[serde(alias = "models_endpoint", skip_serializing_if = "Option::is_none")]
    pub models_list_url: Option<String>,
    /// Env: `CODEL_FEEDBACK_BASE_URL`. Where feedback submissions go.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub feedback_base_url: Option<String>,
    /// Env: `CODEL_TRACE_UPLOAD_URL`. Where trace uploads go.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trace_upload_url: Option<String>,
    /// Env: `CODEL_TRACE_UPLOAD_BUCKET`. A `gs://` or `s3://` bucket that receives uploads directly, without the proxy.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trace_upload_bucket: Option<String>,
    /// Env: `CODEL_TRACE_UPLOAD_REGION`. AWS region (S3 only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trace_upload_region: Option<String>,
    /// Env: `CODEL_TRACE_UPLOAD_CREDENTIALS_FILE`. The path to a GCS service account key or an AWS credentials file.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trace_upload_credentials_file: Option<String>,
    /// Inline credentials as JSON or INI, preferred over `trace_upload_credentials_file`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trace_upload_credentials: Option<String>,
    /// Env: `CODEL_TRACE_UPLOAD_ENDPOINT_URL`. Custom S3-compatible endpoint.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trace_upload_endpoint_url: Option<String>,
    /// Env: `CODEL_DEPLOYMENT_KEY`. The management API key for an enterprise deployment.
    /// Telemetry and service requests carry it to identify the deployment.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deployment_key: Option<String>,
    /// Env: `CODEL_MANAGED_CONFIG_URL`. The managed config endpoint.
    /// The default is `{proxy_url()}/deployment/config`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub managed_config_url: Option<String>,
    /// `load_management_api_key_sync()` reads this key.
    /// Declaring the field stops `serde_ignored` from reporting the key as unrecognized.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub management_api_key: Option<String>,
    /// `load_gcs_service_account_key_sync()` reads this key.
    /// Declaring the field stops `serde_ignored` from reporting the key as unrecognized.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gcs_service_account_key: Option<String>,
}
/// A blank or whitespace-only override counts as unset.
fn blank_as_unset(opt: &Option<String>) -> Option<String> {
    opt.as_deref()
        .filter(|s| !s.trim().is_empty())
        .map(str::to_owned)
}
impl EndpointsConfig {
    pub fn has_custom_endpoint(&self) -> bool {
        self.models_base_url.is_some() || self.models_list_url.is_some()
    }
    /// `default()` with the managed and requirements `[endpoints]` overrides merged on top.
    /// Startup fetches use it to reach the configured endpoints.
    pub fn from_effective_config() -> Self {
        match crate::effective_config::load_effective_config() {
            Ok(cfg) => Self::from_config_value(&cfg),
            Err(_) => Self::default(),
        }
    }
    /// Merges the `[endpoints]` table from `config` over `default()`.
    /// Only the resolver methods apply defaults.
    pub fn from_config_value(config: &toml::Value) -> Self {
        let default = Self::default();
        let mut base = match toml::Value::try_from(default) {
            Ok(v) => v,
            Err(_) => return Self::default(),
        };
        if let Some(endpoints) = config.get("endpoints") {
            crate::deep_merge_toml(&mut base, endpoints);
        }
        let resolved: Self = base.try_into().unwrap_or_default();
        resolved
    }
    /// The cli-chat-proxy base URL for the auxiliary services and for inference with OAuth or session auth.
    pub fn proxy_url(&self) -> String {
        blank_as_unset(&self.cli_chat_proxy_base_url)
            .unwrap_or_else(|| CLI_CHAT_PROXY_BASE_URL_DEFAULT.to_owned())
    }
    pub fn resolve_inference_base_url(&self) -> String {
        self.models_base_url
            .clone()
            .unwrap_or_else(|| self.proxy_url())
    }
    pub fn resolve_feedback_base_url(&self) -> String {
        blank_as_unset(&self.feedback_base_url).unwrap_or_else(|| self.proxy_url())
    }
    pub fn resolve_trace_upload_url(&self) -> String {
        blank_as_unset(&self.trace_upload_url).unwrap_or_else(|| self.proxy_url())
    }
    /// The managed deployment config URL for `codel setup`.
    /// The deployment key sent here must reach the proxy, never the inference host.
    pub fn resolve_managed_config_url(&self) -> String {
        blank_as_unset(&self.managed_config_url).unwrap_or_else(|| {
            format!(
                "{}/deployment/config",
                self.proxy_url().trim_end_matches('/')
            )
        })
    }
    /// `None` means the upload falls back to the default cloud credentials.
    pub fn resolve_trace_credentials(&self) -> Option<String> {
        if let Some(inline) = blank_as_unset(&self.trace_upload_credentials) {
            return Some(inline.trim().to_owned());
        }
        self.trace_upload_credentials_file
            .as_deref()
            .and_then(|path| {
                std::fs::read_to_string(path)
                    .inspect_err(|e| {
                        tracing::warn!(
                            path = %path,
                            error = %e,
                            "Failed to read trace upload credentials file"
                        );
                    })
                    .ok()
            })
    }
    pub fn resolve_models_list_url(&self) -> String {
        if let Some(ref url) = self.models_list_url {
            return url.clone();
        }
        let base = self
            .models_base_url
            .clone()
            .unwrap_or_else(|| self.proxy_url());
        format!("{}/models", base)
    }
}
const CODEL_API_BASE_URL_DEFAULT: &str = "https://api.codel.dev/v1";
impl Default for EndpointsConfig {
    fn default() -> Self {
        Self {
            cli_chat_proxy_base_url: std::env::var("CODEL_CLI_CHAT_PROXY_BASE_URL").ok(),
            codel_api_base_url: std::env::var("CODEL_CODEL_API_BASE_URL")
                .unwrap_or_else(|_| CODEL_API_BASE_URL_DEFAULT.to_owned()),
            alpha_test_key: None,
            models_base_url: env_string("CODEL_MODELS_BASE_URL"),
            models_list_url: env_string("CODEL_MODELS_LIST_URL"),
            feedback_base_url: env_string("CODEL_FEEDBACK_BASE_URL"),
            trace_upload_url: env_string("CODEL_TRACE_UPLOAD_URL"),
            trace_upload_bucket: env_string("CODEL_TRACE_UPLOAD_BUCKET"),
            trace_upload_region: env_string("CODEL_TRACE_UPLOAD_REGION"),
            trace_upload_credentials_file: env_string("CODEL_TRACE_UPLOAD_CREDENTIALS_FILE"),
            trace_upload_credentials: None,
            trace_upload_endpoint_url: env_string("CODEL_TRACE_UPLOAD_ENDPOINT_URL"),
            deployment_key: env_string("CODEL_DEPLOYMENT_KEY"),
            managed_config_url: env_string("CODEL_MANAGED_CONFIG_URL"),
            management_api_key: None,
            gcs_service_account_key: None,
        }
    }
}
#[cfg(test)]
#[path = "endpoints_tests.rs"]
mod tests;
