//! Going back, and noticing when something else moved what tapkey owns.

use serde_json::{Value, json};

mod support;
use support::{Machine, call};

fn zai() -> Value {
    json!({"profiles": [{
        "id": "zai", "name": "Z.ai GLM",
        "tools": {"claude": {"endpoint": "https://api.z.ai/api/anthropic",
                             "slots": {"main": "glm-5.3"}}}
    }]})
}

fn switch(p: &str) -> Value {
    json!({"version": 1, "op": "switch", "params": {"profile_id": p}})
}

fn settings(machine: &Machine) -> String {
    std::fs::read_to_string(machine.home().join(".claude").join("settings.json")).expect("read")
}

fn claude(response: &Value) -> &Value {
    response["tools"]
        .as_array()
        .expect("tools")
        .iter()
        .find(|t| t["tool"] == "claude")
        .expect("claude")
}

#[test]
fn restoring_the_snapshot_returns_the_file_as_it_was_before_tapkey() {
    let before = b"{\n  \"theme\": \"dark\"\n}\n";
    let machine = Machine::new("rs-snapshot");
    machine.write_profiles(zai());
    machine.write_user_settings_raw(before);
    call(&machine, switch("zai"));
    assert_ne!(
        settings(&machine).as_bytes(),
        before,
        "the switch changed it"
    );

    let response = call(
        &machine,
        json!({"version": 1, "op": "restore", "params": {"target": "snapshot"}}),
    );

    assert_eq!(response["outcome"], json!("applied"), "{response}");
    assert_eq!(settings(&machine).as_bytes(), before, "byte for byte");
}

/// Restoring is a change tapkey makes, so it takes a backup of its own — otherwise the one
/// action with no way back is the way back itself.
#[test]
fn restoring_takes_a_backup_before_it_acts() {
    let machine = Machine::new("rs-backs-up");
    machine.write_profiles(zai());
    machine.write_user_settings_raw(b"{}");
    call(&machine, switch("zai"));

    call(
        &machine,
        json!({"version": 1, "op": "restore", "params": {"target": "snapshot"}}),
    );

    let count = std::fs::read_dir(machine.store().join("backups"))
        .expect("backups")
        .count();
    assert_eq!(count, 2, "one for the switch, one for the restore");
}

/// The target is tagged rather than inferred from the shape of an id: "snapshot cannot look
/// like a timestamp" is a guard that lasts exactly until the format changes.
#[test]
fn an_unknown_restore_target_is_refused() {
    let machine = Machine::new("rs-unknown");
    machine.write_profiles(zai());
    machine.write_user_settings_raw(b"{}");

    let response = call(
        &machine,
        json!({"version": 1, "op": "restore", "params": {"target": "backup", "id": "nope"}}),
    );

    assert_eq!(response["ok"], json!(false), "{response}");
}

/// Drift is defined on the slot, not on the file: Claude Code re-serialises its whole settings
/// file on every write it makes, so a file-level signal would fire constantly and stop being
/// read.
#[test]
fn a_value_changed_outside_tapkey_is_reported_as_drift() {
    let machine = Machine::new("dr-drift");
    machine.write_profiles(zai());
    machine.write_user_settings_raw(b"{}");
    call(&machine, switch("zai"));

    // Something else edits the slot tapkey owns.
    let path = machine.home().join(".claude").join("settings.json");
    let edited = settings(&machine).replace("glm-5.3", "somebody-elses-model");
    std::fs::write(&path, edited).expect("write");

    let response = call(
        &machine,
        json!({"version": 1, "op": "effective_state", "params": {}}),
    );

    let main = claude(&response)["slots"]
        .as_array()
        .expect("slots")
        .iter()
        .find(|s| s["slot"] == "main")
        .expect("main");
    assert_eq!(main["drifted"], json!(true), "{response}");
}

/// The tool rewriting its own file is not drift. It does that constantly, and a signal that
/// fires constantly stops being read.
#[test]
fn a_file_reformatted_by_the_tool_is_not_drift() {
    let machine = Machine::new("dr-reserialised");
    machine.write_profiles(zai());
    machine.write_user_settings_raw(b"{\n  \"theme\": \"dark\"\n}\n");
    call(&machine, switch("zai"));

    // What Claude Code does on any write of its own: same content, its own layout.
    let path = machine.home().join(".claude").join("settings.json");
    let value: Value = serde_json::from_str(&settings(&machine)).expect("parse");
    std::fs::write(&path, serde_json::to_string(&value).expect("serialise")).expect("write");

    let response = call(
        &machine,
        json!({"version": 1, "op": "effective_state", "params": {}}),
    );

    let drifted: Vec<&Value> = claude(&response)["slots"]
        .as_array()
        .expect("slots")
        .iter()
        .filter(|s| s["drifted"] == json!(true))
        .collect();
    assert!(
        drifted.is_empty(),
        "reformatting is the tool's business: {drifted:?}"
    );
}

/// Nothing has been written yet, so nothing can have drifted. Reporting drift against slots
/// tapkey never owned would make the signal meaningless on a fresh machine.
#[test]
fn without_a_switch_there_is_nothing_to_drift_from() {
    let machine = Machine::new("dr-fresh");
    machine.write_user_settings_raw(br#"{"env":{"ANTHROPIC_MODEL":"theirs"}}"#);

    let response = call(
        &machine,
        json!({"version": 1, "op": "effective_state", "params": {}}),
    );

    for slot in claude(&response)["slots"].as_array().expect("slots") {
        assert_eq!(slot["drifted"], json!(false), "{slot}");
    }
}

/// Restoring the snapshot hands the tool back to its own login, so tapkey owns nothing
/// afterwards — or drift would fire on values that are no longer ours.
#[test]
fn restoring_the_snapshot_records_owning_nothing() {
    let machine = Machine::new("dr-after-restore");
    machine.write_profiles(zai());
    machine.write_user_settings_raw(br#"{"env":{"ANTHROPIC_MODEL":"theirs"}}"#);
    call(&machine, switch("zai"));

    call(
        &machine,
        json!({"version": 1, "op": "restore", "params": {"target": "snapshot"}}),
    );
    let response = call(
        &machine,
        json!({"version": 1, "op": "effective_state", "params": {}}),
    );

    for slot in claude(&response)["slots"].as_array().expect("slots") {
        assert_eq!(slot["drifted"], json!(false), "{slot}");
    }
}
