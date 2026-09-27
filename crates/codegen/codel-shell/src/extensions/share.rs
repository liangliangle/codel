//! `codel/share_session` extension handler.
//!
//! Loads a local session, exports it, uploads the message payload to cloud storage via a signed URL, and asks the backend for a public share URL.
//! The signed URL lets large sessions bypass the proxy/backend body-size limits.
//! Best-effort metadata upload is fire-and-forget on the spawned task.

use agent_client_protocol as acp;

use super::{ExtResult, parse_params, to_raw_response};
use crate::agent::MvpAgent;
use crate::remote::client::BackendClient;
use crate::session::export::{ExportedMessage, ExportedSession};
use crate::session::info::Info as SessionInfo;
use crate::session::persistence::list_summaries;
use crate::session::share::{ShareSessionRequest, ShareSessionResponse};
use crate::upload::trace::{SessionMetadataType, upload_session_metadata};
use codel_logging::id::agent_id;

#[tracing::instrument(skip_all, fields(method = %args.method))]
pub async fn handle(agent: &MvpAgent, args: &acp::ExtRequest) -> ExtResult {
    match args.method.as_ref() {
        "codel/share_session" => {
            tracing::info!("handling share session request");
            handle_share_session(agent, args).await
        }
        _ => Err(acp::Error::method_not_found()),
    }
}

async fn handle_share_session(agent: &MvpAgent, args: &acp::ExtRequest) -> ExtResult {
    let request: ShareSessionRequest = parse_params(args)?;

    let auth = require_codel_auth_for_share(&agent.auth_manager)?;

    // Remote settings / feature-flag gate: sharing_enabled defaults to false and is only enabled for eligible accounts
    let sharing_enabled = agent
        .cfg
        .borrow()
        .remote_settings
        .as_ref()
        .and_then(|rs| rs.sharing_enabled)
        .unwrap_or(false);
    if !sharing_enabled {
        return Err(
            acp::Error::invalid_params().data("Session sharing is not available for your account.")
        );
    }

    // Only block for ZDR teams (hard data-retention policy), not for coding-data-retention opt-out; sharing is user-initiated
    if auth.is_zdr_team() {
        return Err(acp::Error::invalid_params()
            .data("Session sharing is disabled for your team's data retention policy"));
    }

    // Find session info by searching through summaries
    let summaries = list_summaries(None).await.map_err(|e| {
        acp::Error::internal_error().data(format!("Failed to list sessions: {}", e))
    })?;

    let summary = summaries
        .iter()
        .find(|s| s.info.id.0.as_ref() == request.session_id.as_str())
        .ok_or_else(|| acp::Error::resource_not_found(Some("Session not found".into())))?;

    // Get turn number from the summary we already loaded
    let current_turn = summary.next_trace_turn.saturating_sub(1);

    let info = SessionInfo {
        id: acp::SessionId::new(request.session_id.clone()),
        cwd: summary.info.cwd.clone(),
    };

    let exported = ExportedSession::from_local_session(&info)
        .await
        .map_err(|e| acp::Error::internal_error().data(format!("Failed to load session: {}", e)))?;

    if exported.messages.is_empty() {
        return Err(acp::Error::invalid_params().data("No messages to share yet"));
    }

    // Obtain trace context once; used for the signed URL upload and then moved into the spawned metadata task
    let trace_context = agent.get_trace_context(&info, current_turn).await;

    // Upload session data to cloud storage via signed URL so large sessions don't hit the 413 body-size limit on the backend API
    if let Some(ref ctx) = trace_context {
        upload_share_data_to_gcs(
            &request.session_id,
            &exported.messages,
            &ctx.gcs_config,
            Some(agent.auth_manager.clone()),
        )
        .await;
    }

    // Upload to backend and get share URL.
    // The `save_session_data` call may fail with 413 for very large sessions; that is acceptable because the data is already in cloud storage
    let client = BackendClient::new().with_auth_manager(agent.auth_manager.clone());
    let agent_id = agent_id();
    let share_url = client
        .share_session(&exported, &agent_id)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "Failed to share session with backend");
            acp::Error::internal_error().data(format!("Failed to share session: {}", e))
        })?;

    // Upload share metadata to cloud storage (best-effort, fire-and-forget).
    if let Some(mut ctx) = trace_context {
        ctx.gcs_config.gcs_prefix = None;
        tokio::spawn(async move {
            upload_session_metadata(&ctx, SessionMetadataType::Share).await;
        });
    }

    let response = ShareSessionResponse { share_url };
    to_raw_response(&response)
}

