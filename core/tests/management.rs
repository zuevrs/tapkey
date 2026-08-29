//! Profile and provider management.
//!
//! The dividing line from the ticket: an operation that touches **the tools' files** is ADR-0005
//! verbatim — staged, rolled back whole, backed up; an operation that touches **only the store** is
//! a store write and nothing more. Profile operations are the second kind.

use serde_json::{Value, json};

mod support;
use support::install_helper;
use support::{Machine, call};

fn op(name: &str, params: Value) -> Value {
    json!({"version": 1, "op": name, "params": params})
}

fn seeded() -> Machine {
    let machine = Machine::new("mgmt");
    machine.write_profiles(json!({
        "providers": [
            {"id": "zai", "name": "Z.ai", "base_url": "https://api.z.ai/api/v1",
             "formats": ["openai_responses"], "enabled": true}
        ],
        "profiles": [
            {"id": "glm", "name": "Z.ai GLM",
             "tools": {"codex": {"provider": "zai", "slots": {
                 "main": "glm-5.3",
                 "utility": {"provider": "zai", "model": "glm-4.6-air"}
             }}}}
        ]
    }));
    machine
}

fn profiles(machine: &Machine) -> Value {
    serde_json::from_slice(&std::fs::read(machine.store().join("profiles.json")).expect("read"))
        .expect("the store is JSON")
}

