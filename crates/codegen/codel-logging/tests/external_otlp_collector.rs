//! The external stream carries no first party credential: the real exporter, configured from
//! `exporter_env()` alone, sends no `authorization` header.

use std::time::Duration;

use codel_logging::enums::PermissionMode;
use codel_logging::events::SessionNew;
use codel_logging::external::ExternalOtelConfig;
use codel_logging::external::config::ExternalClientInfo;
use codel_test_support::MockOtelServer;

#[tokio::test]
async fn external_stream_carries_no_first_party_credential() {
    let server = MockOtelServer::start().await.unwrap();
    let env = server.exporter_env();
    let mut cfg = ExternalOtelConfig::resolve_with(|name| env.get(name).cloned(), None)
        .expect("exporter_env resolves to an active external stream");
    cfg.client = ExternalClientInfo {
        service_version: "0.0.0-test".into(),
        client_version: "0.0.0-test".into(),
        app_entrypoint: "cli".into(),
    };
    codel_logging::external::init(Some(cfg));
    assert!(codel_logging::external::is_active());

    codel_logging::log_event(SessionNew {
        session_id: "sess-collector-1".into(),
        client_identifier: None,
        client_version: None,
        is_git_repo: true,
        permission_mode: PermissionMode::Ask,
    });
    tokio::task::spawn_blocking(codel_logging::external::flush)
        .await
        .unwrap();
    server
        .recorder()
        .wait_for_events(Duration::from_secs(10), |events| !events.is_empty())
        .await
        .unwrap();
    tokio::task::spawn_blocking(codel_logging::external::shutdown)
        .await
        .unwrap();

    let exports = server.recorder().exports();
    assert_eq!(
        vec![None; exports.len()],
        exports
            .iter()
            .map(|export| export.header("authorization"))
            .collect::<Vec<_>>()
    );
}