/// Upload session messages to cloud storage via signed URL (best-effort).
/// Serialises the messages to JSON and uploads them under `share/{session_id}_{timestamp}_data.json`.
/// On failure the error is logged as a warning; the caller is expected to fall back to the backend API.
async fn upload_share_data_to_gcs(
    session_id: &str,
    messages: &[ExportedMessage],
    gcs_config: &crate::session::repo_changes::TraceExportConfig,
    auth_manager: Option<std::sync::Arc<codel_login::AuthManager>>,
) {
    let data_json = match serde_json::to_vec(messages) {
        Ok(json) => json,
        Err(e) => {
            tracing::warn!(
                session_id = %session_id,
                error = %e,
                "Failed to serialise share data"
            );
            return;
        }
    };

    let timestamp = chrono::Utc::now().format("%Y%m%dT%H%M%SZ");
    let gcs_path = format!("share/{}_{}_data.json", session_id, timestamp);

    use crate::upload::gcs::WithAuth as _;
    if let Err(e) = codel_file_utils::gcs::upload_bytes_signed(
        &gcs_config.with_auth(auth_manager),
        &gcs_path,
        &data_json,
        "application/json",
    )
    .await
    {
        tracing::warn!(
            session_id = %session_id,
            error = %e,
            "Failed to upload share data via signed URL, \
             falling back to backend API"
        );
    }
}

fn require_codel_auth_for_share(
    auth_manager: &codel_login::AuthManager,
) -> Result<codel_login::CodelAuth, acp::Error> {
    super::auth_gate::require_codel_auth(
        auth_manager,
        "Authentication required to share session",
        "Share session is disabled. Run `codel login` to authenticate.",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, Utc};
    use std::sync::Arc;
    use tempfile::tempdir;
    use codel_login::CodelComConfig;
    use codel_login::{AuthMode, CodelAuth};




    #[test]
    fn share_fails_with_no_auth_at_all() {
        let dir = tempdir().expect("tempdir");
        let mgr = Arc::new(codel_login::AuthManager::new(
            dir.path(),
            CodelComConfig::default(),
        ));
        assert!(require_codel_auth_for_share(&mgr).is_err());
    }

    #[test]
    fn share_rejects_non_codel_auth_with_actionable_codel_login_message() {
        let dir = tempdir().expect("tempdir");
        let mgr = Arc::new(codel_login::AuthManager::new(
            dir.path(),
            CodelComConfig::default(),
        ));

        // API key is the simplest non-Codel credential (External and enterprise OIDC are also rejected the same way)
        let non_codel = CodelAuth {
            auth_mode: AuthMode::ApiKey,
            key: "codel-test-key".into(),
            create_time: Utc::now(),
            ..Default::default()
        };
        mgr.hot_swap(non_codel);

        let err = require_codel_auth_for_share(&mgr)
            .expect_err("non-Codel accounts (API key, External, enterprise IdP) must be rejected");

        // Test the *exact* actionable data string for the non-Codel path (distinct from the generic "Authentication required to share session" path)
        let serialized =
            serde_json::to_value(&err).expect("acp::Error serializes to JSON-RPC shape");
        let data = serialized
            .get("data")
            .and_then(|v| v.as_str())
            .expect("auth_required error carries a data string");

        assert_eq!(
            data,
            "Share session is disabled. Run `codel login` to authenticate."
        );
    }
}
