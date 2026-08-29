//! What OpenCode will actually use.
//!
//! Measured against 1.18.25 in an isolated config home. Two of these contradict the research the
//! adapter was designed from, and the corrections are recorded in ticket 26: the project layer
//! applies with **no gate at all**, and the model picker's state is a SQLite database rather than
//! the JSON file the note named — so it is reported as unreadable rather than guessed at.

use serde_json::{Value, json};

mod support;
use support::{Machine, call};

fn effective_state() -> Value {
    json!({"version": 1, "op": "effective_state", "params": {}})
}

fn opencode(response: &Value) -> Value {
    response["tools"]
        .as_array()
        .expect("tools")
        .iter()
        .find(|t| t["tool"] == json!("opencode"))
        .cloned()
        .unwrap_or_else(|| panic!("no opencode in {response}"))
}

fn slot(response: &Value, name: &str) -> Value {
    opencode(response)["slots"]
        .as_array()
        .expect("slots")
        .iter()
        .find(|s| s["slot"] == json!(name))
        .cloned()
        .unwrap_or_else(|| panic!("no {name} slot"))
}

/// All three global files are read and deep-merged, `.jsonc` highest — measured. An adapter that
/// read only `opencode.jsonc` would miss keys somebody keeps in `opencode.json`, and report their
/// absence confidently.
#[test]
fn the_three_global_files_are_merged_rather_than_chosen_between() {
    let machine = Machine::new("oc-merge");
    machine.write_opencode_config("opencode.jsonc", b"{\n  \"model\": \"from/jsonc\"\n}\n");
    machine.write_opencode_config("opencode.json", b"{\n  \"small_model\": \"from/json\"\n}\n");

    let response = call(&machine, effective_state());

    assert_eq!(slot(&response, "main")["effective"], json!("from/jsonc"));
    assert_eq!(slot(&response, "utility")["effective"], json!("from/json"));
}

/// And where two of them hold the same key, `.jsonc` wins and both appear in the chain — one entry
/// per file that actually had an opinion, because which file to edit is the actionable part.
#[test]
fn a_key_in_two_files_shows_both_and_the_higher_one_wins() {
    let machine = Machine::new("oc-both");
    machine.write_opencode_config("opencode.jsonc", b"{\n  \"model\": \"from/jsonc\"\n}\n");
    machine.write_opencode_config("opencode.json", b"{\n  \"model\": \"from/json\"\n}\n");

    let main = slot(&call(&machine, effective_state()), "main");

    assert_eq!(main["effective"], json!("from/jsonc"), "{main}");
    let sources: Vec<String> = main["chain"]
        .as_array()
        .expect("chain")
        .iter()
        .filter(|l| l["value"].is_string())
        .map(|l| l["source"].as_str().unwrap_or_default().to_string())
        .collect();
    assert_eq!(
        sources.len(),
        2,
        "both files had an opinion and both belong in the chain: {main}"
    );
}

/// The project layer applies with **no gate at all** — the opposite of Codex, where the same file is
/// ignored in silence until the user's own config trusts the repository root. So a repository
/// somebody cloned can redirect their requests from the moment OpenCode starts in it, and the chain
/// has to say where the value came from.
#[test]
fn a_project_file_wins_with_nothing_to_grant() {
    let machine = Machine::new("oc-project");
    machine.write_opencode_config("opencode.jsonc", b"{\n  \"model\": \"global/model\"\n}\n");
    machine.write_opencode_project_config(b"{\n  \"model\": \"project/model\"\n}\n");

    let main = slot(&call(&machine, effective_state()), "main");

    assert_eq!(main["effective"], json!("project/model"), "{main}");
    let winner = main["chain"]
        .as_array()
        .expect("chain")
        .iter()
        .find(|l| l["wins"] == json!(true))
        .cloned()
        .expect("something won");
    assert_eq!(winner["scope"], json!("project"), "{winner}");
}

/// Three scopes cannot be read from here, and each gets an explicit entry rather than silence:
/// saying nothing would claim knowledge we do not have, which is the failure the invariant names.
/// The picker is the sharpest of them — its choice lives in the tool's own SQLite database, so the
/// model in use may not be the model in any file.
#[test]
fn the_scopes_we_cannot_read_are_named_rather_than_omitted() {
    let machine = Machine::new("oc-unseen");
    machine.write_opencode_config("opencode.jsonc", b"{\n  \"model\": \"global/model\"\n}\n");

    let main = slot(&call(&machine, effective_state()), "main");

    let unreadable: Vec<String> = main["chain"]
        .as_array()
        .expect("chain")
        .iter()
        .filter(|l| l["observable"] == json!(false))
        .map(|l| l["scope"].as_str().unwrap_or_default().to_string())
        .collect();
    assert!(
        unreadable.contains(&"picker".to_string()),
        "the picker outranks every file and is not in one: {main}"
    );
    assert!(
        unreadable.contains(&"console".to_string()),
        "the network-fetched tier sits above even the project layer: {main}"
    );
}
