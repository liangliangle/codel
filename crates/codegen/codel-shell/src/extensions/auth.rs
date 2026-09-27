//! `codel/auth/*` and legacy `codel/{get,set}ApiKey` extension handlers.
//!
//! These methods let the client read/write the API key via the agent and drive the OAuth login flow.
//! The agent is the single source of truth for `auth.json`.

use agent_client_protocol as acp;
use serde::{Deserialize, Serialize};

use super::{ExtResult, parse_params, to_raw_response};
use crate::agent::MvpAgent;
use crate::session::ExtMethodResult;

#[tracing::instrument(skip_all, fields(method = %args.method))]
pub async fn handle(agent: &MvpAgent, args: &acp::ExtRequest) -> ExtResult {
    match args.method.as_ref() {
        "codel/auth/getBearerToken" => handle_get_bearer_token(agent).await,
        "codel/getApiKey" => handle_get_api_key(),
        "codel/setApiKey" => handle_set_api_key(args),
        "codel/auth/info" => handle_info(agent),
        "codel/auth/check_subscription" => handle_check_subscription(agent).await,
        "codel/auth/hydrate_team_capability" => handle_hydrate_team_capability(agent, args).await,
        _ => Err(acp::Error::method_not_found()),
    }
}

/// `pub` with both serde directions so the pager builds the request from the type the agent parses.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HydrateTeamCapabilityRequest {
    pub email: Option<String>,
    pub team_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HydrateTeamCapabilityResponse {
    /// Required but nullable: the handler always emits it, so an absent or misspelled key is a broken peer, not an unknown answer.
    #[serde(deserialize_with = "Option::deserialize")]
    pub can_administer_team: Option<bool>,
}

/// Backfills `canAdministerTeam` for a cached credential that predates it: startup only enriches a missing user id and the token fast path skips `/user`.
/// The ACP layer clones the agent's `Rc` into this request's task, so a dropped connection does not cancel the GET; nobody reads the answer, but the shared cache still fills.
async fn handle_hydrate_team_capability(agent: &MvpAgent, args: &acp::ExtRequest) -> ExtResult {
    let params: HydrateTeamCapabilityRequest = parse_params(args)?;
    let can_administer_team = agent
        .auth_manager
        .hydrate_can_administer_team(params.email.as_deref(), params.team_id.as_deref())
        .await;
    to_raw_response(&HydrateTeamCapabilityResponse {
        can_administer_team,
    })
}



async fn handle_get_bearer_token(agent: &MvpAgent) -> ExtResult {
    // Fail closed for session tokens: desktop resume treats non-null as success. Never return a hard-expired access token
    // Still return wire-valid session tokens and static user-supplied keys (process model key, env, or disk api_key) That keeps non-session sessions working when AuthManager has no OIDC entry
    let token = match agent.auth_manager.get_valid_token().await {
        Ok(token) => Some(token),
        Err(_) => agent
            .auth_manager
            .current_wire_valid()
            .map(|a| a.key)
            .or_else(|| agent.auth_manager.static_api_key_for_export()),
    };
    ExtMethodResult::success(serde_json::json!({ "token": token }))
        .to_ext_response()
        .map_err(|e| acp::Error::internal_error().data(e.to_string()))
}

fn handle_get_api_key() -> ExtResult {
    let key = crate::agent::auth_method::read_codel_api_key_env().ok();
    ExtMethodResult::success(serde_json::json!({ "key": key }))
        .to_ext_response()
        .map_err(|e| acp::Error::internal_error().data(e.to_string()))
}

fn handle_set_api_key(args: &acp::ExtRequest) -> ExtResult {
    let params: serde_json::Value = parse_params(args)?;
    let key = params.get("key").and_then(|v| v.as_str());
    let codel_home = crate::util::codel_home::codel_home();
    match key {
        Some(k) if !k.is_empty() => {
            codel_login::store_api_key(&codel_home, k)
                .map_err(|e| acp::Error::internal_error().data(e.to_string()))?;
            codel_login::auth_method::set_runtime_codel_api_key(k);
        }
        _ => {
            codel_login::clear_api_key(&codel_home)
                .map_err(|e| acp::Error::internal_error().data(e.to_string()))?;
            codel_login::auth_method::clear_runtime_codel_api_key();
        }
    }
    ExtMethodResult::success(serde_json::json!({ "ok": true }))
        .to_ext_response()
        .map_err(|e| acp::Error::internal_error().data(e.to_string()))
}







