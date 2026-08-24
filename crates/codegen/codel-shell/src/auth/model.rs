use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub(crate) const TOKEN_TTL: Duration = Duration::days(30);
const DEFAULT_EARLY_INVALIDATION_SECS: u64 = 300; // 5 minutes

/// Legacy auth.json scope key. Fallback for old devbox auth files.
pub(super) const LEGACY_SCOPE: &str = "https://accounts.codel/sign-in";

/// auth.json scope key for plain API key auth (desktop login, per-model `api_key`).
pub const API_KEY_SCOPE: &str = "codel::api_key";

const BLOCKED_REASON_NO_LOGS: &str = "BLOCKED_REASON_NO_LOGS";
const BLOCKED_REASON_NO_LOGS_MODERATED: &str = "BLOCKED_REASON_NO_LOGS_MODERATED";

/// Fresh-credential / missing-field default: opted out until the user or
/// server enrichment opts in. Single source for `CodelAuth`, `AuthMeta`, and
/// every login-path constructor so the sides cannot drift.
pub(crate) fn default_coding_data_retention_opt_out() -> bool {
    true
}

/// Token provenance (debugging/auth.json only -- no code branches on this).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AuthMode {
    /// Plain API key (e.g. from codel-desktop login or a per-model `api_key`)
    ApiKey,
}

/// Wire value of `principal_type` for team OAuth principals (capitalized by
/// the auth service). Single source for every comparison site.
pub(crate) const TEAM_PRINCIPAL_TYPE: &str = "Team";

#[derive(Clone, Serialize, Deserialize)]
pub struct CodelAuth {
    pub key: String,
    pub auth_mode: AuthMode,
    pub create_time: DateTime<Utc>,
    pub user_id: String,
    pub email: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_image_asset_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub principal_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub principal_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub team_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub team_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub team_role: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub organization_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub organization_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub organization_role: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_blocked_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub team_blocked_reasons: Vec<String>,
    /// Defaults to `true` (opted out) for safer consumer privacy until the
    /// user explicitly shares or server enrichment sets the team preference.
    #[serde(default = "default_coding_data_retention_opt_out")]
    pub coding_data_retention_opt_out: bool,

    /// Deprecated. Kept for deserializing existing auth.json files.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub has_codel_code_access: Option<bool>,

    /// Server-provided expiration.
    /// When present, takes precedence over the hardcoded `TOKEN_TTL`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<DateTime<Utc>>,
}

impl std::fmt::Debug for CodelAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CodelAuth")
            .field("key", &token_suffix(&self.key))
            .field("auth_mode", &self.auth_mode)
            .field("user_id", &self.user_id)
            .field("expires_at", &self.expires_at)
            .finish_non_exhaustive()
    }
}

impl CodelAuth {
    /// Seconds since this credential was minted. Negative when the local
    /// clock stepped back past `create_time` (NTP correction, VM restore, or
    /// a sibling machine's clock via an adopted auth.json) — `create_time`
    /// is always stamped from the minting machine's local clock.
    pub(crate) fn mint_age_seconds(&self) -> i64 {
        Utc::now()
            .signed_duration_since(self.create_time)
            .num_seconds()
    }

    /// Always `false` — only API key auth is supported; no first-party
    /// session credentials exist.
    pub fn is_codel_auth(&self) -> bool {
        false
    }

    /// Always `false` — only API key auth is supported; managed MCP
    /// connectors require session auth.
    pub fn is_managed_mcp_eligible(&self) -> bool {
        false
    }

    /// Always `false` — only API key auth is supported; API keys cannot
    /// access `supported_in_api: false` models.
    pub fn is_session_auth(&self) -> bool {
        false
    }

    pub fn is_team_principal(&self) -> bool {
        self.principal_type.as_deref() == Some(TEAM_PRINCIPAL_TYPE) && self.team_id.is_some()
    }

    /// `true` when the team has Zero Data Retention (ZDR) enabled.
    pub fn is_zdr_team(&self) -> bool {
        self.team_blocked_reasons
            .iter()
            .any(|r| r == BLOCKED_REASON_NO_LOGS || r == BLOCKED_REASON_NO_LOGS_MODERATED)
    }

    /// `true` when the team has ZDR or the user opted out of coding data
    /// retention. Use this for trace-upload and research-data gates.
    /// Product analytics and user-facing sync
    /// features should use `is_zdr_team()` directly.
    pub fn is_data_collection_disabled(&self) -> bool {
        self.is_zdr_team() || self.coding_data_retention_opt_out
    }

