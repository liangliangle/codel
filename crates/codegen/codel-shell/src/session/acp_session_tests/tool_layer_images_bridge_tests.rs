//! Wiring tests for MCP tool-layer images through `handle_bridge_tool_success`.
use super::support::*;
use super::*;
use codel_sampling_types::{ContentPart, ConversationItem};
use codel_tools::types::output::{MCPOutput, ToolOutput, ToolRunResult};
use codel_tools::util::base64_images::{ExtractedImage, IMAGE_CONTENT_PLACEHOLDER};
fn mcp_screenshot_result(payload_b64: &str) -> ToolRunResult {
    let mut mcp = MCPOutput::okay_output(
        "browser_screenshot".into(),
        "browser-use".into(),
        IMAGE_CONTENT_PLACEHOLDER.into(),
    );
    mcp.extracted_images = vec![ExtractedImage {
        data: payload_b64.to_owned(),
        mime_type: "image/png".into(),
    }];
    ToolRunResult {
        output: ToolOutput::MCP(mcp),
        prompt_text: IMAGE_CONTENT_PLACEHOLDER.into(),
        effective_tool_name: None,
    }
}
fn tool_result_text(item: &ConversationItem) -> &str {
    match item {
        ConversationItem::ToolResult(tr) => tr.content.as_ref(),
        other => panic!("expected ToolResult, got {other:?}"),
    }
}
fn last_tool_result_text(conv: &[ConversationItem]) -> &str {
    let tool = conv
        .iter()
        .rev()
        .find(|item| matches!(item, ConversationItem::ToolResult(_)))
        .expect("tool result pushed");
    tool_result_text(tool)
}
fn followup_has_data_image(followups: &[ConversationItem]) -> bool {
    followups.iter().any(|item| match item {
        ConversationItem::User(u) => u
            .content
            .iter()
            .any(|p| matches!(p, ContentPart::Image { url } if url.starts_with("data:image/"))),
        _ => false,
    })
}
fn prepared_post_tool_use_call(id: &str, tool_name: &str) -> PreparedToolCall {
    PreparedToolCall {
        call_id: id.to_string(),
        tool_call_id: acp::ToolCallId::new(id),
        tool_name: tool_name.to_string(),
        raw_arguments: "{}".to_string(),
        mcp_file: None,
        parsed_args: serde_json::json!({}),
        model_id: Some("test-model".to_string()),
        invocation_id: "018f6b6c-7b3a-7c3a-8c3a-000000000001".to_string(),
        tool_id: "opaque".to_string(),
        tool_version: None,
        concatenated_json_count: 0,
        coercion_note: None,
        dispatch_target_name: None,
        is_read_only: false,
        rewriting_hook: None,
        additional_context: Vec::new(),
    }
}
fn mcp_text_result(marker: &str) -> ToolRunResult {
    ToolRunResult {
        output: ToolOutput::MCP(MCPOutput::okay_output(
            "search".into(),
            "memory".into(),
            marker.into(),
        )),
        prompt_text: marker.into(),
        effective_tool_name: None,
    }
}
fn bash_text_result(marker: &str) -> ToolRunResult {
    let output: ToolOutput = serde_json::from_value(serde_json::json!({
        "type": "Bash",
        "output": [],
        "output_for_prompt": marker,
        "exit_code": 0,
        "command": "ls",
        "truncated": false,
        "timed_out": false,
        "current_dir": "/tmp",
        "output_file": "",
        "total_bytes": 0,
    }))
    .expect("bash output should deserialize");
    ToolRunResult {
        output,
        prompt_text: marker.into(),
        effective_tool_name: None,
    }
}
#[tokio::test(flavor = "current_thread")]
async fn post_tool_use_replacement_reaches_model_original_stays_on_record() {
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let (gateway_tx, _gateway_rx) = tokio::sync::mpsc::unbounded_channel::<
                codel_acp_lib::AcpClientMessage,
            >();
            let (persistence_tx, _) = tokio::sync::mpsc::unbounded_channel::<
                PersistenceMsg,
            >();
            let (mut actor, mut event_rx) = create_test_actor_ex(
                    0,
                    256_000,
                    85,
                    gateway_tx,
                    persistence_tx,
                )
                .await;
            install_pre_tool_use_hooks(
                &mut actor,
                vec![post_tool_use_spec(
                    "redact",
                    None,
                    r#"echo '{"hookSpecificOutput":{"updatedMCPToolOutput":"[redacted by hook]"}}'"#,
                )],
            );
            let original_marker = "ORIGINAL-mcp-content-xyz";
            let drained = DrainedToolSuccess::new(mcp_text_result(original_marker));
            let prepared = prepared_post_tool_use_call("tc-redact", "search__memory");
            let (mut delivery, _deferred_scrollback) = actor
                .dispatch_post_tool_use_hook(&prepared, drained.output(), None)
                .await;
            let model_output_override = delivery.model_output.take();
            assert_eq!(
                model_output_override.as_deref(),
                Some("[redacted by hook]"),
                "the real dispatch+plan wiring must produce the replacement"
            );
            actor
                .handle_bridge_tool_success(BridgeToolSuccess {
                    tool_call_id: &acp::ToolCallId::new("tc-redact"),
                    call_id: "tc-redact",
                    requested_tool_name: "search__memory",
                    effective_tool_name: "search__memory",
                    drained,
                    concatenated_json_count: 0,
                    coercion_note: None,
                    model_id: "test-model",
                    tool_parsed_args: &serde_json::json!({}),
                    model_output_override,
                })
                .await
                .expect("bridge success");
            let conv = actor.chat_state_handle.get_conversation().await;
            let text = last_tool_result_text(&conv);
            assert!(
                text.contains("[redacted by hook]"),
                "the model reads the replacement: {text}"
            );
            assert!(
                !text.contains(original_marker),
                "the model must not see the original: {text}"
            );
            let mut acp_dump = String::new();
            while let Ok(event) = event_rx.try_recv() {
                if let SessionEvent::Notification(SessionNotification::Acp(n)) = event
                    && let Ok(v) = serde_json::to_value(&n.update)
                {
                    acp_dump.push_str(&v.to_string());
                }
            }
            assert!(
                acp_dump.contains(original_marker),
                "the ACP session/update must retain the original output: {acp_dump}"
            );
            assert!(
                !acp_dump.contains("[redacted by hook]"),
                "the replacement must not reach the ACP record: {acp_dump}"
            );
        })
        .await;
}
#[tokio::test(flavor = "current_thread")]
async fn post_tool_use_rejection_downgrades_only_the_producing_run() {
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let (gateway_tx, gateway_rx) = tokio::sync::mpsc::unbounded_channel::<
                codel_acp_lib::AcpClientMessage,
            >();
            let (persistence_tx, _) = tokio::sync::mpsc::unbounded_channel::<
                PersistenceMsg,
            >();
            let (_acp_updates, codel_updates) = spawn_capturing_gateway_loop(gateway_rx);
            let mut actor = create_test_actor(0, 256_000, 85, gateway_tx, persistence_tx)
                .await;
            install_pre_tool_use_hooks(
                &mut actor,
                vec![
                    post_tool_use_spec(
                        "redact",
                        None,
                        r#"echo '{"hookSpecificOutput":{"updatedToolOutput":{"not":"a tool output"}}}'"#,
                    ),
                    post_tool_use_spec(
                        "redact",
                        None,
                        r#"echo '{"hookSpecificOutput":{"additionalContext":"noted"}}'"#,
                    ),
                ],
            );
            let drained = DrainedToolSuccess::new(bash_text_result("original"));
            let prepared = prepared_post_tool_use_call(
                "tc-reject",
                "run_terminal_command",
            );
            let (_delivery, deferred_scrollback) = actor
                .dispatch_post_tool_use_hook(&prepared, drained.output(), None)
                .await;
            if let Some(scrollback) = deferred_scrollback {
                actor.emit_post_tool_use_scrollback(scrollback).await;
            }
            drain_gateway_turns().await;
            let updates = codel_updates.lock().unwrap();
            let mut failed = 0usize;
            let mut success = 0usize;
            for update in updates.iter() {
                let Some(runs) = update.get("runs").and_then(|r| r.as_array()) else {
                    continue;
                };
                for run in runs {
                    if run.get("name").and_then(serde_json::Value::as_str)
                        != Some("redact")
                    {
                        continue;
                    }
                    match run
                        .get("status")
                        .and_then(|s| s.get("status"))
                        .and_then(serde_json::Value::as_str)
                    {
                        Some("failed") => failed += 1,
                        Some("success") => success += 1,
                        other => panic!("unexpected run status {other:?} in {update:?}"),
                    }
                }
            }
            assert_eq!(
                (failed, success),
                (1, 1),
                "only the producing run is downgraded; the sibling same-named run stays success \
                 (failed={failed}, success={success}): {:?}",
                *updates
            );
        })
        .await;
}
