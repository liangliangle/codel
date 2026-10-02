//! Compile-time selection of the authority this build talks to.
//!
//! One implementation compiles, so the trait is a checklist: a backend that
//! forgets a decision fails to build. Upstream's trait also carried the login
//! entry point (`LoginRequest`) and the session-token refresher factory; the
//! fork authenticates with an API key, so both are gone and what remains is the
//! set of decisions the credential plumbing still needs.

use crate::{CodelAuth, CodelComConfig};

mod codel;

/// Every method is a decision the credential plumbing would otherwise guess.
pub trait AuthBackend {
    /// Key under which this backend owns its entry in auth.json.
    fn scope_key(&self, config: &CodelComConfig) -> String;
    /// Whether this backend minted the credential, which the scope key alone cannot establish.
    fn owns(&self, auth: &CodelAuth) -> bool;
    /// Whether this backend's credential may be sent to `url`.
    /// A model entry carries its own base URL, so without this a poisoned or hand-edited entry aims the bearer anywhere.
    fn may_receive_session(&self, url: &str) -> bool;
    /// The host to name when telling the user whose credential they hold.
    fn credential_host(&self, config: &CodelComConfig) -> String;
    /// Whether Codel issued this backend's credentials and may therefore receive them.
    /// Gates every request that carries the bearer to an Codel host, and every Codel-only policy.
    fn is_codel_authority(&self) -> bool;
}
pub type ActiveAuthBackend = codel::CodelAuthBackend;
/// Reports a URL the way a user says it, without the scheme.
pub fn host_of(url: &str) -> String {
    url.strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .unwrap_or(url)
        .to_owned()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn host_of_drops_the_scheme_and_leaves_the_rest_alone() {
        assert_eq!(host_of("https://example.test"), "example.test");
        assert_eq!(host_of("http://localhost:8080"), "localhost:8080");
        assert_eq!(host_of("codel.dev"), "codel.dev");
    }
}
