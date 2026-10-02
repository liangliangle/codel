//! `CODEL_HOME` override tests in an isolated binary so `codel_home()`'s process-wide `OnceLock` initializes from the overridden env var.

use std::path::PathBuf;

#[test]
#[serial_test::serial(CODEL_HOME)]
fn codel_home_override_path_helpers() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let codel_home = tmp.path().to_path_buf();
    unsafe {
        std::env::set_var("CODEL_HOME", &codel_home);
    }

    assert_eq!(
        codel_pager::util::pager_toml_path(),
        codel_home.join("pager.toml")
    );
    assert_eq!(
        codel_pager::util::display_codel_home_prefix(),
        "$CODEL_HOME"
    );
    assert_eq!(
        codel_pager::util::display_user_codel_path("config.toml"),
        "$CODEL_HOME/config.toml"
    );

    let memory_path = codel_home.join("memory/MEMORY.md");
    assert_eq!(
        codel_pager::util::abbreviate_path(&memory_path.display().to_string()),
        "$CODEL_HOME/memory/MEMORY.md"
    );

    // The copy toast abbreviates paths the same way, so a custom $CODEL_HOME outside $HOME still shows the short form
    assert_eq!(
        codel_pager::clipboard::display_copy_path(&codel_home.join("last-copy.txt")),
        "$CODEL_HOME/last-copy.txt"
    );

    assert!(codel_pager::util::is_under_user_codel_home(&memory_path));
    assert!(!codel_pager::util::is_under_user_codel_home(
        PathBuf::from("/tmp/other").as_path()
    ));
}

/// Isolated because `codel_home()`'s `OnceLock` is already initialized by the time the shared lib-test binary reaches a case like this.
#[test]
#[serial_test::serial(CODEL_HOME)]
fn disk_usage_run_creates_no_codel_home() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let ghost = tmp.path().join("ghost-home");
    unsafe {
        std::env::set_var("CODEL_HOME", &ghost);
    }

    for json in [false, true] {
        codel_pager::disk_usage_cmd::run(codel_pager::disk_usage_cmd::DiskUsageArgs {
            json,
            clean: false,
            clean_orphaned: false,
            yes: false,
        })
        .expect("a missing home is not an error");
        assert!(
            !ghost.exists(),
            "codel du must not create the home it reports on (json={json})"
        );
    }
}
