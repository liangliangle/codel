//! The published external-auth contract, end to end.
//!
//! Operator binaries live outside this repo and read `CODEL_AUTH_EXPIRED=1` as "headless, don't prompt".
//! They decline a run they cannot complete silently.
//! So a binary that declines the boot probe must still be able to sign the user in.
//! The two runs have to reach it in the order boot produces them.

#![cfg(unix)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use chrono::Utc;
use codel_login::{
    AuthMode, CodelAuth, CodelComConfig, ensure_authenticated, try_ensure_fresh_auth,
};

const STALE_TOKEN: &str = "stale-token-the-provider-will-not-renew";
const SSO_TOKEN: &str = "token-minted-by-the-interactive-flow";

/// A regression here reaches the browser login, which hangs rather than fails.
const LOGIN_BUDGET: Duration = Duration::from_secs(60);

/// Measured at ~0.3s healthy and 47s while the flow contended with its own `auth.json.lock`; loose enough for a loaded CI runner in between.
const NO_SELF_CONTENTION: Duration = Duration::from_secs(20);

/// The skeleton published in `README.md` and `docs/user-guide/02-authentication.md`, which operators copy.
fn write_conforming_provider(home: &Path) -> String {
    use std::os::unix::fs::PermissionsExt;

    let log = invocation_log(home);
    let script = home.join("acme-auth.sh");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\n\
             echo \"expired=${{CODEL_AUTH_EXPIRED:-unset}}\" >> {log}\n\
             if [ \"$CODEL_AUTH_EXPIRED\" = \"1\" ]; then\n\
             \x20   echo 'SSO session lapsed; cannot mint without the user' >&2\n\
             \x20   exit 1\n\
             fi\n\
             echo 'Authenticating via Acme Corp SSO...' >&2\n\
             printf '%s' {SSO_TOKEN}\n",
            log = log.display(),
        ),
    )
    .expect("write provider script");
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))
        .expect("chmod provider script");
    script.display().to_string()
}

fn invocation_log(home: &Path) -> PathBuf {
    home.join("provider-invocations")
}

fn invocations(home: &Path) -> Vec<String> {
    std::fs::read_to_string(invocation_log(home))
        .map(|s| s.lines().map(str::to_owned).collect())
        .unwrap_or_default()
}


fn dead_endpoint() -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();
    drop(listener);
    format!("http://127.0.0.1:{port}")
}

