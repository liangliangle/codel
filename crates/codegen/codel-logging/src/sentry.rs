//! Crash-report upload hook.
//!
//! The upstream implementation shipped panics and traces to a hosted Sentry
//! project. The fork does not report anything off-box, so the public surface is
//! kept as no-ops and no Sentry client is ever constructed. Local crash capture
//! lives in `codel-crash-handler`.

use std::marker::PhantomData;

/// Per-host config. Retained for API compatibility; the values are unused.
pub struct Config {
    /// Sentry tag `client`, e.g. `"codel-pager"`.
    pub client: &'static str,
    pub client_version: &'static str,
    pub release: &'static str,
    /// Retained for API compatibility.
    pub disabled: bool,
}

/// Guard returned by [`init`]. Holds no client.
pub struct ClientInitGuard(PhantomData<()>);

/// No-op: nothing is reported off-box, so there is nothing to keep alive.
pub fn init(_config: Config) -> ClientInitGuard {
    ClientInitGuard(PhantomData)
}

/// No-op: there are no in-flight events to flush.
pub fn flush_on_shutdown() {}
