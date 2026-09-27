#![deny(clippy::indexing_slicing)]

//! W3C trace-context helpers plus shared redaction.
//!
//! The crate carries no exporter: [`set_local_trace_subscriber`] gives
//! `traceparent`-bearing spans real W3C ids in the local subscriber, and the
//! header helpers copy those ids between spans and outgoing requests. Nothing
//! is batched, queued or shipped off-box.

pub mod redact_common;
mod trace_context;

pub use trace_context::*;
