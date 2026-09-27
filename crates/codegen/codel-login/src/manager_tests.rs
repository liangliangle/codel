//! Unit tests for [`super::manager::AuthManager`].
//! Extracted from `manager.rs` so the implementation reads top-to-bottom; wired in via `#[path = "manager_tests.rs"] mod tests;` in manager.rs.
use super::*;
use crate::error::RefreshTokenError;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Instant;
/// A token inside the 5-minute early-invalidation buffer must be invisible to `current()` (returns None) but visible to `expired_auth()`.
/// That lets callers attempt a silent refresh.
#[test]
fn near_expiry_token_invisible_to_current_visible_to_expired_auth() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = CodelComConfig::default();
    let mgr = Arc::new(AuthManager::new(dir.path(), cfg));
    let near_expiry = CodelAuth {
        key: "near-expiry-key".into(),
        user_id: "user-1".into(),
        email: Some("user@test.com".into()),
        refresh_token: Some("rt-valid".into()),
        expires_at: Some(Utc::now() + Duration::minutes(3)),
        oidc_issuer: Some("https://idp.example.com".into()),
        oidc_client_id: Some("client-1".into()),
        ..CodelAuth::test_default()
    };
    mgr.hot_swap(near_expiry);
    assert!(
        mgr.current().is_none(),
        "current() should return None for token within 5-min buffer"
    );
    assert!(
        mgr.is_expired(),
        "is_expired() should be true for token within 5-min buffer"
    );
    let expired = mgr.expired_auth();
    assert!(
        expired.is_some(),
        "expired_auth() should return the near-expiry token"
    );
    assert_eq!(expired.as_ref().unwrap().key, "near-expiry-key");
    assert_eq!(
        expired.as_ref().unwrap().refresh_token.as_deref(),
        Some("rt-valid"),
        "refresh_token must be preserved for silent refresh"
    );
}
/// Regression: when auth.json contains corrupt JSON, update() must not clobber the file with a single-entry map.
/// Instead it should update in-memory only and leave the file untouched.
#[tokio::test]
async fn update_recovers_from_corrupt_auth_json_by_backing_up_old_file() {
    let dir = tempfile::tempdir().unwrap();
    let auth_path = dir.path().join("auth.json");
    let cfg = CodelComConfig::default();
    let mgr = Arc::new(AuthManager::new(dir.path(), cfg.clone()));
    let bad_content = b"NOT VALID JSON {{{";
    std::fs::write(&auth_path, bad_content).unwrap();
    let new_auth = CodelAuth {
        key: "fresh-token".into(),
        user_id: "fresh-user".into(),
        expires_at: Some(Utc::now() + Duration::hours(1)),
        ..CodelAuth::test_default()
    };
    let result = mgr.update(new_auth).await;
    assert!(
        result.is_ok(),
        "update must succeed and persist after corrupt recovery: {result:?}"
    );
    let current = mgr.current();
    assert_eq!(
        current.as_ref().map(|a| a.key.as_str()),
        Some("fresh-token")
    );
    let on_disk_raw = std::fs::read_to_string(&auth_path).unwrap();
    assert!(
        on_disk_raw.contains("fresh-token"),
        "auth.json must contain the new credential after recovery, got: {on_disk_raw}"
    );
    let on_disk: AuthStore =
        serde_json::from_str(&on_disk_raw).expect("auth.json must be valid JSON after recovery");
    assert!(on_disk.contains_key(&cfg.auth_scope()));
    let mut backup_found = None;
    for entry in std::fs::read_dir(dir.path()).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with("auth.json.corrupt.") {
            backup_found = Some(entry.path());
            break;
        }
    }
    let backup_path = backup_found.expect("a .corrupt.* backup file must have been created");
    let backup_content = std::fs::read_to_string(&backup_path).unwrap();
    assert!(
        backup_content.contains("NOT VALID JSON"),
        "backup must contain the original corrupt content, got: {backup_content}"
    );
}
