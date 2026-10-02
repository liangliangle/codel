//! Configuration types the shell resolves and hands to this crate.
//!
//! The fork ships no analytics transport, so the only sink left to configure is
//! the support-bundle upload (trace upload).

use serde::{Deserialize, Serialize};

use codel_env::env_bool;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TelemetryConfig {
    /// Declared for `serde_ignored`: configs written by an older build still carry it.
    #[serde(default)]
    pub enabled: Option<bool>,
    /// `Some(false)` disables the support-bundle upload; `None` defers to the other layers.
    pub trace_upload: Option<bool>,
}

impl Default for TelemetryConfig {
    fn default() -> Self {
        Self {
            enabled: None,
            trace_upload: None,
        }
    }
}

impl TelemetryConfig {
    pub fn apply_env_overrides(&mut self) {
        if let Some(value) = env_bool("CODEL_TELEMETRY_TRACE_UPLOAD") {
            self.trace_upload = Some(value);
        }
    }
}

/// Derive a stable deployment ID (UUIDv5) from the deployment key.
pub fn deployment_id_from_key(key: &str) -> String {
    uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, key.as_bytes()).to_string()
}
