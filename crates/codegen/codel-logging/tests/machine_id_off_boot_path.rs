//! Fresh-process pins; the assertions consume process-global state.

#[test]
fn env_override_pins_the_agent_id_without_persisting_it() {
    let home = tempfile::tempdir().expect("tempdir");
    // SAFETY: single-threaded here; set before anything caches `codel_home()`.
    unsafe {
        std::env::set_var("CODEL_HOME", home.path());
        std::env::set_var("CODEL_AGENT_ID", "pinned-agent-id");
    }
    assert_eq!(codel_logging::id::agent_id(), "pinned-agent-id");
    assert!(!home.path().join("agent_id").exists());
}
