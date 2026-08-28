//! One writer at a time.

use serde_json::json;
use tapkey_core::lock::Lock;

mod support;
use support::{Machine, TempDir, call};

#[test]
fn a_second_holder_is_refused_rather_than_made_to_wait() {
    let dir = TempDir::new("lock-second");
    std::fs::create_dir_all(dir.path()).expect("mkdir");

    let first = Lock::acquire(dir.path()).expect("the first holder gets it");
    let second = Lock::acquire(dir.path());

    assert!(
        second.is_err(),
        "a switch takes milliseconds; a queue would apply it too late"
    );
    drop(first);
    assert!(Lock::acquire(dir.path()).is_ok(), "and it is released");
}

#[test]
fn a_switch_while_another_holds_the_lock_is_refused_and_changes_nothing() {
    let machine = Machine::new("lock-switch");
    machine.write_profiles(json!({"profiles": [{
        "id": "zai", "name": "Z.ai",
        "tools": {"claude": {"endpoint": "https://api.z.ai/api/anthropic", "slots": {}}}
    }]}));
    machine.write_user_settings_raw(b"{}");
    std::fs::create_dir_all(machine.store()).expect("store");
    let _held = Lock::acquire(&machine.store()).expect("held by somebody else");

    let response = call(
        &machine,
        json!({"version": 1, "op": "switch", "params": {"profile_id": "zai"}}),
    );

    assert_eq!(response["ok"], json!(false), "{response}");
    assert_eq!(response["failure"]["kind"], json!("busy"));
    let after = std::fs::read(machine.home().join(".claude").join("settings.json")).expect("read");
    assert_eq!(after, b"{}", "refused before anything was staged");
}

/// Reading never takes the lock. Blocking effective state for the duration of a write would
/// blank the interface at the one moment it is most interesting.
#[test]
fn reading_effective_state_is_not_blocked_by_a_held_lock() {
    let machine = Machine::new("lock-read");
    machine.write_user_settings_raw(br#"{"env":{"ANTHROPIC_BASE_URL":"https://x.test"}}"#);
    std::fs::create_dir_all(machine.store()).expect("store");
    let _held = Lock::acquire(&machine.store()).expect("held");

    let response = call(
        &machine,
        json!({"version": 1, "op": "effective_state", "params": {}}),
    );

    assert_eq!(response["ok"], json!(true), "{response}");
}

/// There is no explicit unlock: closing the descriptor releases the lock. That is what makes a
/// stale lock impossible, so it is worth a test of its own rather than resting on the fact that
/// the previous test happens to drop one.
#[test]
fn the_lock_goes_when_the_holder_does_and_nothing_has_to_notice() {
    let dir = TempDir::new("lock-release");
    std::fs::create_dir_all(dir.path()).expect("mkdir");

    for _ in 0..3 {
        let held = Lock::acquire(dir.path()).expect("acquire");
        assert!(Lock::acquire(dir.path()).is_err(), "held");
        drop(held);
    }
    assert!(Lock::acquire(dir.path()).is_ok());
}
