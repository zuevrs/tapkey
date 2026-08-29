//! Applying a profile to OpenCode.
//!
//! The stakes are higher here than in either previous adapter, and both reasons were measured. A
//! parse failure **echoes the whole config to stderr**, credential included, so a bad splice leaks
//! rather than merely breaking. And there is **no schema oracle at all** — no strict mode, no
//! warning tier, unknown keys silently ignored — so a key tapkey mistypes is accepted, ignored, and
//! reported as absent.

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
            "formats": ["openai_chat"],
            "enabled": true
        }],
        "profiles": [{
            "id": "glm",
            "name": "Z.ai GLM",
            "tools": {"opencode": {"provider": "zai", "slots": slots}}
        }]
    })
}

fn config(machine: &Machine) -> String {
    std::fs::read_to_string(
        machine
            .home()
            .join(".config")
            .join("opencode")
            .join("opencode.jsonc"),
    )
    .expect("read")
}

/// Model ids are namespaced `providerID/modelID`, so the selection and the registry entry are
/// written together — one without the other names nothing.
#[test]
fn a_switch_writes_a_namespaced_selection_and_its_registry_entry() {
    let machine = Machine::new("oc-sw-basic");
    machine.write_profiles(profiles(json!({"main": "glm-5.3"})));
    machine.write_opencode_config("opencode.jsonc", b"{\n  \"theme\": \"dark\"\n}\n");

    let response = call(&machine, switch("glm"));

    assert_eq!(response["outcome"], json!("applied"), "{response}");
    let after = config(&machine);
    assert!(
        after.contains("\"model\": \"tapkey-zai/glm-5.3\""),
        "{after}"
    );
    assert!(after.contains("\"tapkey-zai\""), "{after}");
    assert!(
        after.contains("\"baseURL\": \"https://api.z.ai/api/v1\""),
        "{after}"
    );
    assert!(after.contains("\"theme\": \"dark\""), "{after}");
}

/// The one key whose absence makes the tool rewrite the file on a plain read — stripping a BOM and
/// inserting an LF into a CRLF file. Measured: with it present, no write at all. It is the first
/// top-level key tapkey writes that no profile asked for, and its whole justification is that it
/// protects what we promised to preserve.
#[test]
fn the_schema_key_is_written_so_the_tool_stops_rewriting_the_file() {
    let machine = Machine::new("oc-sw-schema");
    machine.write_profiles(profiles(json!({"main": "glm-5.3"})));
    machine.write_opencode_config("opencode.jsonc", b"{\n  \"theme\": \"dark\"\n}\n");

    call(&machine, switch("glm"));

    assert!(
        config(&machine).contains("\"$schema\": \"https://opencode.ai/config.json\""),
        "{}",
        config(&machine)
    );
}

/// The credential is a **path**, never a key. When the config breaks, the tool prints it — so with
/// an inline key that output carries the credential, and with `{file:}` it carries a path. Measured
/// on both counts: the echo happens, and `{file:}` trims both ends so our file may end with a
/// newline like any other.
#[test]
fn the_credential_is_referenced_by_path_and_never_written_into_the_config() {
    let machine = Machine::new("oc-sw-credential");
    machine.write_profiles(profiles(json!({"main": "glm-5.3"})));
    machine.write_opencode_config("opencode.jsonc", b"{}\n");

    call(&machine, switch("glm"));

    let after = config(&machine);
    assert!(
        after.contains("\"apiKey\": \"{file:"),
        "the key must be referenced, not embedded: {after}"
    );
    assert!(
        !after.contains("secret") && !after.contains("sk-"),
        "nothing key-shaped belongs in this file: {after}"
    );
}

/// Comments, blank lines and a trailing comma are content here, and they survive.
#[test]
fn every_byte_tapkey_does_not_own_survives() {
    let machine = Machine::new("oc-sw-survive");
    machine.write_profiles(profiles(json!({"main": "glm-5.3"})));
    machine.write_opencode_config(
        "opencode.jsonc",
        b"{\n  // a note the person wrote\n  \"theme\": \"dark\",\n\n  /* and a block one */\n  \"autoupdate\": false,\n}\n",
    );

    call(&machine, switch("glm"));

    let after = config(&machine);
    assert!(after.contains("// a note the person wrote"), "{after}");
    assert!(after.contains("/* and a block one */"), "{after}");
    assert!(after.contains("\"autoupdate\": false"), "{after}");
}

/// Arrays replace rather than merge, so a provider tapkey adds is invisible while an existing
/// `enabled_providers` omits it. Appending carries out the switch somebody asked for; **creating**
/// the list would turn "no restriction" into "exactly one" and disable everything else.
#[test]
fn an_existing_enabled_providers_list_gains_our_id_and_an_absent_one_is_not_invented() {
    let with_list = Machine::new("oc-sw-enabled");
    with_list.write_profiles(profiles(json!({"main": "glm-5.3"})));
    with_list.write_opencode_config(
        "opencode.jsonc",
        b"{\n  \"enabled_providers\": [\"anthropic\"]\n}\n",
    );
    call(&with_list, switch("glm"));
    let after = config(&with_list);
    // The id appears in the provider map regardless, so asserting that it appears *somewhere*
    // would pass without the append ever happening — which is exactly what a defect injection
    // showed. The list itself is what has to hold it.
    assert!(
        after.contains("\"enabled_providers\": [\"anthropic\", \"tapkey-zai\"]"),
        "the list must gain our id without losing theirs: {after}"
    );

    let without = Machine::new("oc-sw-no-enabled");
    without.write_profiles(profiles(json!({"main": "glm-5.3"})));
    without.write_opencode_config("opencode.jsonc", b"{}\n");
    call(&without, switch("glm"));
    assert!(
        !config(&without).contains("enabled_providers"),
        "creating this list would disable every provider not in it: {}",
        config(&without)
    );
}