/// Re-checks the subscription once, for the retry button on the paywall screen.
/// Returns the updated auth response with gate info so the pager can refresh the gate state.
async fn handle_check_subscription(agent: &MvpAgent) -> ExtResult {
    agent.retry_subscription_check().await;
    let response = agent.auth_response_with_meta();
    to_raw_response(&serde_json::json!({
        "authenticated": response.meta.is_some(),
        "meta": response.meta,
    }))
}

/// Returns current auth method ID, user profile fields, and team/principal metadata.
fn handle_info(agent: &MvpAgent) -> ExtResult {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct AuthInfoResponse {
        method_id: Option<String>,
        email: Option<String>,
        first_name: Option<String>,
        last_name: Option<String>,
        /// `codel-asset://` URL resolved by the Electron protocol handler, or a full `http(s)://` URL passed through unchanged.
        profile_image_url: Option<String>,
        team_id: Option<String>,
        team_name: Option<String>,
        team_role: Option<String>,
        organization_id: Option<String>,
        organization_name: Option<String>,
        organization_role: Option<String>,
        principal_type: Option<String>,
        principal_id: Option<String>,
        user_blocked_reason: Option<String>,
        team_blocked_reasons: Vec<String>,
        coding_data_retention_opt_out: bool,
    }

    let method_id = agent
        .auth_method_id
        .load()
        .as_ref()
        .map(|m| m.0.to_string());
    let auth = agent.auth_manager.current_or_expired();
    let raw_asset_id = auth.as_ref().and_then(|a| a.profile_image_asset_id.clone());

    // Return a codel-asset:// URL that the Electron renderer resolves at display time via a custom protocol handler
    // The handler proxies through cli-chat-proxy's /asset endpoint; Electron's HTTP cache handles reuse
    // Nothing here touches a disk cache or the network
    let profile_image_url = match raw_asset_id.as_deref().filter(|k| !k.is_empty()) {
        Some(key) if key.starts_with("http://") || key.starts_with("https://") => {
            Some(key.to_owned())
        }
        Some(key) => Some(format!("codel-asset:///{key}")),
        None => None,
    };
    to_raw_response(&AuthInfoResponse {
        method_id,
        email: auth.as_ref().and_then(|a| a.email.clone()),
        first_name: auth.as_ref().and_then(|a| a.first_name.clone()),
        last_name: auth.as_ref().and_then(|a| a.last_name.clone()),
        profile_image_url,
        team_id: auth.as_ref().and_then(|a| a.team_id.clone()),
        team_name: auth.as_ref().and_then(|a| a.team_name.clone()),
        team_role: auth.as_ref().and_then(|a| a.team_role.clone()),
        organization_id: auth.as_ref().and_then(|a| a.organization_id.clone()),
        organization_name: auth.as_ref().and_then(|a| a.organization_name.clone()),
        organization_role: auth.as_ref().and_then(|a| a.organization_role.clone()),
        principal_type: auth.as_ref().and_then(|a| a.principal_type.clone()),
        principal_id: auth.as_ref().and_then(|a| a.principal_id.clone()),
        user_blocked_reason: auth.as_ref().and_then(|a| a.user_blocked_reason.clone()),
        team_blocked_reasons: auth
            .as_ref()
            .map(|a| a.team_blocked_reasons.clone())
            .unwrap_or_default(),
        // With no credential the privacy state is unknown, so report opted-out (fail closed)
        // This matches `AuthManager::allows_data_collection` and the CodelAuth Default
        coding_data_retention_opt_out: auth
            .as_ref()
            .map(|a| a.coding_data_retention_opt_out)
            .unwrap_or_else(codel_login::default_coding_data_retention_opt_out),
    })
}
