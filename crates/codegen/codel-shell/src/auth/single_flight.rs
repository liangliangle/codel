//! Stub for the removed interactive auth single-flight mechanism.
//! API-key-only auth has no interactive login flow, so all methods are no-ops.

use tokio::sync::Mutex;

/// Error from submitting a device code (stub).
#[derive(Debug)]
pub enum SubmitCodeError {
    SendFailed(String),
    NoPendingAttempt,
}

/// Channels for an interactive auth attempt (stub).
pub struct AttemptChannels {
    _code_tx: tokio::sync::mpsc::Sender<String>,
    _url_rx: tokio::sync::oneshot::Receiver<String>,
}

impl AttemptChannels {
    pub fn new(
        code_tx: tokio::sync::mpsc::Sender<String>,
        url_rx: tokio::sync::oneshot::Receiver<String>,
    ) -> Self {
        Self {
            _code_tx: code_tx,
            _url_rx: url_rx,
        }
    }
}

/// Stub single-flight guard for interactive auth. All operations are no-ops
/// since API-key auth has no interactive login flow.
#[derive(Default)]
pub struct AuthSingleFlight {
    _inner: Mutex<()>,
}

impl AuthSingleFlight {
    /// Cancel any in-flight login (no-op).
    pub fn cancel(&self) {}

    /// Cancel for a specific client sequence (no-op).
    pub fn cancel_for_client_seq(&self, _seq: u64) {}

    /// Submit a device code (no-op, returns false).
    pub fn submit_code(&self, _code: &str) -> bool {
        false
    }

    /// Take the URL receiver (returns None).
    pub fn take_url_rx(&self) -> Option<tokio::sync::oneshot::Receiver<String>> {
        None
    }

    /// Begin an interactive auth session (no-op).
    pub fn begin(
        &self,
        _channels: Option<AttemptChannels>,
        _client_seq: Option<u64>,
    ) -> (tokio_util::sync::CancellationToken, ()) {
        (tokio_util::sync::CancellationToken::new(), ())
    }
}
