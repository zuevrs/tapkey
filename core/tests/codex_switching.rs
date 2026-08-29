//! Applying a profile to Codex: what lands in `config.toml`, what survives, and what stops it.

use serde_json::{Value, json};

mod support;
use support::{Machine, call};

fn switch(profile: &str) -> Value {
    json!({"version": 1, "op": "switch", "params": {"profile_id": profile}})
}

fn profiles(slots: Value) -> Value {
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
            "tools": {"codex": {"provider": "zai", "slots": slots}}
        }]
    })
}

fn config(machine: &Machine) -> String {
    std::fs::read_to_string(machine.home().join(".codex").join("config.toml")).expect("read")
}

/// The provider id is namespaced **always**, not on collision. Codex rejects its built-in ids as
/// reserved, and a list that can grow would otherwise make an already-written table illegal on
/// somebody else's release — a migration on live machines caused by a decision that was not ours.
#[test]
fn a_switch_writes_the_selection_and_a_namespaced_provider_table() {
    let machine = Machine::new("cx-sw-basic");
    machine.write_profiles(profiles(json!({"main": "glm-5.3"})));
    machine.write_codex_config(b"# mine\nmodel = \"gpt-5.6\"\n");

    let response = call(&machine, switch("glm"));

    assert_eq!(response["outcome"], json!("applied"), "{response}");
    let after = config(&machine);
    assert!(after.contains("model = \"glm-5.3\""), "{after}");
    assert!(
        after.contains("model_provider = \"tapkey-zai\""),
        "the id must be namespaced: {after}"
    );
    assert!(
        after.contains("[model_providers.tapkey-zai]"),
        "and so must the table: {after}"
    );
    assert!(
        after.contains("base_url = \"https://api.z.ai/api/v1\""),
        "{after}"
    );
}

/// `wire_api` has exactly one legal value and `"chat"` is a hard config-load error, so writing it
/// is writing the only thing that works — done **only inside an entry tapkey created**, which is
/// the rule stage one settled for a created `env` block: an entry we made is ours entirely while
/// every key in it is ours; an entry somebody hand-wrote we do not touch.
#[test]
fn our_own_provider_entry_carries_the_only_protocol_codex_speaks() {
    let machine = Machine::new("cx-sw-wire");
    machine.write_profiles(profiles(json!({"main": "glm-5.3"})));
    machine.write_codex_config(b"model = \"gpt-5.6\"\n");

    call(&machine, switch("glm"));

    assert!(config(&machine).contains("wire_api = \"responses\""));
}

/// Ownership is per assignment, not per key name. `review_model` and the subagent slot inherit the
/// session model when unset, so writing them unasked would pin what moves along for free.
#[test]
fn a_slot_the_profile_says_nothing_about_is_left_alone() {
    let machine = Machine::new("cx-sw-unassigned");
    machine.write_profiles(profiles(json!({"main": "glm-5.3"})));
    machine.write_codex_config(b"model = \"gpt-5.6\"\n");

    call(&machine, switch("glm"));

    let after = config(&machine);
    assert!(!after.contains("review_model"), "{after}");
    assert!(!after.contains("default_subagent_model"), "{after}");
}

/// The utility assignment reaches Codex through **two** keys, and both are written whenever it is
/// assigned — unconditionally on `features.memories`. Left unset they fall back to hard-coded
/// OpenAI slugs, requested from the new provider with the new key and failing in silence. Writing
/// conditionally would make the feature flag an input to the switch and leave a leak that can be
/// acquired later with no file change we would ever see.
#[test]
fn the_utility_assignment_reaches_both_memory_keys() {
    let machine = Machine::new("cx-sw-memories");
    machine.write_profiles(profiles(
        json!({"main": "glm-5.3", "utility": "glm-4.6-air"}),
    ));
    machine.write_codex_config(b"model = \"gpt-5.6\"\n");

    call(&machine, switch("glm"));

    let after = config(&machine);
    assert!(after.contains("extract_model = \"glm-4.6-air\""), "{after}");
    assert!(
        after.contains("consolidation_model = \"glm-4.6-air\""),
        "{after}"
    );
}

/// Everything tapkey does not own comes back byte for byte, comments and hand-alignment included.
#[test]
fn every_byte_tapkey_does_not_own_survives() {
    let machine = Machine::new("cx-sw-survive");
    machine.write_profiles(profiles(json!({"main": "glm-5.3"})));
    machine.write_codex_config(
        b"# a note the person wrote\nmodel = \"gpt-5.6\"\napproval_policy = \"never\"\n\n[mcp_servers.thing]\ncommand   = \"/bin/echo\"  # aligned by hand\nargs = [\"hi\"]\n",
    );

    call(&machine, switch("glm"));

    let after = config(&machine);
    assert!(after.contains("# a note the person wrote"), "{after}");
    assert!(after.contains("approval_policy = \"never\""), "{after}");
    assert!(
        after.contains("command   = \"/bin/echo\"  # aligned by hand"),
        "the untouched table did not survive: {after}"
    );
}

/// A top-level `profile` key makes Codex refuse to start — exit 1, before anything else runs. It
/// parses fine, so this is not `unparsable`: the file is fatal rather than broken. The tool is
/// skipped and the key is named; removing it is offered as its own action, never done here, because
/// merge-never-own forbids repairing what we do not own and a write of ours would make tapkey the
/// last hand on a file that does not work.
#[test]
fn a_config_that_stops_codex_starting_is_reported_and_left_alone() {
    let machine = Machine::new("cx-sw-profile-key");
    machine.write_profiles(profiles(json!({"main": "glm-5.3"})));
    let original = b"profile = \"old\"\nmodel = \"gpt-5.6\"\n";
    machine.write_codex_config(original);

    let response = call(&machine, switch("glm"));

    assert_eq!(response["outcome"], json!("applied"), "{response}");
    let attentions = response["tools"]
        .as_array()
        .expect("tools")
        .iter()
        .find(|t| t["tool"] == json!("codex"))
        .and_then(|t| t["attentions"].as_array().cloned())
        .unwrap_or_else(|| panic!("codex must carry an attention: {response}"));
    assert_eq!(attentions[0]["kind"], json!("tool_will_not_start"));
    assert_eq!(attentions[0]["key"], json!("profile"));
    assert_eq!(
        std::fs::read(machine.home().join(".codex").join("config.toml")).expect("read"),
        original,
        "the file must be exactly as it was found"
    );
}
