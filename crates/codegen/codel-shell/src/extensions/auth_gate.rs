use agent_client_protocol as acp;

use codel_login::{AuthManager, CodelAuth};

/// Require Codel auth from a sync context: with no `.await` to refresh, a token inside the client's early-invalidation buffer still counts.
pub(crate) fn require_codel_auth(
    auth_manager: &AuthManager,
    missing_message: &'static str,
    non_codel_message: &'static str,
) -> Result<CodelAuth, acp::Error> {
    let auth = auth_manager
        .current_or_expired()
        .ok_or_else(|| acp::Error::auth_required().data(missing_message))?;
    if !auth.is_codel_auth() {
        return Err(acp::Error::auth_required().data(non_codel_message));
    }
    Ok(auth)
}
