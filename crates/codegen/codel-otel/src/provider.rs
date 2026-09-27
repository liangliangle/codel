//! Trace provider construction.
//!
//! The upstream implementation built an OTLP span exporter that shipped spans
//! to a hosted collector. The fork does not export traces off-box, so
//! [`build_otel_layer`] installs an inert layer and [`shutdown_provider`] is a
//! no-op. The public surface is unchanged so callers keep compiling.

use std::sync::Arc;

use opentelemetry_sdk::trace::SpanData;
use tracing_subscriber::Layer as _;
use tracing_subscriber::registry::LookupSpan;

use crate::config::{OtelClientInfo, OtelLayerConfig};

#[derive(Debug, Clone, Copy)]
pub enum OtelProviderMode {
    Server,
    Local,
}

pub type SessionMetricsGate = Arc<dyn Fn() -> bool + Send + Sync>;

/// Redacts each span batch in place before export. Retained for API
/// compatibility; no batch is ever exported.
pub type SpanRedactor = Arc<dyn Fn(&mut [SpanData]) + Send + Sync>;

/// Install the tracing -> trace layer.
///
/// No-op: spans are never exported, so no tracing layer is attached.
pub fn build_otel_layer<S>(
    client: OtelClientInfo,
    config: OtelLayerConfig,
    mode: OtelProviderMode,
    session_metrics_gate: SessionMetricsGate,
    redact: SpanRedactor,
) -> impl tracing_subscriber::layer::Layer<S>
where
    S: tracing::Subscriber + for<'span> LookupSpan<'span>,
{
    let _ = (client, config, mode, session_metrics_gate, redact);
    tracing_subscriber::layer::Identity::new()
}

/// No-op: there is no provider to flush or shut down.
pub fn shutdown_provider() {}