#[test]
fn a_profile_can_be_created_and_renamed_without_touching_its_id() {
    let machine = seeded();

    let response = call(
        &machine,
        op(
            "create_profile",
            json!({"profile": {"id": "deep", "name": "DeepSeek",
                "tools": {"claude": {"provider": "zai", "slots": {"main": "deep-v3"}}}}}),
        ),
    );
    assert_eq!(response["ok"], json!(true), "{response}");
    assert_eq!(profiles(&machine)["profiles"].as_array().unwrap().len(), 2);

    let response = call(
        &machine,
        op(
            "rename_profile",
            json!({"id": "deep", "name": "DeepSeek chat"}),
        ),
    );
    assert_eq!(response["ok"], json!(true), "{response}");
    let stored = profiles(&machine);
    let deep = stored["profiles"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"] == json!("deep"))
        .expect("still there under the same id");
    assert_eq!(deep["name"], json!("DeepSeek chat"), "{stored}");
    // The assignments survived the rename: only the label moved.
    assert!(deep["tools"].is_object(), "{stored}");
}

/// The catalogue's own hint — *same provider, different model* — is what duplicate is for, so the
/// copy carries everything, per-slot providers included. A copy that lost assignments would make
/// that hint a lie.
#[test]
fn duplicating_copies_everything_including_per_slot_providers() {
    let machine = seeded();

    let response = call(
        &machine,
        op(
            "duplicate_profile",
            json!({"id": "glm", "as_id": "glm-two"}),
        ),
    );

    assert_eq!(response["ok"], json!(true), "{response}");
    let stored = profiles(&machine);
    let copy = stored["profiles"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"] == json!("glm-two"))
        .expect("the copy exists");
    let utility = &copy["tools"]["codex"]["slots"]["utility"];
    assert_eq!(
        utility["provider"],
        json!("zai"),
        "the per-slot provider came across: {copy}"
    );
}

/// Deleting the profile the tools are currently on deletes it and reports honestly. Switching the
/// tools to System default would be a large action in answer to a small one — somebody removed a
/// label, and we would have rewritten three configuration files.
#[test]
fn deleting_the_current_profile_deletes_it_and_changes_no_tool() {
    let machine = seeded();
    machine.write_codex_config(b"model = \"glm-5.3\"\n");
    call(
        &machine,
        json!({"version": 1, "op": "switch", "params": {"profile_id": "glm"}}),
    );
    let before = std::fs::read(machine.home().join(".codex").join("config.toml")).expect("read");

    let response = call(&machine, op("delete_profile", json!({"id": "glm"})));

    assert_eq!(response["ok"], json!(true), "{response}");
    assert!(
        profiles(&machine)["profiles"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        std::fs::read(machine.home().join(".codex").join("config.toml")).expect("read"),
        before,
        "the tools keep what was applied"
    );
}

/// A plan naming a removed provider stays visible rather than being cascaded away. A tool stripped
/// of its entry breaks *now*; a plan stripped of its provider fails *when applied* and says why.
#[test]
fn a_profile_naming_a_removed_provider_keeps_the_dangling_reference() {
    let machine = seeded();
    call(&machine, op("delete_profile", json!({"id": "glm"})));
    // Re-create it against a provider id that does not exist in the store.
    call(
        &machine,
        op(
            "create_profile",
            json!({"profile": {"id": "ghost", "name": "Ghost",
                "tools": {"codex": {"provider": "gone", "slots": {}}}}}),
        ),
    );

    let response = call(
        &machine,
        json!({"version": 1, "op": "switch", "params": {"profile_id": "ghost"}}),
    );

    assert_eq!(
        response["failure"]["kind"],
        json!("unknown_provider"),
        "{response}"
    );
}

/// Creating a profile that names no managed tool is the same refusal a switch already gives: an
/// empty profile cannot be applied, so it cannot be created either.
#[test]
fn a_profile_covers_no_tool_is_refused() {
    let machine = seeded();

    let response = call(
        &machine,
        op(
            "create_profile",
            json!({"profile": {"id": "empty", "name": "Empty", "tools": {}}}),
        ),
    );

    assert_eq!(
        response["failure"]["kind"],
        json!("unknown_profile"),
        "{response}"
    );
}

// -- Providers -------------------------------------------------------------------------------

fn provider_profiles() -> Value {
    json!({
        "providers": [
            {"id": "zai", "name": "Z.ai", "base_url": "https://api.z.ai/api/v1",
             "formats": ["openai_responses"], "enabled": true},
            {"id": "spare", "name": "Spare", "base_url": "https://spare.test/v1",
             "enabled": true}
        ],
        "profiles": [
            {"id": "glm", "name": "Z.ai GLM",
             "tools": {"codex": {"provider": "zai", "slots": {"main": "glm-5.3"}}}}
        ]
    })
}

/// Renaming touches the **name** only. The id is a reference held in three tools' registry entries,
/// in profiles and in the credential store, so a rename that moved it would oblige tapkey to walk
/// through everything that mentions it — including tools that may be uninstalled.
#[test]
fn renaming_a_provider_never_touches_the_id() {
    let machine = seeded();
    machine.write_codex_config(b"model = \"gpt-5.6\"\n\n[model_providers.tapkey-zai]\nname = \"Z.ai\"\nbase_url = \"https://api.z.ai/api/v1\"\nwire_api = \"responses\"\n");

    let response = call(
        &machine,
        op("rename_provider", json!({"id": "zai", "name": "Z.ai GLM"})),
    );

    assert_eq!(response["ok"], json!(true), "{response}");
    let stored = profiles(&machine);
    assert_eq!(stored["providers"][0]["name"], json!("Z.ai GLM"));
    assert_eq!(stored["providers"][0]["id"], json!("zai"), "{stored}");
    let after =
        std::fs::read_to_string(machine.home().join(".codex").join("config.toml")).expect("read");
    assert!(
        after.contains("[model_providers.tapkey-zai]"),
        "the registry table is untouched by a display-name change: {after}"
    );
}

/// The refusal ADR-0013 deferred twice was for: removing the entry a tool currently points at
/// breaks the tool outright. The tool is named, and switching away first is the remedy.
#[test]
fn removing_a_provider_a_tool_is_using_is_refused() {
    let machine = Machine::new("mgmt-in-use");
    machine.write_profiles(provider_profiles());
    machine.write_codex_config(
        b"model = \"glm-5.3\"\nmodel_provider = \"tapkey-zai\"\n\n[model_providers.tapkey-zai]\nname = \"Z.ai\"\nbase_url = \"https://api.z.ai/api/v1\"\nwire_api = \"responses\"\n",
    );

    let response = call(&machine, op("remove_provider", json!({"id": "zai"})));

    assert_eq!(response["ok"], json!(false), "{response}");
    assert_eq!(
        response["failure"]["kind"],
        json!("provider_in_use"),
        "{response}"
    );
    // And the refusal left both the file and the store exactly as they were.
    assert!(profiles(&machine)["providers"].as_array().unwrap().len() == 2);
    assert!(
        std::fs::read_to_string(machine.home().join(".codex").join("config.toml"))
            .expect("read")
            .contains("[model_providers.tapkey-zai]"),
    );
}

/// Not selected: the removal goes through, transactionally — a backup first, the registry entry
/// gone after, the person's own tables untouched.
#[test]
fn removing_an_unused_provider_takes_our_entries_and_keeps_theirs() {
    let machine = Machine::new("mgmt-remove");
    // The helper must be reachable, and that is the honest requirement: if it cannot run, the
    // removal refuses rather than deleting a provider while leaving its key behind.
    install_helper(&machine);
    machine.write_profiles(provider_profiles());
    machine.write_codex_config(
        b"# hand written\nmodel = \"gpt-5.6\"\n\n[model_providers.tapkey-spare]\nname = \"Spare\"\nbase_url = \"https://spare.test/v1\"\nwire_api = \"responses\"\n\n[model_providers.mine]\nname = \"Mine\"\nbase_url = \"https://mine.test/v1\"\nwire_api = \"responses\"\n",
    );

    let response = call(&machine, op("remove_provider", json!({"id": "spare"})));

    assert_eq!(response["ok"], json!(true), "{response}");
    let after =
        std::fs::read_to_string(machine.home().join(".codex").join("config.toml")).expect("read");
    assert!(
        !after.contains("tapkey-spare"),
        "our entry is gone: {after}"
    );
    assert!(after.contains("# hand written"), "{after}");
    assert!(
        after.contains("[model_providers.mine]"),
        "the person's own provider is untouched: {after}"
    );
    assert!(profiles(&machine)["providers"].as_array().unwrap().len() == 1);
}

/// The stored key is deleted with its provider — a secret outliving its owner is a secret with no
/// visible owner — and the deletion goes through the helper, the only writer of secrets.
#[test]
fn removing_a_provider_forgets_the_stored_key() {
    let machine = Machine::new("mgmt-forget");
    machine.write_profiles(provider_profiles());
    install_helper(&machine);
    std::fs::create_dir_all(machine.store().join("keys")).expect("keys dir");
    std::fs::write(machine.store().join("keys").join("spare"), b"sk-spare").expect("seed");

    let response = call(&machine, op("remove_provider", json!({"id": "spare"})));

    assert_eq!(response["ok"], json!(true), "{response}");
    assert!(
        !machine.store().join("keys").join("spare").exists(),
        "the stored key went with its provider"
    );
}
