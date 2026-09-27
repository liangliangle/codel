//! End-to-end guard for `auth_provider_command`: a configured external auth provider must actually mint the session credential on the host platform.
//!
//! The provider used to be spawned through a hardcoded `sh -c`.
//! On Windows that either fails to spawn (no `sh` in a default install) or silently eats the backslashes in a native path where Git Bash is present.
//! `C:\Windows\System32\whoami.exe` reaches the shell as `C:WindowsSystem32whoami.exe` and exits 127.
//! Either way the auth flow fell through to the built-in browser login, so a configured provider looked like it had been ignored.
//!
//! The test drives the public entry point: `try_ensure_fresh_auth`, then `AuthManager::auth`, the external refresher, and the platform shell.
//! It is hermetic: a throwaway `CODEL_HOME`, no network, and a provider command that needs no binary beyond what the platform shell already provides.

use std::collections::BTreeMap;
use std::path::Path;

use chrono::Utc;
use codel_login::{AuthMode, CodelAuth, CodelComConfig, try_ensure_fresh_auth};

const SEED_TOKEN: &str = "stale-token-that-must-be-replaced";

/// `codel_home()` memoizes into a `OnceLock`, so every phase below shares this one directory.
/// That is why the phases live in a single test rather than racing each other as separate ones.
fn use_temp_codel_home(dir: &Path) {
    // SAFETY: single-threaded test entry, before any thread that reads the
    // environment is spawned.
    unsafe {
        std::env::set_var("CODEL_HOME", dir);
    }
}



