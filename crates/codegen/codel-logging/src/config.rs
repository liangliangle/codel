//! These types live here so the data-collector engine can construct a [`TelemetryClient`](crate::client::TelemetryClient) without depending on shell.
//!
//! Shell still re-exports these types from their original paths so existing call sites (and `Config` derive impls) compile unchanged.
use serde::{Deserialize, Serialize};
use codel_env::env_bool;
/// Telemetry mode: `true`/`false` (legacy bool) or `"session_metrics"` (string). `Disabled`: nothing sent (enterprise
/// default); `SessionMetrics`: metadata-only lifecycle events, no content; `Enabled`: full product telemetry (events and
/// Mixpanel).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TelemetryMode {
    #[default]
    Disabled,
    SessionMetrics,
    Enabled,
}
impl TelemetryMode {
    pub fn is_disabled(&self) -> bool {
        matches!(self, Self::Disabled)
    }
    pub fn is_enabled(&self) -> bool {
        matches!(self, Self::Enabled)
    }
    /// True for both `SessionMetrics` and `Enabled`.
    pub fn session_metrics_enabled(&self) -> bool {
        matches!(self, Self::SessionMetrics | Self::Enabled)
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" | "enabled" | "full" => Some(Self::Enabled),
            "0" | "false" | "no" | "off" | "disabled" => Some(Self::Disabled),
            "session-metrics" | "session_metrics" => Some(Self::SessionMetrics),
            _ => None,
        }
    }
}
#[cfg(test)]
mod telemetry_mode_tests {
    use super::TelemetryMode;
    /// A parent process hands its resolved mode to spawned children via `CODEL_TELEMETRY_ENABLED={mode}` (Display).
    /// Every Display output must parse back to the same mode.
    #[test]
    fn display_round_trips_through_parse() {
        for mode in [
            TelemetryMode::Enabled,
            TelemetryMode::Disabled,
            TelemetryMode::SessionMetrics,
        ] {
            assert_eq!(
                TelemetryMode::parse(&mode.to_string()),
                Some(mode),
                "Display value for {mode:?} must parse back to itself"
            );
        }
    }
}
impl std::fmt::Display for TelemetryMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Disabled => write!(f, "false"),
            Self::SessionMetrics => write!(f, "session_metrics"),
            Self::Enabled => write!(f, "true"),
        }
    }
}
impl From<bool> for TelemetryMode {
    fn from(b: bool) -> Self {
        if b { Self::Enabled } else { Self::Disabled }
    }
}
impl serde::Serialize for TelemetryMode {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Disabled => serializer.serialize_bool(false),
            Self::Enabled => serializer.serialize_bool(true),
            Self::SessionMetrics => serializer.serialize_str("session_metrics"),
        }
    }
}
/// Wire format for `[features] telemetry`: accepts `true`, `false`, or `"session_metrics"`.
#[derive(serde::Deserialize)]
#[serde(untagged)]
enum TelemetryModeValue {
    Bool(bool),
    Str(String),
}
impl<'de> serde::Deserialize<'de> for TelemetryMode {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        match TelemetryModeValue::deserialize(deserializer)? {
            TelemetryModeValue::Bool(b) => Ok(Self::from(b)),
            TelemetryModeValue::Str(s) => Ok(Self::parse(&s).unwrap_or_else(|| {
                tracing::warn!(
                    value = %s,
                    "TELEMETRY_MODE_UNKNOWN: unrecognized telemetry mode; treating as disabled",
                );
                Self::Disabled
            })),
        }
    }
}
/// Parse an env var as a `TelemetryMode`. Returns `None` if unset or empty.
pub fn env_telemetry_mode(name: &str) -> Option<TelemetryMode> {
    let value = std::env::var(name).ok()?;
    TelemetryMode::parse(&value)
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TelemetryConfig {
    /// Declared for `serde_ignored`. Actual toggle is `[features] telemetry`.
    #[serde(default)]
    pub enabled: Option<bool>,
    pub events_url: Option<String>,
    pub events_api_key: Option<String>,
    pub mixpanel_token: Option<String>,
    pub mixpanel_enabled: bool,
    /// `None` inherits from `[features] telemetry`; `Some(false)` disables GCS uploads only.
    pub trace_upload: Option<bool>,
}
fn internal_defaults() -> (Option<String>, Option<String>, Option<String>, bool) {
    (None, None, None, false)
}
fn build_env_default(value: Option<&'static str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_owned)
}
impl Default for TelemetryConfig {
    fn default() -> Self {
        let (baked_url, baked_key, baked_token, baked_enabled) = internal_defaults();
        let build_url = build_env_default(option_env!("CODEL_TELEMETRY_BUILD_EVENTS_URL"));
        let build_key = build_env_default(option_env!("CODEL_TELEMETRY_BUILD_EVENTS_API_KEY"));
        let build_token = build_env_default(option_env!("CODEL_TELEMETRY_BUILD_MIXPANEL_TOKEN"));
        let mixpanel_enabled = baked_enabled || build_token.is_some();
        let (events_url, events_api_key, mixpanel_token) = (
            build_url.or(baked_url),
            build_key.or(baked_key),
            build_token.or(baked_token),
        );
        Self {
            enabled: None,
            events_url,
            events_api_key,
            mixpanel_token,
            mixpanel_enabled,
            trace_upload: None,
        }
    }
}
impl TelemetryConfig {
    pub fn apply_env_overrides(&mut self) {
        self.normalize();
        if let Some(value) = Self::env_override("CODEL_TELEMETRY_EVENTS_URL") {
            self.events_url = value;
        }
        if let Some(value) = Self::env_override("CODEL_TELEMETRY_EVENTS_API_KEY") {
            self.events_api_key = value;
        }
        if let Some(value) = Self::env_override("CODEL_TELEMETRY_MIXPANEL_TOKEN") {
            self.mixpanel_token = value;
        }
        if let Some(value) = env_bool("CODEL_TELEMETRY_MIXPANEL_ENABLED") {
            self.mixpanel_enabled = value;
        }
        if let Some(value) = env_bool("CODEL_TELEMETRY_TRACE_UPLOAD") {
            self.trace_upload = Some(value);
        }
    }
    fn normalize(&mut self) {
        self.events_url = Self::normalize_optional_string(self.events_url.take());
        self.events_api_key = Self::normalize_optional_string(self.events_api_key.take());
        self.mixpanel_token = Self::normalize_optional_string(self.mixpanel_token.take());
    }
    fn env_override(name: &str) -> Option<Option<String>> {
        match std::env::var(name) {
            Ok(value) => Some(Self::normalize_optional_string(Some(value))),
            Err(_) => None,
        }
    }
    fn normalize_optional_string(value: Option<String>) -> Option<String> {
        value.and_then(|raw| {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        })
    }
}
/// Derive a stable deployment ID (UUIDv5) from the deployment key.
pub fn deployment_id_from_key(key: &str) -> String {
    uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, key.as_bytes()).to_string()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn default_is_build_env_layer_when_feature_off() {
        let cfg = TelemetryConfig::default();
        let url = build_env_default(option_env!("CODEL_TELEMETRY_BUILD_EVENTS_URL"));
        let key = build_env_default(option_env!("CODEL_TELEMETRY_BUILD_EVENTS_API_KEY"));
        let token = build_env_default(option_env!("CODEL_TELEMETRY_BUILD_MIXPANEL_TOKEN"));
        assert_eq!(cfg.mixpanel_enabled, token.is_some());
        assert_eq!(cfg.events_url, url);
        assert_eq!(cfg.events_api_key, key);
        assert_eq!(cfg.mixpanel_token, token);
    }

    #[test]
    fn disarm_baked_sinks_keeps_explicit_sinks() {
        let mut cfg = TelemetryConfig {
            events_url: Some("http://127.0.0.1:9/events".into()),
            mixpanel_token: Some("explicit-token".into()),
            mixpanel_enabled: true,
            ..TelemetryConfig::default()
        };
        let key_before = cfg.events_api_key.clone();
        assert!(
            !cfg.disarm_baked_sinks(),
            "explicit sinks must not count as cleared"
        );
        assert_eq!(
            (
                cfg.events_url.as_deref(),
                cfg.events_api_key == key_before,
                cfg.mixpanel_token.as_deref(),
                cfg.mixpanel_enabled
            ),
            (
                Some("http://127.0.0.1:9/events"),
                true,
                Some("explicit-token"),
                true
            )
        );
    }

    #[test]
    fn build_env_default_normalizes() {
        assert_eq!(build_env_default(None), None);
        assert_eq!(build_env_default(Some("")), None);
        assert_eq!(build_env_default(Some(" \t ")), None);
        assert_eq!(build_env_default(Some(" key ")), Some("key".to_owned()));
    }

}