    /// Carry `/user`-derived fields from a previous auth so refresh rebuilds don't drop them.
    pub(crate) fn carry_user_profile_from(&mut self, prev: &CodelAuth) {
        self.user_id = prev.user_id.clone();
        self.email = prev.email.clone();
        self.principal_type = prev.principal_type.clone();
        self.principal_id = prev.principal_id.clone();
        self.team_id = prev.team_id.clone();
        self.team_name = prev.team_name.clone();
        self.team_role = prev.team_role.clone();
        self.organization_id = prev.organization_id.clone();
        self.organization_name = prev.organization_name.clone();
        self.organization_role = prev.organization_role.clone();
        self.user_blocked_reason = prev.user_blocked_reason.clone();
        self.team_blocked_reasons = prev.team_blocked_reasons.clone();
        self.coding_data_retention_opt_out = prev.coding_data_retention_opt_out;
    }
}

impl Default for CodelAuth {
    fn default() -> Self {
        Self {
            key: String::new(),
            auth_mode: AuthMode::ApiKey,
            create_time: Utc::now(),
            user_id: String::new(),
            email: None,
            first_name: None,
            last_name: None,
            profile_image_asset_id: None,
            principal_type: None,
            principal_id: None,
            team_id: None,
            team_name: None,
            team_role: None,
            organization_id: None,
            organization_name: None,
            organization_role: None,
            user_blocked_reason: None,
            team_blocked_reasons: vec![],
            coding_data_retention_opt_out: default_coding_data_retention_opt_out(),
            has_codel_code_access: None,
            expires_at: None,
        }
    }
}

#[cfg(test)]
impl CodelAuth {
    /// Returns a `CodelAuth` with sensible defaults for tests. Override fields
    /// with struct update syntax:
    /// ```ignore
    /// CodelAuth { key: "my-key".into(), ..CodelAuth::test_default() }
    /// ```
    pub fn test_default() -> Self {
        Self {
            key: "test-key".into(),
            user_id: "test-user".into(),
            // Tests that exercise collection gates need sharing enabled by
            // default; opt out explicitly when asserting the privacy path.
            coding_data_retention_opt_out: false,
            ..Default::default()
        }
    }
}

pub(crate) type AuthStore = BTreeMap<String, CodelAuth>;

/// User information from the cli-chat-proxy `GET /v1/user` endpoint.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UserInfo {
    pub(crate) user_id: String,
    #[serde(default)]
    pub(super) email: Option<String>,
    #[serde(default)]
    pub(super) first_name: Option<String>,
    #[serde(default)]
    pub(super) last_name: Option<String>,
    #[serde(default)]
    pub(super) profile_image_asset_id: Option<String>,
    #[serde(default)]
    pub(super) principal_type: Option<String>,
    #[serde(default)]
    pub(super) principal_id: Option<String>,
    #[serde(default)]
    pub(super) team_id: Option<String>,
    #[serde(default)]
    pub(super) team_name: Option<String>,
    #[serde(default)]
    pub(super) team_role: Option<String>,
    #[serde(default)]
    pub(super) organization_id: Option<String>,
    #[serde(default)]
    pub(super) organization_name: Option<String>,
    #[serde(default)]
    pub(super) organization_role: Option<String>,
    #[serde(default)]
    pub(super) user_blocked_reason: Option<String>,
    #[serde(default)]
    pub(super) team_blocked_reasons: Option<Vec<String>>,
    #[serde(default)]
    pub(super) coding_data_retention_opt_out: Option<bool>,
    /// Live subscription tier from the backend (only present when
    /// `?include=subscription` is passed to `/user`).
    #[serde(default)]
    pub(crate) subscription_tier: Option<String>,
}

/// Last 12 chars of a token string, safe for diagnostic logging.
/// Uses the tail because JWT access tokens all share the same base64
/// header prefix (`eyJ0eXAiOiJh…`); the tail (signature bytes) is
/// unique per token and makes `key_changed` / `is_stale_snapshot`
/// diagnostics meaningful.
pub(crate) fn token_suffix(t: &str) -> &str {
    let len = t.len();
    if len > 12 { &t[len - 12..] } else { t }
}

/// Look up auth from the store by scope key.
pub fn lookup_auth(map: &AuthStore, scope: &str) -> Option<CodelAuth> {
    map.get(scope).cloned().or_else(|| {
        if scope == LEGACY_SCOPE {
            None
        } else {
            map.get(LEGACY_SCOPE).cloned()
        }
    })
}

