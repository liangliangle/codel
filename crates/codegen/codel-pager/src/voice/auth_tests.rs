use std::sync::Arc;

use chrono::{Duration, Utc};
use pretty_assertions::assert_eq;
use codel_login::{AuthManager, AuthMode, CodelAuth, CodelComConfig};
use codel_test_support::EnvGuard;
use codel_voice::VoiceAuthError;

use super::build_voice_auth;

fn session(issuer: &str) -> CodelAuth {
    CodelAuth {
        key: "session-token".to_owned(),
        auth_mode: AuthMode::Oidc,
        oidc_issuer: Some(issuer.to_owned()),
        refresh_token: Some("rt".to_owned()),
        expires_at: Some(Utc::now() + Duration::hours(1)),
        ..CodelAuth::test_default()
    }
}

/// The positive case is a static key, which every build of the manager serves; the Codel-session case is pinned in
/// `codel-login`.
#[tokio::test]
#[serial_test::serial]
async fn foreign_session_is_refused_and_codel_credential_is_served() {
    let _codel = EnvGuard::unset("CODEL_API_KEY");
    let _legacy = EnvGuard::unset("CODEL_CODE_CODEL_API_KEY");
    let _auth_path = EnvGuard::unset("CODEL_AUTH_PATH");
    let dir = tempfile::tempdir().unwrap();
    let mgr = Arc::new(AuthManager::new(dir.path(), CodelComConfig::default()));
    let auth = build_voice_auth(mgr.clone());

    mgr.hot_swap(session("https://cursor.com"));
    assert_eq!(Err(VoiceAuthError::ForeignSession), auth.bearer().await);

    mgr.set_process_static_api_key(Some("codel-static-key".to_owned()));
    assert_eq!(Ok("codel-static-key".to_owned()), auth.bearer().await);

    mgr.set_process_static_api_key(None);
    mgr.clear().unwrap();
    assert_eq!(Err(VoiceAuthError::NotSignedIn), auth.bearer().await);
}
