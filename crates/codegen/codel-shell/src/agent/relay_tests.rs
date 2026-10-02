//! Tests for [`super`] (the codel.dev relay connection loop).
//! Extracted from `relay.rs` so the implementation reads top-to-bottom; wired in via `#[path = "relay_tests.rs"] mod tests;`.
use super::*;
use codel_login::AuthMode;
use serde_json::json;
use std::sync::atomic::{AtomicU32, Ordering};
use tokio_tungstenite::tungstenite::{Utf8Bytes, protocol::Role};
/// Create an in-memory WebSocket pair (no network, no handshake needed).
async fn ws_pair() -> (
    tokio_tungstenite::WebSocketStream<tokio::io::DuplexStream>,
    tokio_tungstenite::WebSocketStream<tokio::io::DuplexStream>,
) {
    let (client, server) = tokio::io::duplex(64 * 1024);
    let client_ws =
        tokio_tungstenite::WebSocketStream::from_raw_socket(client, Role::Client, None).await;
    let server_ws =
        tokio_tungstenite::WebSocketStream::from_raw_socket(server, Role::Server, None).await;
    (client_ws, server_ws)
}
#[test]
fn test_handshake_401_detected_through_anyhow_context() {
    use tokio_tungstenite::tungstenite::Error as WsError;
    let resp = axum::http::Response::builder()
        .status(401)
        .body(None::<Vec<u8>>)
        .unwrap();
    let err =
        anyhow::Error::from(WsError::Http(Box::new(resp))).context("WebSocket connection failed");
    assert!(is_handshake_unauthorized(&err));
}
#[test]
fn test_handshake_non_401_and_non_ws_errors_rejected() {
    use tokio_tungstenite::tungstenite::Error as WsError;
    let resp = axum::http::Response::builder()
        .status(403)
        .body(None::<Vec<u8>>)
        .unwrap();
    let err =
        anyhow::Error::from(WsError::Http(Box::new(resp))).context("WebSocket connection failed");
    assert!(!is_handshake_unauthorized(&err));
    let err = anyhow::anyhow!("some random error");
    assert!(!is_handshake_unauthorized(&err));
}
#[tokio::test]
async fn test_ws_session_auth_error_returns_auth_error() {
    let (client_ws, server_ws) = ws_pair().await;
    let (mut server_tx, _server_rx) = server_ws.split();
    let (to_agent_tx, _to_agent_rx) = mpsc::unbounded_channel::<String>();
    let (_agent_out_tx, mut agent_out_rx) = mpsc::unbounded_channel::<String>();
    let cancel = CancellationToken::new();
    tokio::spawn(async move {
        let auth_error = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "error": { "code": -32000, "message": "Authentication required" }
        });
        let _ = server_tx
            .send(Message::Text(Utf8Bytes::from(auth_error.to_string())))
            .await;
        let _ = server_tx.close().await;
    });
    let result = tokio::time::timeout(
        Duration::from_secs(5),
        run_websocket_session(client_ws, &to_agent_tx, &mut agent_out_rx, &cancel),
    )
    .await
    .expect("test timed out")
    .expect("session should not error");
    assert_eq!(result, SessionEndReason::AuthError);
}
/// The writer can win the reader/writer select while the `-32000` frame is still unread (here: the agent outbound
/// channel is already closed, so the writer exits at once). The session must still be classified as an auth error,
/// not a normal close, or the reconnect loop would reset its backoff. Repeated because `select!` picks a random order.
#[tokio::test]
async fn test_ws_session_writer_exit_does_not_mask_pending_auth_error() {
    for _ in 0..20 {
        let (client_ws, server_ws) = ws_pair().await;
        let (mut server_tx, _server_rx) = server_ws.split();
        let auth_error = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "error": { "code": AUTH_ERROR_CODE, "message": "Authentication required" }
        });
        server_tx
            .send(Message::Text(Utf8Bytes::from(auth_error.to_string())))
            .await
            .unwrap();
        let (to_agent_tx, _to_agent_rx) = mpsc::unbounded_channel::<String>();
        let (agent_out_tx, mut agent_out_rx) = mpsc::unbounded_channel::<String>();
        drop(agent_out_tx);
        let cancel = CancellationToken::new();
        let result = tokio::time::timeout(
            Duration::from_secs(5),
            run_websocket_session(client_ws, &to_agent_tx, &mut agent_out_rx, &cancel),
        )
        .await
        .expect("test timed out")
        .expect("session should not error");
        assert_eq!(result, SessionEndReason::AuthError);
    }
}
#[tokio::test]
async fn test_ws_session_non_auth_error_skipped() {
    let (client_ws, server_ws) = ws_pair().await;
    let (mut server_tx, _server_rx) = server_ws.split();
    let (to_agent_tx, _to_agent_rx) = mpsc::unbounded_channel::<String>();
    let (_agent_out_tx, mut agent_out_rx) = mpsc::unbounded_channel::<String>();
    let cancel = CancellationToken::new();
    tokio::spawn(async move {
        let other_error = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "error": { "code": -32600, "message": "Invalid Request" }
        });
        let _ = server_tx
            .send(Message::Text(Utf8Bytes::from(other_error.to_string())))
            .await;
        tokio::time::sleep(Duration::from_millis(50)).await;
        let _ = server_tx.close().await;
    });
    let result = tokio::time::timeout(
        Duration::from_secs(5),
        run_websocket_session(client_ws, &to_agent_tx, &mut agent_out_rx, &cancel),
    )
    .await
    .expect("test timed out")
    .expect("session should not error");
    assert_eq!(
        result,
        SessionEndReason::Normal {
            authenticated: false
        }
    );
}
#[tokio::test]
async fn test_ws_session_normal_close_returns_normal() {
    let (client_ws, server_ws) = ws_pair().await;
    let (mut server_tx, _server_rx) = server_ws.split();
    let (to_agent_tx, _to_agent_rx) = mpsc::unbounded_channel::<String>();
    let (_agent_out_tx, mut agent_out_rx) = mpsc::unbounded_channel::<String>();
    let cancel = CancellationToken::new();
    tokio::spawn(async move {
        let _ = server_tx.send(Message::Close(None)).await;
    });
    let result = tokio::time::timeout(
        Duration::from_secs(5),
        run_websocket_session(client_ws, &to_agent_tx, &mut agent_out_rx, &cancel),
    )
    .await
    .expect("test timed out")
    .expect("session should not error");
    assert_eq!(
        result,
        SessionEndReason::Normal {
            authenticated: false
        }
    );
}
#[tokio::test]
async fn test_ws_session_read_liveness_timeout_ends_session() {
    let (client_ws, server_ws) = ws_pair().await;
    let _silent_server = server_ws;
    let (to_agent_tx, _to_agent_rx) = mpsc::unbounded_channel::<String>();
    let (_agent_out_tx, mut agent_out_rx) = mpsc::unbounded_channel::<String>();
    let cancel = CancellationToken::new();
    let result = tokio::time::timeout(
        Duration::from_secs(5),
        run_websocket_session_with_liveness(
            client_ws,
            &to_agent_tx,
            &mut agent_out_rx,
            &cancel,
            Duration::from_millis(100),
        ),
    )
    .await
    .expect("session must end via read-liveness timeout instead of hanging")
    .expect("session should not error");
    assert_eq!(
        result,
        SessionEndReason::Normal {
            authenticated: false
        }
    );
}
#[tokio::test]
async fn test_ws_session_inbound_traffic_resets_liveness_window() {
    let (client_ws, server_ws) = ws_pair().await;
    let (mut server_tx, _server_rx) = server_ws.split();
    let (to_agent_tx, mut to_agent_rx) = mpsc::unbounded_channel::<String>();
    let (_agent_out_tx, mut agent_out_rx) = mpsc::unbounded_channel::<String>();
    let cancel = CancellationToken::new();
    tokio::spawn(async move {
        for i in 0..12 {
            let msg = json!({ "jsonrpc": "2.0", "method": "ping", "id": i });
            if server_tx
                .send(Message::Text(Utf8Bytes::from(msg.to_string())))
                .await
                .is_err()
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        let _ = server_tx.close().await;
    });
    let result = tokio::time::timeout(
        Duration::from_secs(5),
        run_websocket_session_with_liveness(
            client_ws,
            &to_agent_tx,
            &mut agent_out_rx,
            &cancel,
            Duration::from_millis(200),
        ),
    )
    .await
    .expect("test timed out")
    .expect("session should not error");
    assert_eq!(
        result,
        SessionEndReason::Normal {
            authenticated: true
        }
    );
    let mut forwarded = 0;
    while to_agent_rx.try_recv().is_ok() {
        forwarded += 1;
    }
    assert_eq!(forwarded, 12);
}
#[tokio::test]
async fn test_ws_session_forwards_text_to_agent() {
    let (client_ws, server_ws) = ws_pair().await;
    let (mut server_tx, _server_rx) = server_ws.split();
    let (to_agent_tx, mut to_agent_rx) = mpsc::unbounded_channel::<String>();
    let (_agent_out_tx, mut agent_out_rx) = mpsc::unbounded_channel::<String>();
    let cancel = CancellationToken::new();
    let test_msg = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {}
    });
    let msg_str = test_msg.to_string();
    tokio::spawn(async move {
        let _ = server_tx
            .send(Message::Text(Utf8Bytes::from(msg_str)))
            .await;
        tokio::time::sleep(Duration::from_millis(50)).await;
        let _ = server_tx.close().await;
    });
    let _result = tokio::time::timeout(
        Duration::from_secs(5),
        run_websocket_session(client_ws, &to_agent_tx, &mut agent_out_rx, &cancel),
    )
    .await
    .expect("test timed out");
    let received = to_agent_rx
        .try_recv()
        .expect("should have forwarded message to agent");
    let received_json: serde_json::Value = serde_json::from_str(&received).unwrap();
    assert_eq!(
        received_json.get("method").and_then(|v| v.as_str()),
        Some("initialize")
    );
}
#[tokio::test]
async fn test_ws_session_cancel_stops_session() {
    let (client_ws, server_ws) = ws_pair().await;
    let _server_ws = server_ws;
    let (to_agent_tx, _to_agent_rx) = mpsc::unbounded_channel::<String>();
    let (_agent_out_tx, mut agent_out_rx) = mpsc::unbounded_channel::<String>();
    let cancel = CancellationToken::new();
    let cancel_clone = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(100)).await;
        cancel_clone.cancel();
    });
    let result = tokio::time::timeout(
        Duration::from_secs(5),
        run_websocket_session(client_ws, &to_agent_tx, &mut agent_out_rx, &cancel),
    )
    .await
    .expect("test timed out")
    .expect("session should not error");
    assert_eq!(
        result,
        SessionEndReason::Normal {
            authenticated: false
        }
    );
}
/// Helper to create a test CodelAuth with the given key.
fn test_auth(key: &str) -> CodelAuth {
    CodelAuth {
        key: key.to_string(),
        refresh_token: Some("rt".to_string()),
        ..CodelAuth::test_default()
    }
}
/// Helper: write a CodelAuth to disk under the given scope.
fn write_test_auth_to_disk(dir: &std::path::Path, scope: &str, auth: &CodelAuth) {
    let path = dir.join("auth.json");
    let mut map = codel_login::read_auth_json(&path).unwrap_or_default();
    map.insert(scope.to_owned(), auth.clone());
    let json = serde_json::to_string_pretty(&map).unwrap();
    std::fs::write(&path, json).unwrap();
}
#[tokio::test]
async fn test_auth_refresh_via_auth_manager_on_auth_error() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let connection_count = Arc::new(AtomicU32::new(0));
    let count_clone = connection_count.clone();
    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                break;
            };
            let count = count_clone.clone();
            tokio::spawn(async move {
                let Ok(ws) = tokio_tungstenite::accept_async(stream).await else {
                    return;
                };
                let (mut tx, _rx) = ws.split();
                let n = count.fetch_add(1, Ordering::SeqCst);
                if n == 0 {
                    let auth_err = json!({
                        "jsonrpc": "2.0",
                        "id": 1,
                        "error": { "code": -32000, "message": "Token expired" }
                    });
                    let _ = tx
                        .send(Message::Text(Utf8Bytes::from(auth_err.to_string())))
                        .await;
                } else {
                    tokio::time::sleep(Duration::from_secs(30)).await;
                }
            });
        }
    });
    let dir = tempfile::tempdir().unwrap();
    let cfg = codel_login::CodelComConfig::default();
    let scope = cfg.auth_scope();
    let am = Arc::new(AuthManager::new(dir.path(), cfg));
    am.hot_swap(test_auth("old-key"));
    write_test_auth_to_disk(dir.path(), &scope, &test_auth("new-key"));
    let config = RelayConfig {
        ws_url: format!("ws://{}", addr),
        ws_origin: format!("http://{}", addr),
        token_header: "test-token".to_string(),
        auth: test_auth("old-key"),
        auth_manager: Some(am),
    };
    let cancel = CancellationToken::new();
    let (from_relay_tx, _from_relay_rx) = mpsc::unbounded_channel();
    let (_to_relay_tx, _handle) = spawn_relay_connection(config, from_relay_tx, cancel.clone());
    tokio::time::sleep(Duration::from_secs(3)).await;
    cancel.cancel();
    assert!(
        connection_count.load(Ordering::SeqCst) >= 2,
        "should have connected at least twice (original + after refresh), got {}",
        connection_count.load(Ordering::SeqCst)
    );
}
#[tokio::test]
async fn test_auth_refresh_failure_continues_with_backoff() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let connection_count = Arc::new(AtomicU32::new(0));
    let count_clone = connection_count.clone();
    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                break;
            };
            let count = count_clone.clone();
            tokio::spawn(async move {
                let Ok(ws) = tokio_tungstenite::accept_async(stream).await else {
                    return;
                };
                let (mut tx, _rx) = ws.split();
                count.fetch_add(1, Ordering::SeqCst);
                let auth_err = json!({
                    "jsonrpc": "2.0",
                    "id": 1,
                    "error": { "code": -32000, "message": "Token expired" }
                });
                let _ = tx
                    .send(Message::Text(Utf8Bytes::from(auth_err.to_string())))
                    .await;
            });
        }
    });
    let dir = tempfile::tempdir().unwrap();
    let cfg = codel_login::CodelComConfig::default();
    let scope = cfg.auth_scope();
    let am = Arc::new(AuthManager::new(dir.path(), cfg));
    am.hot_swap(test_auth("old-key"));
    write_test_auth_to_disk(dir.path(), &scope, &test_auth("old-key"));
    let config = RelayConfig {
        ws_url: format!("ws://{}", addr),
        ws_origin: format!("http://{}", addr),
        token_header: "test-token".to_string(),
        auth: test_auth("old-key"),
        auth_manager: Some(am),
    };
    let cancel = CancellationToken::new();
    let (from_relay_tx, _from_relay_rx) = mpsc::unbounded_channel();
    let (_to_relay_tx, _handle) = spawn_relay_connection(config, from_relay_tx, cancel.clone());
    tokio::time::sleep(Duration::from_secs(4)).await;
    cancel.cancel();
    assert!(
        connection_count.load(Ordering::SeqCst) >= 2,
        "should have retried after failed refresh, got {}",
        connection_count.load(Ordering::SeqCst)
    );
}
#[test]
fn relay_initialize_gains_user_message_echo_capability() {
    let mut frame = serde_json::json!({
        "jsonrpc": "2.0", "id": 7, "method": "initialize",
        "params": {
            "protocolVersion": 1,
            "clientCapabilities": { "_meta": { "codel/fs_notify": true } }
        }
    });
    assert!(declare_relay_client_capabilities(&mut frame));
    let meta = frame
        .pointer("/params/clientCapabilities/_meta")
        .expect("_meta present");
    assert_eq!(
        meta.get("codel/userMessageEcho"),
        Some(&serde_json::json!(true))
    );
    assert_eq!(meta.get("codel/fs_notify"), Some(&serde_json::json!(true)));
    assert_eq!(frame.get("id"), Some(&serde_json::json!(7)));
}
#[test]
fn relay_initialize_without_capabilities_block_gets_one() {
    let mut frame = serde_json::json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": { "protocolVersion": 1 }
    });
    assert!(declare_relay_client_capabilities(&mut frame));
    assert_eq!(
        frame.pointer("/params/clientCapabilities/_meta/codel.dev~1userMessageEcho"),
        Some(&serde_json::json!(true))
    );
}
#[test]
fn relay_explicit_user_message_echo_is_respected() {
    let mut frame = serde_json::json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": { "clientCapabilities": { "_meta": { "codel/userMessageEcho": false } } }
    });
    assert!(!declare_relay_client_capabilities(&mut frame));
    assert_eq!(
        frame.pointer("/params/clientCapabilities/_meta/codel.dev~1userMessageEcho"),
        Some(&serde_json::json!(false))
    );
}
#[test]
fn non_initialize_frames_are_left_alone() {
    for frame in [
        serde_json::json!({ "jsonrpc": "2.0", "id": 2, "method": "session/new", "params": { "cwd": "/w", "_meta": {} } }),
        serde_json::json!({ "jsonrpc": "2.0", "id": 3, "result": { "ok": true } }),
        serde_json::json!({ "jsonrpc": "2.0", "method": "session/update", "params": { "sessionId": "s" } }),
    ] {
        let original = frame.clone();
        let mut frame = frame;
        assert!(!declare_relay_client_capabilities(&mut frame));
        assert_eq!(frame, original);
    }
}
