use serde::{Deserialize, Serialize};

/// Gate information shown to the user when access is restricted.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GateInfo {
    pub message: String,
    pub url: Option<String>,
    pub label: Option<String>,
}

/// Metadata attached to authenticate responses for the client.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AuthMeta {
    pub email: Option<String>,
    pub auth_mode: Option<String>,
    pub team_id: Option<String>,
    pub team_name: Option<String>,
    pub is_zdr: bool,
    pub team_role: Option<String>,
    pub coding_data_retention_opt_out: bool,
    pub show_resolved_model: Option<bool>,
    pub gate: Option<GateInfo>,
    pub subscription_tier: Option<String>,
    /// Client request sequence for single-flight cancellation.
    #[serde(skip)]
    pub request_seq: Option<u64>,
    /// Whether the client is headless (no browser).
    #[serde(skip)]
    pub headless: bool,
    /// Whether this is a re-authentication request.
    #[serde(skip)]
    pub reauth: bool,
    /// Whether to force OAuth loopback flow.
    #[serde(skip)]
    pub use_oauth: bool,
    /// Whether to force interactive login.
    #[serde(skip)]
    pub force_interactive: bool,
}