/// Early-invalidation buffer. Override with `CODEL_AUTH_EARLY_INVALIDATION_SECS`
/// for testing (e.g. `=5` to shrink the buffer to 5 seconds).
pub(super) fn early_invalidation() -> Duration {
    std::env::var("CODEL_AUTH_EARLY_INVALIDATION_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .map(|s| Duration::seconds(s as i64))
        .unwrap_or_else(|| Duration::seconds(DEFAULT_EARLY_INVALIDATION_SECS as i64))
}

pub(crate) fn is_expired(auth: &CodelAuth) -> bool {
    is_expired_with_buffer(auth, early_invalidation())
}

/// Like [`is_expired`] but with an explicit pre-expiry buffer. Pass
/// `Duration::zero()` for actual (hard) expiry — the instant the token would
/// really be rejected on the wire, with no early-invalidation margin.
pub(crate) fn is_expired_with_buffer(auth: &CodelAuth, buffer: Duration) -> bool {
    if let Some(expires_at) = auth.expires_at {
        Utc::now() >= (expires_at - buffer)
    } else {
        let age = Utc::now().signed_duration_since(auth.create_time);
        age >= (TOKEN_TTL - buffer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookup_auth_returns_api_key_token() {
        let mut map = AuthStore::new();
        map.insert(
            "scope".into(),
            CodelAuth {
                key: "k".into(),
                auth_mode: AuthMode::ApiKey,
                create_time: Utc::now(),
                user_id: "u".into(),
                ..Default::default()
            },
        );
        assert!(lookup_auth(&map, "scope").is_some());
    }

    #[test]
    fn lookup_auth_returns_none_for_missing_scope() {
        let map = AuthStore::new();
        assert!(lookup_auth(&map, "scope").is_none());
    }

    #[test]
    fn api_key_auth_is_not_session_auth() {
        let auth = CodelAuth {
            key: "k".into(),
            auth_mode: AuthMode::ApiKey,
            create_time: Utc::now(),
            user_id: "u".into(),
            ..Default::default()
        };
        assert!(!auth.is_session_auth());
        assert!(!auth.is_codel_auth());
        assert!(!auth.is_managed_mcp_eligible());
    }

    /// subscriptionTier present → deserializes to Some.
    #[test]
    fn user_info_subscription_tier_present() {
        let json = r#"{
            "userId": "u1",
            "subscriptionTier": "SuperCodelPro"
        }"#;
        let info: UserInfo = serde_json::from_str(json).unwrap();
        assert_eq!(info.subscription_tier.as_deref(), Some("SuperCodelPro"));
    }

    /// subscriptionTier absent → deserializes to None (backwards compat).
    #[test]
    fn user_info_subscription_tier_absent() {
        let json = r#"{"userId": "u1"}"#;
        let info: UserInfo = serde_json::from_str(json).unwrap();
        assert!(info.subscription_tier.is_none());
    }

    /// subscriptionTier null → deserializes to None.
    #[test]
    fn user_info_subscription_tier_null() {
        let json = r#"{"userId": "u1", "subscriptionTier": null}"#;
        let info: UserInfo = serde_json::from_str(json).unwrap();
        assert!(info.subscription_tier.is_none());
    }

    /// subscriptionTier empty string → deserializes to Some("").
    /// The paywall poller treats this as "no subscription" (line 230:
    /// `Some(tier) if !tier.is_empty()`) and keeps polling.
    #[test]
    fn user_info_subscription_tier_empty_string() {
        let json = r#"{"userId": "u1", "subscriptionTier": ""}"#;
        let info: UserInfo = serde_json::from_str(json).unwrap();
        assert_eq!(info.subscription_tier.as_deref(), Some(""));
    }

    /// Pre-default auth.json (no coding_data_retention_opt_out key) must
    /// deserialize as opted-out, not the old fail-open false.
    #[test]
    fn missing_coding_data_retention_opt_out_deserializes_opted_out() {
        let json = r#"{
            "key": "k",
            "auth_mode": "api_key",
            "create_time": "2020-01-01T00:00:00Z",
            "user_id": "u"
        }"#;
        let auth: CodelAuth = serde_json::from_str(json).unwrap();
        assert!(
            auth.coding_data_retention_opt_out,
            "missing field must default to opted-out"
        );
        assert!(default_coding_data_retention_opt_out());
        assert!(CodelAuth::default().coding_data_retention_opt_out);
    }
}
