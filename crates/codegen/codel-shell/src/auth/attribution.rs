use std::sync::Arc;

use crate::auth::manager::AuthManager;

/// Kind of consumer that received a 401.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsumerKind {
    StorageClient,
    SessionRegistryClient,
    FeedbackClient,
    IdleResumeModelRefresh,
}

/// Record a consumer 401 for attribution/diagnostics.
pub fn record_consumer_401(
    _auth_manager: &AuthManager,
    _session_id: Option<&str>,
    _kind: ConsumerKind,
    _operation: &str,
    _sent_bearer_prefix: Option<&str>,
) {
    // Stub: no-op
}

/// Shell-level attribution callback for the sampler.
pub struct ShellAttribution {
    _auth_manager: Arc<AuthManager>,
    _session_id: Option<String>,
}

impl std::fmt::Debug for ShellAttribution {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ShellAttribution")
            .field("_session_id", &self._session_id)
            .finish_non_exhaustive()
    }
}

impl ShellAttribution {
    pub fn new(auth_manager: Arc<AuthManager>, session_id: Option<String>) -> Self {
        Self {
            _auth_manager: auth_manager,
            _session_id: session_id,
        }
    }

    /// Create a tool-level attribution callback.
    pub fn new_tool_callback(
        auth_manager: Arc<AuthManager>,
        session_id: Option<String>,
    ) -> codel_tools::SharedAttributionCallback {
        Arc::new(Self::new(auth_manager, session_id))
    }
}

impl codel_sampler::attribution::Auth401AttributionCallback for ShellAttribution {
    fn record_401(
        &self,
        _consumer: codel_sampler::attribution::SamplingConsumer,
        _sent_bearer_prefix: Option<&str>,
    ) {
        // Stub: no-op
    }
}

impl codel_tools::Auth401AttributionCallback for ShellAttribution {
    fn record_401(
        &self,
        _consumer: codel_tools::ToolConsumer,
        _sent_bearer_prefix: Option<&str>,
    ) {
        // Stub: no-op
    }
}

#[cfg(test)]
static TEST_EMIT_COUNT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

#[cfg(test)]
pub fn reset_test_emit_count() {
    TEST_EMIT_COUNT.store(0, std::sync::atomic::Ordering::SeqCst);
}

#[cfg(test)]
pub fn test_emit_count() -> u32 {
    TEST_EMIT_COUNT.load(std::sync::atomic::Ordering::SeqCst)
}
