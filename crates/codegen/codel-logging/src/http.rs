//! Origin/client identification used by the telemetry engine.
//!
//! [`OriginClientInfo`] is owned by `codel-sampler` (so `SamplerConfig` can use it without depending on shell).
//! Re-exported here so the telemetry engine can label events without depending on shell or sampler internals beyond the type itself.

pub use codel_sampler::OriginClientInfo;

/// Construct an [`OriginClientInfo`] from the `CODEL_CLIENT_NAME` / `CODEL_CLIENT_VERSION` env vars.
/// Returns `None` when `CODEL_CLIENT_NAME` is unset.
/// This is a free function rather than an inherent method because the type lives in another crate.
pub fn origin_client_info_from_env() -> Option<OriginClientInfo> {
    std::env::var("CODEL_CLIENT_NAME")
        .ok()
        .map(|product| OriginClientInfo {
            product,
            version: std::env::var("CODEL_CLIENT_VERSION").ok(),
        })
}
