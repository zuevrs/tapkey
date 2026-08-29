//! Drift, back-fill and restore for Codex.
//!
//! Codex inverts every premise Claude Code's drift model rested on and lands in the same place.
//! It preserves bytes across its own writes — so the byte-stability argument is weaker here — but
//! it writes **the very keys tapkey owns**: `/model` writes `model` and `model_reasoning_effort`,
//! and a startup migration prompt writes the same pair unprompted. File-level drift would fire on
//! an unrelated `mcp add`, and its inode and mtime change on every write including a repeat, so
//! there is no cheap file-level signal to be had either.

use serde_json::{Value, json};

mod support;
use support::{Machine, call};

fn switch(profile: &str) -> Value {
    json!({"version": 1, "op": "switch", "params": {"profile_id": profile}})
}

fn effective_state() -> Value {
    json!({"version": 1, "op": "effective_state", "params": {}})
}

fn profiles() -> Value {
    json!({
        "providers": [{
            "id": "zai",
            "name": "Z.ai",
            "base_url": "https://api.z.ai/api/v1",
            "formats": ["openai_responses"],
            "enabled": true
        }],
        "profiles": [{
            "id": "glm",
            "name": "Z.ai GLM",
            "tools": {"codex": {"provider": "zai", "slots": {"main": "glm-5.3"}}}
        }]
    })
}

fn slot(response: &Value, name: &str) -> Value {
    response["tools"]
        .as_array()
        .expect("tools")
        .iter()
        .find(|t| t["tool"] == json!("codex"))
        .and_then(|t| t["slots"].as_array().cloned())
        .expect("slots")
        .into_iter()
        .find(|s| s["slot"] == json!(name))
        .unwrap_or_else(|| panic!("no {name} slot in {response}"))
}

fn config_path(machine: &Machine) -> std::path::PathBuf {
    machine.home().join(".codex").join("config.toml")
}

/// Somebody's `/model` afterwards is drift, and the slot says so.
#[test]
fn a_slot_changed_after_the_switch_reads_as_drifted() {
    let machine = Machine::new("cx-drift");
    machine.write_profiles(profiles());
    machine.write_codex_config(b"model = \"gpt-5.6\"\n");
    call(&machine, switch("glm"));

    // What `/model` does, byte for byte: the same key, a different value.
    let after = std::fs::read_to_string(config_path(&machine)).expect("read");
    std::fs::write(
        config_path(&machine),
        after.replace("glm-5.3", "somebody-elses-choice"),
    )
    .expect("write");

    let response = call(&machine, effective_state());

    let main = slot(&response, "main");
    assert_eq!(main["effective"], json!("somebody-elses-choice"));
    assert_eq!(main["drifted"], json!(true), "{main}");
}

/// Re-applying the same profile is the fix, so a switch after drift settles it rather than
/// refusing — refusing would strand the person exactly when they are putting it right.
#[test]
fn re_applying_the_profile_settles_the_drift() {
    let machine = Machine::new("cx-drift-fix");
    machine.write_profiles(profiles());
    machine.write_codex_config(b"model = \"gpt-5.6\"\n");
    call(&machine, switch("glm"));
    let after = std::fs::read_to_string(config_path(&machine)).expect("read");
    std::fs::write(config_path(&machine), after.replace("glm-5.3", "elsewhere")).expect("write");

    call(&machine, switch("glm"));

    let main = slot(&call(&machine, effective_state()), "main");
    assert_eq!(main["effective"], json!("glm-5.3"));
    assert_eq!(main["drifted"], json!(false), "{main}");
}

/// No fingerprint is **not** drift. A `model` already in the file the first time tapkey looks is
/// the initial state — it belongs to the snapshot and to back-fill. Calling somebody's own
/// configuration drift accuses them of a change they did not make.
#[test]
fn a_value_tapkey_never_wrote_is_not_drift() {
    let machine = Machine::new("cx-firstsight");
    machine.write_profiles(profiles());
    machine.write_codex_config(b"model = \"was-here-before-us\"\n");

    let main = slot(&call(&machine, effective_state()), "main");

    assert_eq!(main["effective"], json!("was-here-before-us"));
    assert_eq!(main["drifted"], json!(false), "{main}");
}

