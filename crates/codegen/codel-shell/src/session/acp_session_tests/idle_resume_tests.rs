use super::support::*;
use super::*;
use tokio::sync::mpsc;
/// Test that `last_api_request_at` is recorded and used for idle detection.
///
/// `maybe_refresh_model_metadata_on_resume` checks this timestamp to decide whether to proactively refresh model metadata from cli-chat-proxy.
#[tokio::test(flavor = "current_thread")]
async fn test_last_api_request_at_idle_detection() {
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let (gateway_tx, _) = mpsc::unbounded_channel();
            let (persistence_tx, _) = mpsc::unbounded_channel();
            let actor = create_test_actor(50_000, 100_000, 85, gateway_tx, persistence_tx).await;
            let initial = actor
                .last_api_request_at
                .load(std::sync::atomic::Ordering::Relaxed);
            assert_eq!(initial, 0, "last_api_request_at should be 0 initially");
            actor.record_api_request_time();
            let recorded = actor
                .last_api_request_at
                .load(std::sync::atomic::Ordering::Relaxed);
            assert!(
                recorded > 0,
                "last_api_request_at should be set after recording"
            );
            let now_ms = chrono::Utc::now().timestamp_millis();
            let diff = (now_ms - recorded).abs();
            assert!(
                diff < 1000,
                "recorded timestamp should be within 1 second of now"
            );
            let idle_secs = (now_ms - recorded) / 1000;
            assert!(
                idle_secs < SessionActor::IDLE_REFRESH_THRESHOLD_SECS,
                "should be within idle threshold immediately after recording"
            );
        })
        .await;
}
/// Verify `maybe_refresh_model_metadata_on_resume` is a no-op when idle is under 10 minutes.
#[tokio::test(flavor = "current_thread")]
async fn test_idle_resume_noop_when_not_idle_enough() {
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let (gateway_tx, _) = mpsc::unbounded_channel::<codel_acp_lib::AcpClientMessage>();
            let (persistence_tx, _) = mpsc::unbounded_channel::<PersistenceMsg>();
            let actor = create_test_actor(50_000, 200_000, 85, gateway_tx, persistence_tx).await;
            let five_minutes_ago_ms = chrono::Utc::now().timestamp_millis() - (5 * 60 * 1000);
            actor
                .last_api_request_at
                .store(five_minutes_ago_ms, std::sync::atomic::Ordering::Relaxed);
            let cfg_before = actor.chat_state_handle.get_sampling_config().await.unwrap();
            actor.maybe_refresh_model_metadata_on_resume().await;
            let cfg_after = actor.chat_state_handle.get_sampling_config().await.unwrap();
            assert_eq!(
                cfg_before.context_window, cfg_after.context_window,
                "config should not change when idle < 10 min"
            );
        })
        .await;
}
