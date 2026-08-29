//! The harvest offer, and what accepting it is allowed to touch.

use serde_json::{Value, json};

mod support;
use support::{Machine, call};

fn harvest() -> Value {
    json!({"version": 1, "op": "harvest", "params": {}})
}

/// A candidate is what the tool knows — id, endpoint, the *kind* of key seen — and never a value.
/// The secret's whole journey is file → buffer → helper stdin, once, at accept time.
#[test]
fn the_offer_names_candidates_and_never_a_secret() {
    let machine = Machine::new("hv-offer");
    machine.write_user_settings_raw(
        br#"{
  "env": {
    "ANTHROPIC_BASE_URL": "https://api.z.ai/api/anthropic",
    "ANTHROPIC_AUTH_TOKEN": "sk-harvest-me",
    "ANTHROPIC_MODEL": "glm-5.3"
  }
}
"#,
    );

    let response = call(&machine, harvest());

    assert_eq!(response["ok"], json!(true), "{response}");
    let candidates = response["candidates"].as_array().expect("candidates");
    let zai = candidates
        .iter()
        .find(|c| c["tool"] == json!("claude"))
        .expect("the configured endpoint is offered");
    assert_eq!(zai["id"], json!("api.z.ai"), "{zai}");
    assert_eq!(zai["credential"], json!("inline"), "{zai}");
    let text = response.to_string();
    assert!(
        !text.contains("sk-harvest-me"),
        "the offer must not carry the secret: {text}"
    );
}

/// A decline is remembered without being enforced in the dark: the candidate stays listed, marked,
/// because a list that hides refusals curates itself.
#[test]
fn a_decline_is_recorded_and_still_visible() {
    let machine = Machine::new("hv-decline");
    machine.write_user_settings_raw(
        br#"{"env": {"ANTHROPIC_BASE_URL": "https://api.z.ai/api/anthropic"}}"#,
    );

    call(
        &machine,
        json!({"version": 1, "op": "decline_harvest", "params": {"tool": "claude", "id": "api.z.ai"}}),
    );

    let response = call(&machine, harvest());
    let zai = response["candidates"]
        .as_array()
        .expect("candidates")
        .iter()
        .find(|c| c["tool"] == json!("claude"))
        .cloned()
        .expect("still offered");
    assert_eq!(zai["declined"], json!(true), "{zai}");
}

/// The app refreshes the helper into the store at startup; the test does the same, by copying the
/// binary cargo just built.
fn install_helper(machine: &Machine) {
    let mut path = std::env::current_exe().expect("test binary location");
    path.pop();
    if path.ends_with("deps") {
        path.pop();
    }
    let built = path.join("tapkey-helper");
    let dir = machine.store().join("bin");
    std::fs::create_dir_all(&dir).expect("bin dir");
    std::fs::copy(built, dir.join("tapkey-helper")).expect("copy the helper");
}

/// Accepting re-reads the key **now** and stores it through the helper; the provider record lands
/// with the id the tool used, unprefixed — the prefix marks entries *we* created, and this one was
/// the person's.
#[test]
fn accepting_stores_the_key_through_the_helper_and_adopts_the_id() {
    let machine = Machine::new("hv-accept");
    install_helper(&machine);
    machine.write_user_settings_raw(
        br#"{
  "env": {
    "ANTHROPIC_BASE_URL": "https://api.z.ai/api/anthropic",
    "ANTHROPIC_AUTH_TOKEN": "sk-harvest-me"
  }
}
"#,
    );

    // The core scopes `TAPKEY_STORE` to the child it spawns, so the helper writes into the
    // machine's store rather than the developer's real one.
    let response = call(
        &machine,
        json!({"version": 1, "op": "accept_harvest", "params": {"tool": "claude", "id": "api.z.ai"}}),
    );

    assert_eq!(response["ok"], json!(true), "{response}");
    // The stored secret went through the helper's file-backed store.
    let stored = machine.store().join("keys").join("api.z.ai");
    assert_eq!(
        std::fs::read(&stored).expect("stored"),
        b"sk-harvest-me",
        "the key was stored under the id the tool used"
    );

    // And the original stays in place — deleting somebody's credential store on first run is the
    // fastest way to lose them (ADR-0015).
    let settings = std::fs::read_to_string(machine.home().join(".claude").join("settings.json"))
        .expect("read");
    assert!(
        settings.contains("sk-harvest-me"),
        "the original is never touched"
    );
}

/// Measured: a token in Claude Code's `env` block outranks `apiKeyHelper`. Leaving the original in
/// place is ADR-0015's rule, which makes this attention the *condition of the transfer having
/// happened* — so it is raised at accept, not discovered at the first switch.
#[test]
fn accepting_a_key_that_still_wins_says_so_immediately() {
    let machine = Machine::new("hv-attention");
    install_helper(&machine);
    machine.write_user_settings_raw(
        br#"{"env": {"ANTHROPIC_BASE_URL": "https://api.z.ai/api/anthropic", "ANTHROPIC_AUTH_TOKEN": "sk-old"}}"#,
    );
    let response = call(
        &machine,
        json!({"version": 1, "op": "accept_harvest", "params": {"tool": "claude", "id": "api.z.ai"}}),
    );

    let attention = response["attentions"]
        .as_array()
        .and_then(|a| a.first().cloned())
        .expect("the still-winning key must be said out loud: {response}");
    assert_eq!(attention["kind"], json!("credential_overrides_helper"));
}
