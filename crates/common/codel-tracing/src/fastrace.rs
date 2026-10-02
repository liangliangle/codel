use fastrace::prelude::*;
use std::borrow::Cow;

// No reporter is installed: this crate's spans stay in-process, so nothing is
// batched to a collector. The trace-context helpers below only give callers
// existing or fresh W3C ids to correlate local spans and outgoing requests.

pub fn current_trace_id() -> Option<String> {
    SpanContext::current_local_parent().map(|current| current.encode_w3c_traceparent())
}

pub fn local_or_random_span_ctx() -> SpanContext {
    SpanContext::current_local_parent().unwrap_or_else(SpanContext::random)
}

pub fn enter_span_with_traceparent(name: impl Into<Cow<'static, str>>, traceparent: &str) -> Span {
    if let Some(span_ctx) = SpanContext::decode_w3c_traceparent(traceparent) {
        Span::root(name, span_ctx)
    } else {
        Span::enter_with_local_parent(name)
    }
}

// Tonic channel (TODO: Move into grpc_client when deprecated tracing)
#[allow(dead_code)]
pub type FastraceChannel = fastrace_tonic::FastraceClientService<tonic::transport::Channel>;

pub fn fastrace_channel(
    channel: tonic::transport::Channel,
) -> fastrace_tonic::FastraceClientService<tonic::transport::Channel> {
    tower::ServiceBuilder::new()
        .layer(fastrace_tonic::FastraceClientLayer)
        .service(channel)
}

// Request middleware (TODO: Move into http_client when deprecated tracing)
#[derive(Clone)]
#[allow(dead_code)]
pub struct TraceparentMiddleware;

#[async_trait::async_trait]
impl reqwest_middleware::Middleware for TraceparentMiddleware {
    async fn handle(
        &self,
        mut req: reqwest::Request,
        extensions: &mut http::Extensions,
        next: reqwest_middleware::Next<'_>,
    ) -> reqwest_middleware::Result<reqwest::Response> {
        req.headers_mut()
            .extend(fastrace_reqwest::traceparent_headers());
        next.run(req, extensions).await
    }
}
