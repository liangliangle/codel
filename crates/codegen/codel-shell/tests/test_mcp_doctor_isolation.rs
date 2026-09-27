//! Isolated binary so `codel_home()`'s process-wide OnceLock initializes from
//! our `CODEL_HOME`. A lib-test EnvGuard is a no-op if another test already
//! resolved it, and then doctor reads the real ~/.codel.

use std::path::PathBuf;
use std::sync::OnceLock;

fn isolate_home() -> &'static PathBuf {
    static HOME: OnceLock<PathBuf> = OnceLock::new();
    HOME.get_or_init(|| {
        let dir = tempfile::TempDir::new().unwrap().keep();
        let codel = dir.join(".codel");
        std::fs::create_dir_all(&codel).unwrap();
        std::fs::write(codel.join("config.toml"), "").unwrap();
        // SAFETY: this binary's only test; set before any codel_home() call.
        unsafe {
            std::env::set_var("HOME", &dir);
            std::env::set_var("USERPROFILE", &dir);
            std::env::set_var("CODEL_HOME", &codel);
        }
        dir
    })
}

#[tokio::test]
async fn run_doctor_skips_managed_gateway_without_configs_probe() {
    let _home = isolate_home();
    let cwd = tempfile::tempdir().unwrap();

    let report = codel_shell::mcp_doctor::run_doctor(cwd.path(), None).await;
    assert!(
        !report.sources.iter().any(|s| s.path == "codel.dev"),
        "doctor must not invent a codel.dev source: {:?}",
        report.sources
    );
    assert!(
        report.servers.is_empty(),
        "isolated cwd must not probe managed HTTP servers: {:?}",
        report.servers
    );
}
