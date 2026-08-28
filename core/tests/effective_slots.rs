//! Model slots. Each one resolves differently, which is why the chain is computed per slot
//! rather than once per file.

use serde_json::{Value, json};

mod support;
use support::{Machine, call};

fn request() -> Value {
    json!({"version": 1, "op": "effective_state", "params": {}})
}

fn slot<'a>(response: &'a Value, name: &str) -> &'a Value {
    response["tools"]
        .as_array()
        .expect("tools")
        .iter()
        .find(|t| t["tool"] == "claude")
        .expect("claude")["slots"]
        .as_array()
        .expect("slots")
        .iter()
        .find(|s| s["slot"] == name)
        .unwrap_or_else(|| panic!("no slot {name} in {response}"))
}

#[test]
fn the_main_model_comes_from_the_environment_variable() {
    let machine = Machine::new("slots-main");
    machine.write_user_settings(json!({"env": {"ANTHROPIC_MODEL": "deepseek/v3.2"}}));

    let response = call(&machine, request());

    assert_eq!(slot(&response, "main")["effective"], json!("deepseek/v3.2"));
    assert_eq!(slot(&response, "main")["owned"], json!(true));
}

/// Measured behaviour, and the reason the main model is written into the block rather than
/// into the key: the variable outranks the key in every file, managed included.
#[test]
fn anthropic_model_outranks_the_model_key_even_in_managed_settings() {
    let machine = Machine::new("slots-var-beats-key");
    machine.write_managed_settings(json!({"model": "opus"}));
    machine.write_user_settings(json!({"env": {"ANTHROPIC_MODEL": "deepseek/v3.2"}}));

    let response = call(&machine, request());

    assert_eq!(slot(&response, "main")["effective"], json!("deepseek/v3.2"));
}

#[test]
fn the_model_key_decides_when_no_variable_is_set() {
    let machine = Machine::new("slots-key-only");
    machine.write_user_settings(json!({"model": "sonnet"}));

    let response = call(&machine, request());

    assert_eq!(slot(&response, "main")["effective"], json!("sonnet"));
}

/// `ANTHROPIC_DEFAULT_MODEL` applies only when nothing else selects a model, so it sits below
/// the key rather than above it — the opposite way round from `ANTHROPIC_MODEL`.
#[test]
fn anthropic_default_model_is_the_last_word_not_the_first() {
    let machine = Machine::new("slots-default-model");
    machine.write_user_settings(json!({
        "model": "sonnet",
        "env": {"ANTHROPIC_DEFAULT_MODEL": "haiku"}
    }));

    let response = call(&machine, request());

    assert_eq!(slot(&response, "main")["effective"], json!("sonnet"));
}

/// The deprecated variable was never demoted: it still wins on the background path, which is
/// what this slot is. Writing only the current one sends background traffic to the old model.
#[test]
fn the_deprecated_small_fast_variable_still_wins_the_utility_slot() {
    let machine = Machine::new("slots-small-fast");
    machine.write_user_settings(json!({"env": {
        "ANTHROPIC_SMALL_FAST_MODEL": "old/model",
        "ANTHROPIC_DEFAULT_HAIKU_MODEL": "new/model"
    }}));

    let response = call(&machine, request());

    assert_eq!(slot(&response, "utility")["effective"], json!("old/model"));
}

#[test]
fn the_utility_slot_falls_to_the_current_variable_when_the_deprecated_one_is_absent() {
    let machine = Machine::new("slots-haiku");
    machine.write_user_settings(json!({"env": {"ANTHROPIC_DEFAULT_HAIKU_MODEL": "new/model"}}));

    let response = call(&machine, request());

    assert_eq!(slot(&response, "utility")["effective"], json!("new/model"));
}

#[test]
fn the_subagent_and_pin_slots_are_read() {
    let machine = Machine::new("slots-rest");
    machine.write_user_settings(json!({"env": {
        "CLAUDE_CODE_SUBAGENT_MODEL": "sub/model",
        "ANTHROPIC_DEFAULT_OPUS_MODEL": "opus/model",
        "ANTHROPIC_DEFAULT_SONNET_MODEL": "sonnet/model",
        "ANTHROPIC_DEFAULT_FABLE_MODEL": "fable/model"
    }}));

    let response = call(&machine, request());

    assert_eq!(slot(&response, "subagent")["effective"], json!("sub/model"));
    assert_eq!(slot(&response, "opus")["effective"], json!("opus/model"));
    assert_eq!(
        slot(&response, "sonnet")["effective"],
        json!("sonnet/model")
    );
    assert_eq!(slot(&response, "fable")["effective"], json!("fable/model"));
}

/// Slots tapkey reports but never writes. Marking them owned would promise a switch changes
/// them, and the panel would then show an override that nothing maintains.
#[test]
fn advisor_fallback_and_model_overrides_are_observed_not_owned() {
    let machine = Machine::new("slots-observed");
    machine.write_user_settings(json!({"advisorModel": "fable"}));

    let response = call(&machine, request());

    assert_eq!(slot(&response, "advisor")["owned"], json!(false));
    assert_eq!(slot(&response, "advisor")["effective"], json!("fable"));
    assert_eq!(slot(&response, "fallback")["owned"], json!(false));
}

/// Each link says which key or variable it came from. Two links can name the same file and
/// mean different things, and "which file decided this" is only half the question.
#[test]
fn a_link_names_the_key_it_read_not_only_the_file() {
    let machine = Machine::new("slots-link-key");
    machine.write_user_settings(json!({"model": "sonnet"}));

    let response = call(&machine, request());
    let named: Vec<String> = slot(&response, "main")["chain"]
        .as_array()
        .expect("chain")
        .iter()
        .filter(|l| l["value"] == json!("sonnet"))
        .map(|l| l["key"].as_str().unwrap_or("").to_string())
        .collect();

    assert_eq!(named, vec!["model"]);
}

/// `/model` during a session outranks a command line flag, which outranks the variable. The
/// order was wrong when the command line was inserted rather than declared, and no test that
/// only looked at values could see it.
#[test]
fn the_main_slots_unobservable_scopes_are_in_the_right_order() {
    let machine = Machine::new("slots-order");
    machine.write_user_settings(json!({"env": {"ANTHROPIC_MODEL": "x"}}));

    let response = call(&machine, request());
    let scopes: Vec<String> = slot(&response, "main")["chain"]
        .as_array()
        .expect("chain")
        .iter()
        .map(|l| l["scope"].as_str().expect("scope").to_string())
        .collect();

    assert_eq!(
        scopes,
        vec!["session", "command line", "user", "shell", "user", "user", "shell", "cloud session"],
        "session above the command line, then ANTHROPIC_MODEL, then the model key, then \
         ANTHROPIC_DEFAULT_MODEL"
    );
}
