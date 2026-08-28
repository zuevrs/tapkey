//! The fingerprint drift compares against. It is written to disk, so its values are part of
//! the format: changing the algorithm silently would make every stored state unreadable while
//! looking like nothing had happened.

use tapkey_core::fingerprint::{State, hash};

/// The published FNV-1a 64-bit vectors, plus two model names computed independently. Deriving
/// them the way the code derives them would make this pass by construction.
#[test]
fn hashes_match_the_published_vectors() {
    for (input, expected) in [
        ("", "cbf29ce484222325"),
        ("a", "af63dc4c8601ec8c"),
        ("foobar", "85944171f73967e8"),
        ("glm-5.3", "1bbc9f46730f0bd4"),
    ] {
        assert_eq!(hash(input), format!("fnv1a64:{expected}"), "for {input:?}");
    }
}

/// Two model names one character apart must not collide, or a switch between them would look
/// like no change at all and drift would never fire.
#[test]
fn near_identical_values_hash_differently() {
    assert_ne!(hash("glm-5.3"), hash("glm-5.4"));
    assert_ne!(hash("claude-opus-5"), hash("claude-opus-4"));
    assert_ne!(hash("ab"), hash("ba"), "order has to matter");
}

#[test]
fn an_unknown_slot_never_drifts() {
    let state = State::default();
    assert!(!state.drifted("claude", "main", Some("anything")));
}

#[test]
fn a_state_file_that_is_not_there_reports_no_drift() {
    let state = State::read(std::path::Path::new("/nonexistent/state.json"));
    assert!(!state.drifted("claude", "main", Some("anything")));
}
