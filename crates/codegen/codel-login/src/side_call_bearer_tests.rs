use std::sync::Arc;

use base64::Engine as _;
use chrono::{Duration, Utc};
use pretty_assertions::assert_eq;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};
use codel_test_support::EnvGuard;
use codel_tools::implementations::codel_build::image_gen::{ImageGenClient, ImageGenConfig};
use codel_tools::implementations::codel_build::media_bearer::SIDE_CALL_BEARER_ERROR_CODE;
use codel_tools::types::api_key_provider::SideCallBearerError;

use super::{SharedAuthKeyProvider, is_codel_side_call_principal};
use crate::config::{CodelComConfig, CODEL_OAUTH2_ISSUER};
use crate::{AuthManager, AuthMode, CodelAuth};

const FOREIGN_ISSUER: &str = "https://cursor.com";

fn unsigned_jwt(header: &str, payload: &str) -> String {
    let enc = base64::engine::general_purpose::URL_SAFE_NO_PAD;
    format!("{}.{}.sig", enc.encode(header), enc.encode(payload))
}

fn foreign_access_token() -> String {
    unsigned_jwt(
        r#"{"alg":"RS256","typ":"at+jwt"}"#,
        &format!(r#"{{"iss":"{FOREIGN_ISSUER}","sub":"user-1"}}"#),
    )
}

fn session(key: &str, mode: AuthMode, issuer: Option<&str>) -> CodelAuth {
    CodelAuth {
        key: key.to_owned(),
        auth_mode: mode,
        oidc_issuer: issuer.map(str::to_owned),
        refresh_token: Some("rt".to_owned()),
        expires_at: Some(Utc::now() + Duration::hours(1)),
        ..CodelAuth::test_default()
    }
}

fn api_key(key: &str) -> CodelAuth {
    CodelAuth {
        key: key.to_owned(),
        auth_mode: AuthMode::ApiKey,
        oidc_issuer: None,
        expires_at: None,
        ..CodelAuth::test_default()
    }
}

/// The process env may carry a developer's key; every test pins it out so the credential alone decides.
fn static_key_guards() -> [EnvGuard; 3] {
    [
        EnvGuard::unset("CODEL_API_KEY"),
        EnvGuard::unset("CODEL_CODE_CODEL_API_KEY"),
        EnvGuard::unset("CODEL_AUTH_PATH"),
    ]
}

fn manager(dir: &tempfile::TempDir, auth: Option<CodelAuth>) -> Arc<AuthManager> {
    let mgr = Arc::new(AuthManager::new(dir.path(), CodelComConfig::default()));
    if let Some(auth) = auth {
        mgr.hot_swap(auth);
    }
    mgr
}

/// Only a foreign-issuer credential is refused. Everything chat sends today keeps going to the
/// server, which decides: an enterprise IdP session, a bare external-provider token, a legacy web login.
#[test]
fn only_a_foreign_credential_is_refused() {
    assert!(is_codel_side_call_principal(&api_key("codel-plain-key")));
    assert!(is_codel_side_call_principal(&session(
        "opaque",
        AuthMode::Oidc,
        Some(CODEL_OAUTH2_ISSUER)
    )));
    assert!(is_codel_side_call_principal(&session(
        "opaque",
        AuthMode::Oidc,
        Some("https://idp.acme.example")
    )));
    assert!(is_codel_side_call_principal(&session(
        "bare-provider-token",
        AuthMode::External,
        None
    )));
    assert!(is_codel_side_call_principal(&session(
        "legacy",
        AuthMode::WebLogin,
        None
    )));

    assert!(!is_codel_side_call_principal(&session(
        &foreign_access_token(),
        AuthMode::Oidc,
        Some(FOREIGN_ISSUER)
    )));
    assert!(!is_codel_side_call_principal(&session(
        &foreign_access_token(),
        AuthMode::Oidc,
        Some("https://api.cursor.com")
    )));
    assert!(is_codel_side_call_principal(&session(
        "opaque",
        AuthMode::Oidc,
        Some("https://cursor.com.evil.test")
    )));
}

#[test]
#[serial_test::serial]
fn foreign_session_is_an_error_not_a_bearer() {
    let _guards = static_key_guards();
    let dir = tempfile::tempdir().unwrap();
    let mgr = manager(
        &dir,
        Some(session(
            &foreign_access_token(),
            AuthMode::Oidc,
            Some(FOREIGN_ISSUER),
        )),
    );

    assert_eq!(
        Err(SideCallBearerError::ForeignSession),
        mgr.side_call_bearer()
    );
}

#[tokio::test]
#[serial_test::serial]
async fn foreign_session_is_an_error_after_the_refresh_path_too() {
    let _guards = static_key_guards();
    let dir = tempfile::tempdir().unwrap();
    let mgr = manager(
        &dir,
        Some(session(
            &foreign_access_token(),
            AuthMode::Oidc,
            Some(FOREIGN_ISSUER),
        )),
    );

    assert_eq!(
        Err(SideCallBearerError::ForeignSession),
        mgr.side_call_bearer_async().await
    );
}

#[test]
#[serial_test::serial]
fn no_credential_is_missing_not_a_fallback() {
    let _guards = static_key_guards();
    let dir = tempfile::tempdir().unwrap();
    let mgr = manager(&dir, None);

    assert_eq!(Err(SideCallBearerError::Missing), mgr.side_call_bearer());
}

#[test]
#[serial_test::serial]
fn static_key_still_serves_a_foreign_login() {
    let _guards = static_key_guards();
    let dir = tempfile::tempdir().unwrap();
    let mgr = manager(
        &dir,
        Some(session(
            &foreign_access_token(),
            AuthMode::Oidc,
            Some(FOREIGN_ISSUER),
        )),
    );
    mgr.set_process_static_api_key(Some("codel-static-key".to_owned()));

    assert_eq!(Ok("codel-static-key".to_owned()), mgr.side_call_bearer());
}

#[test]
#[serial_test::serial]
fn kill_switch_blocks_the_static_key_for_side_calls_too() {
    let _guards = static_key_guards();
    let dir = tempfile::tempdir().unwrap();
    let mgr = Arc::new(AuthManager::new(
        dir.path(),
        CodelComConfig {
            disable_api_key_auth: Some(true),
            ..CodelComConfig::default()
        },
    ));
    mgr.hot_swap(session(
        &foreign_access_token(),
        AuthMode::Oidc,
        Some(FOREIGN_ISSUER),
    ));
    mgr.set_process_static_api_key(Some("codel-static-key".to_owned()));

    assert_eq!(
        Err(SideCallBearerError::ForeignSession),
        mgr.side_call_bearer()
    );
}

#[test]
#[serial_test::serial]
fn codel_principals_are_returned() {
    let _guards = static_key_guards();
    let dir = tempfile::tempdir().unwrap();

    let mgr = manager(&dir, Some(api_key("codel-plain-key")));
    assert_eq!(Ok("codel-plain-key".to_owned()), mgr.side_call_bearer());

    mgr.hot_swap(session(
        "codel-session",
        AuthMode::Oidc,
        Some(CODEL_OAUTH2_ISSUER),
    ));
    assert_eq!(Ok("codel-session".to_owned()), mgr.side_call_bearer());
}

#[test]
#[serial_test::serial]
fn an_expired_codel_login_is_missing_in_the_sync_path() {
    let _guards = static_key_guards();
    let dir = tempfile::tempdir().unwrap();
    let mgr = manager(
        &dir,
        Some(CodelAuth {
            expires_at: Some(Utc::now() - Duration::hours(1)),
            ..session("codel-session", AuthMode::Oidc, Some(CODEL_OAUTH2_ISSUER))
        }),
    );

    assert_eq!(Err(SideCallBearerError::Missing), mgr.side_call_bearer());
}

fn image_client(base_url: &str, mgr: &Arc<AuthManager>) -> ImageGenClient {
    let config = ImageGenConfig::Enabled {
        api_key: None,
        base_url: base_url.to_owned(),
        extra_headers: indexmap::IndexMap::new(),
        image_gen_enabled: true,
        image_edit_enabled: true,
        model_override: None,
        edit_model_override: None,
        tier_restricted: false,
    };
    ImageGenClient::new(&config, Some(Arc::new(SharedAuthKeyProvider(mgr.clone())))).unwrap()
}

fn image_response() -> ResponseTemplate {
    let b64 = base64::engine::general_purpose::STANDARD.encode(b"jpeg-bytes");
    ResponseTemplate::new(200).set_body_json(serde_json::json!({ "data": [{ "b64_json": b64 }] }))
}

#[tokio::test]
#[serial_test::serial]
async fn imagine_request_carries_only_an_codel_bearer() {
    let _guards = static_key_guards();
    let dir = tempfile::tempdir().unwrap();
    let server = MockServer::start().await;
    let mgr = manager(
        &dir,
        Some(session(
            &foreign_access_token(),
            AuthMode::Oidc,
            Some(FOREIGN_ISSUER),
        )),
    );
    let client = image_client(&server.uri(), &mgr);

    let error = client
        .generate("a cat", "auto")
        .await
        .expect_err("a foreign session must not reach the imagine api");
    assert_eq!(
        Some(SIDE_CALL_BEARER_ERROR_CODE),
        error
            .details
            .as_ref()
            .and_then(|d| d.get("code"))
            .and_then(serde_json::Value::as_str)
    );
    assert!(
        server.received_requests().await.unwrap().is_empty(),
        "no HTTP request may leave on a foreign session"
    );

    Mock::given(method("POST"))
        .and(path("/images/generations"))
        .and(header("authorization", "Bearer codel-plain-key"))
        .respond_with(image_response())
        .expect(1)
        .mount(&server)
        .await;
    mgr.hot_swap(api_key("codel-plain-key"));
    assert_eq!(
        b"jpeg-bytes".to_vec(),
        client.generate("a cat", "auto").await.unwrap()
    );
    server.reset().await;

    Mock::given(method("POST"))
        .and(path("/images/generations"))
        .and(header("authorization", "Bearer codel-session"))
        .respond_with(image_response())
        .expect(1)
        .mount(&server)
        .await;
    mgr.hot_swap(session(
        "codel-session",
        AuthMode::Oidc,
        Some(CODEL_OAUTH2_ISSUER),
    ));
    assert_eq!(
        b"jpeg-bytes".to_vec(),
        client.generate("a cat", "auto").await.unwrap()
    );
}