/// Restore returns the file to what was on disk before tapkey's first change — comments and all.
#[test]
fn restoring_the_snapshot_returns_the_original_bytes() {
    let machine = Machine::new("cx-restore");
    machine.write_profiles(profiles());
    let original = b"# hand written\nmodel = \"gpt-5.6\"\n\n[mcp_servers.thing]\nargs = [\"hi\"]\n";
    machine.write_codex_config(original);
    call(&machine, switch("glm"));
    assert_ne!(
        std::fs::read(config_path(&machine)).expect("read"),
        original,
        "the switch must have changed something, or this test proves nothing"
    );

    let response = call(
        &machine,
        json!({"version": 1, "op": "restore", "params": {"target": "snapshot"}}),
    );

    assert_eq!(response["outcome"], json!("applied"), "{response}");
    assert_eq!(
        std::fs::read(config_path(&machine)).expect("read"),
        original
    );
}

/// tapkey creates `config.toml`, and `~/.codex/` with it, when neither is there. Measured: Codex
/// does not create a config file it did not find, so an installed-but-unconfigured tool is an
/// ordinary state — and it is the case this app exists for. A missing *binary* is a different
/// absence and stays `tool_gone`.
#[test]
fn a_tool_with_no_config_file_yet_still_switches() {
    let machine = Machine::new("cx-no-file");
    machine.write_profiles(profiles());

    let response = call(&machine, switch("glm"));

    assert_eq!(response["outcome"], json!("applied"), "{response}");
    let written = std::fs::read_to_string(config_path(&machine)).expect("the file was created");
    assert!(written.contains("model = \"glm-5.3\""), "{written}");
}

/// The first-run snapshot is the floor under every restore, and it goes down **before the first
/// change over every managed file** — not only the ones that switch happened to touch. With one
/// tool the two were the same set; with two they are not, and a snapshot that recorded only the
/// touched file would leave the other tool with nothing to come back to.
#[test]
fn the_snapshot_covers_a_tool_the_first_switch_did_not_touch() {
    let machine = Machine::new("cx-snapshot-scope");
    machine.write_profiles(json!({
        "providers": [{
            "id": "zai", "name": "Z.ai", "base_url": "https://api.z.ai/api/v1",
            "formats": ["openai_responses", "anthropic_messages"], "enabled": true
        }],
        "profiles": [
            {"id": "codex-only", "name": "Codex only",
             "tools": {"codex": {"provider": "zai", "slots": {"main": "glm-5.3"}}}},
            {"id": "claude-only", "name": "Claude only",
             "tools": {"claude": {"provider": "zai", "slots": {"main": "glm-5.3"}}}}
        ]
    }));
    let claude_original = b"{\n  \"theme\": \"dark\"\n}\n";
    machine.write_user_settings_raw(claude_original);
    machine.write_codex_config(b"model = \"gpt-5.6\"\n");

    // The first switch touches Codex alone, so this is the moment the snapshot is taken.
    call(&machine, switch("codex-only"));
    // Only afterwards does anything touch Claude Code.
    call(&machine, switch("claude-only"));

    call(
        &machine,
        json!({"version": 1, "op": "restore", "params": {"target": "snapshot"}}),
    );

    assert_eq!(
        std::fs::read(machine.home().join(".claude").join("settings.json")).expect("read"),
        claude_original,
        "the snapshot did not record a file the first switch left alone"
    );
}

/// A restore reports every managed tool, like every other operation. It did not: the core
/// enumerated its tools in five separate places and one of them was written before Codex existed.
/// That omission is the evidence the adapter seam is real — the cost of not having one is a list
/// somebody eventually forgets to extend.
#[test]
fn a_restore_reports_every_tool() {
    let machine = Machine::new("cx-restore-reports");
    machine.write_profiles(profiles());
    machine.write_codex_config(b"model = \"gpt-5.6\"\n");
    call(&machine, switch("glm"));

    let response = call(
        &machine,
        json!({"version": 1, "op": "restore", "params": {"target": "snapshot"}}),
    );

    let reported: Vec<&str> = response["tools"]
        .as_array()
        .expect("tools")
        .iter()
        .filter_map(|t| t["tool"].as_str())
        .collect();
    assert_eq!(reported, vec!["claude", "codex"], "{response}");
}
