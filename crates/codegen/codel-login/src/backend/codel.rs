//! The Codel Build backend: the authority this build talks to.

use super::AuthBackend;
use crate::{CodelAuth, CodelComConfig};

#[derive(Default)]
pub struct CodelAuthBackend;

impl AuthBackend for CodelAuthBackend {
    fn scope_key(&self, config: &CodelComConfig) -> String {
        config.auth_scope()
    }

    /// An Codel credential can be minted several ways, so there is no one issuer to check for.
    /// Saying yes to all of them is safe: a credential minted elsewhere still gets sent to Codel, which rejects it.
    fn owns(&self, _auth: &CodelAuth) -> bool {
        true
    }

    /// Some customers run their own gateway and authenticate there with the credential Codel issued them, so a list of allowed hosts would lock them out.
    /// The models cache stops one backend's models from being used by another: it remembers the URL each entry came from and ignores the rest.
    fn may_receive_session(&self, _url: &str) -> bool {
        true
    }

    fn credential_host(&self, config: &CodelComConfig) -> String {
        super::host_of(&config.codel_ws_origin)
    }

    fn is_codel_authority(&self) -> bool {
        true
    }
}
