// Per-test-case module for the `pty_e2e` integration test crate.
#[allow(unused_imports)]
use super::common::*;

/// A single argv containing whitespace routes through `$SHELL -i -c`, the same hop OSC 52 takes.
const PRINT_APPEARANCE: &str =
    "printf 'codel=%s lc=%s\\n' \"$CODEL_APPEARANCE\" \"$LC_CODEL_APPEARANCE\"";

fn parse_printed_appearance(raw: &str) -> Option<(String, String)> {
    let line = raw.lines().find(|l| l.starts_with("codel="))?;
    let rest = line.strip_prefix("codel=")?;
    let (codel, lc) = rest.split_once(" lc=")?;
    Some((codel.to_owned(), lc.to_owned()))
}

/// End-to-end check that the appearance stamp survives the interactive shell hop. Do not call
/// `detect_desktop()` here: two live portal probes can disagree.
#[test]
#[ignore = "PTY e2e; run the owning pty_e2e_* Cargo test with --ignored (see Cargo.toml)"]
#[cfg(unix)]
fn wrap_appearance_env_advertised_through_shell() {
    let (code, raw) = run_wrap(
        &[PRINT_APPEARANCE],
        &[
            ("SHELL", "/bin/sh"),
            ("COLORFGBG", "15;0"),
            ("CODEL_APPEARANCE", ""),
            ("LC_CODEL_APPEARANCE", ""),
        ],
    );
    let (codel, lc) = parse_printed_appearance(&raw)
        .unwrap_or_else(|| panic!("missing codel=/lc= line\nraw:\n{raw}"));
    match (codel.as_str(), lc.as_str()) {
        ("", "") => {}
        ("dark", "dark") | ("light", "light") => {}
        _ => panic!(
            "CODEL and LC must agree and not invent from COLORFGBG; codel={codel:?} lc={lc:?}\nraw:\n{raw}"
        ),
    }
    assert_eq!(
        code,
        Some(0),
        "shell-routed printf must exit 0\nraw:\n{raw}"
    );
}

/// The parent sets `CODEL_APPEARANCE=light` and pins LC empty.
/// A desktop probe that answers overrides both names to the same polarity; one that answers `None` inherits CODEL and must not invent LC.
/// The test itself never probes the desktop; a second live probe could disagree.
#[test]
#[ignore = "PTY e2e; run the owning pty_e2e_* Cargo test with --ignored (see Cargo.toml)"]
#[cfg(unix)]
fn wrap_appearance_env_desktop_none_does_not_restamp_parent_codel() {
    let (code, raw) = run_wrap(
        &[PRINT_APPEARANCE],
        &[
            ("SHELL", "/bin/sh"),
            ("CODEL_APPEARANCE", "light"),
            ("LC_CODEL_APPEARANCE", ""),
        ],
    );
    let (codel, lc) = parse_printed_appearance(&raw)
        .unwrap_or_else(|| panic!("missing codel=/lc= line\nraw:\n{raw}"));
    match (codel.as_str(), lc.as_str()) {
        ("light", "") => {}
        ("dark", "dark") | ("light", "light") => {}
        _ => panic!(
            "expected inherit codel=light with empty lc, or a matching desktop stamp; codel={codel:?} lc={lc:?}\nraw:\n{raw}"
        ),
    }
    assert_eq!(
        code,
        Some(0),
        "shell-routed printf must exit 0\nraw:\n{raw}"
    );
}
