//! Ambient session context for the local logs.
//! The task-local [`TelemetryCtx`] carries the `session_id` a span stamps so the
//! debug-log firehose can route a session's records to its own file.

use std::sync::Arc;

/// The per-session context the local logs describe.
#[derive(Clone)]
pub struct TelemetryCtx {
    pub session_id: String,
    /// Prompt counter the session runtime advances; kept so callers share one handle.
    pub prompt_index: Arc<tokio::sync::Mutex<usize>>,
}

impl TelemetryCtx {
    pub fn new(session_id: String, prompt_index: Arc<tokio::sync::Mutex<usize>>) -> Self {
        Self {
            session_id,
            prompt_index,
        }
    }
}

tokio::task_local! {
    static TELEMETRY_CTX: Arc<TelemetryCtx>;
}

/// The `session_id` field name the debug-log firehose router keys on.
/// `debug_log::SessionIdVisitor` stashes a `SessionId` extension on any span carrying this field; the span *name* plays no part in routing.
/// Shared so the `info_span!` here and the router in `debug_log` can't silently drift; a rename trips `session_span_exposes_router_field` below.
pub(crate) const SESSION_ID_FIELD: &str = "session_id";

/// Build the per-session tracing span the firehose router routes by.
/// The field name MUST be the literal `session_id` (tracing field names can't be a const); the test below pins it against [`SESSION_ID_FIELD`].
fn session_span(session_id: &str) -> tracing::Span {
    tracing::info_span!("session", session_id = %session_id)
}

/// Run `fut` with the session context active. Also sets a `tracing` span.
pub async fn with_session_ctx<F: std::future::Future>(ctx: TelemetryCtx, fut: F) -> F::Output {
    use tracing::Instrument;
    let span = session_span(&ctx.session_id);
    TELEMETRY_CTX
        .scope(Arc::new(ctx), fut.instrument(span))
        .await
}

/// Clone the ambient ctx; `None` in a `spawn_local` child, whose task-locals are not inherited, so call this on the parent.
fn clone_current() -> Option<TelemetryCtx> {
    TELEMETRY_CTX.try_with(|c| (**c).clone()).ok()
}

/// `spawn_local` `fut` with the caller's [`TelemetryCtx`] re-entered in the child, so turn work keeps its `session_id` span field.
pub fn spawn_local_in_session_ctx<F>(fut: F) -> tokio::task::JoinHandle<F::Output>
where
    F: std::future::Future + 'static,
    F::Output: 'static,
{
    let ctx = clone_current();
    tokio::task::spawn_local(async move {
        match ctx {
            Some(ctx) => with_session_ctx(ctx, fut).await,
            None => fut.await,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The debug-log firehose router (`debug_log`) finds the session span by its `session_id` field (not by name).
    /// That field name is a literal in `session_span` (tracing field names can't be a const), so pin it against the shared const here.
    /// A rename of either breaks this test instead of silently degrading routing to the per-pid fallback.
    #[test]
    fn session_span_exposes_router_field() {
        // A bare registry enables every callsite, so the span has live metadata.
        let subscriber = tracing_subscriber::registry();
        tracing::subscriber::with_default(subscriber, || {
            let span = session_span("test-id");
            let meta = span
                .metadata()
                .expect("session span must have metadata under an enabling subscriber");
            assert!(
                meta.fields().field(SESSION_ID_FIELD).is_some(),
                "session span must expose `{SESSION_ID_FIELD}` for debug-log routing",
            );
        });
    }
}
