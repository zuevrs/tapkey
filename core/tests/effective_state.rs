//! Tests for the one entry point, reading state rather than writing it.

use serde_json::{Value, json};

mod support;
use support::{Machine, call};
use tapkey_core::env::ShellVar;

#[test]
fn reports_the_endpoint_a_user_settings_file_sets() {
    let machine = Machine::new("es-user-endpoint");
    machine.write_user_settings(json!({
        "env": { "ANTHROPIC_BASE_URL": "https://api.a6api.com" }
    }));

    let response = call(
        &machine,
        json!({"version": 1, "op": "effective_state", "params": {}}),
    );

    assert_eq!(response["ok"], json!(true));
    let claude = tool(&response, "claude");
    assert_eq!(
        claude["endpoint"]["effective"],
        json!("https://api.a6api.com")
    );
}

fn tool<'a>(response: &'a Value, name: &str) -> &'a Value {
    response["tools"]
        .as_array()
        .expect("tools is an array")
        .iter()
        .find(|t| t["tool"] == name)
        .unwrap_or_else(|| panic!("no tool named {name} in {response}"))
}

#[test]
fn a_project_config_outranks_the_user_file_and_the_chain_names_both() {
    let machine = Machine::new("es-project-wins");
    machine.write_user_settings(json!({"env": {"ANTHROPIC_BASE_URL": "https://user.test"}}));
    machine.write_project_settings(json!({"env": {"ANTHROPIC_BASE_URL": "https://project.test"}}));

    let response = call(&machine, effective_state());

    let endpoint = &tool(&response, "claude")["endpoint"];
    assert_eq!(endpoint["effective"], json!("https://project.test"));
    assert_eq!(
        winners(endpoint),
        vec!["https://project.test"],
        "exactly one link may win"
    );
    assert!(
        values(endpoint).contains(&Some("https://user.test".to_string())),
        "the losing file stays in the chain: which file decided this is the question being asked"
    );
}

#[test]
fn project_local_outranks_the_shared_project_file() {
    let machine = Machine::new("es-local-wins");
    machine.write_project_settings(json!({"env": {"ANTHROPIC_BASE_URL": "https://shared.test"}}));
    machine
        .write_project_local_settings(json!({"env": {"ANTHROPIC_BASE_URL": "https://local.test"}}));

    let response = call(&machine, effective_state());

    assert_eq!(
        tool(&response, "claude")["endpoint"]["effective"],
        json!("https://local.test")
    );
}

#[test]
fn managed_settings_outrank_everything_on_disk() {
    let machine = Machine::new("es-managed-wins");
    machine.write_user_settings(json!({"env": {"ANTHROPIC_BASE_URL": "https://user.test"}}));
    machine
        .write_project_local_settings(json!({"env": {"ANTHROPIC_BASE_URL": "https://local.test"}}));
    machine.write_managed_settings(json!({"env": {"ANTHROPIC_BASE_URL": "https://managed.test"}}));

    let response = call(&machine, effective_state());

    assert_eq!(
        tool(&response, "claude")["endpoint"]["effective"],
        json!("https://managed.test")
    );
}

/// Measured against the tool: an `env` block in a settings file replaces the value inherited
/// from the shell, for credentials as well as for model selection.
#[test]
fn a_settings_file_beats_a_shell_export() {
    let machine = Machine::new("es-file-beats-shell").exporting(
        "ANTHROPIC_BASE_URL",
        ShellVar::Value("https://shell.test".into()),
    );
    machine.write_user_settings(json!({"env": {"ANTHROPIC_BASE_URL": "https://file.test"}}));

    let response = call(&machine, effective_state());

    let endpoint = &tool(&response, "claude")["endpoint"];
    assert_eq!(endpoint["effective"], json!("https://file.test"));
    assert!(
        values(endpoint).contains(&Some("https://shell.test".to_string())),
        "the export is shown, so somebody who typed it learns why they see another provider"
    );
}

#[test]
fn a_shell_export_wins_when_no_file_has_an_opinion() {
    let machine = Machine::new("es-shell-only").exporting(
        "ANTHROPIC_BASE_URL",
        ShellVar::Value("https://shell.test".into()),
    );

    let response = call(&machine, effective_state());

    assert_eq!(
        tool(&response, "claude")["endpoint"]["effective"],
        json!("https://shell.test")
    );
}

/// The shell is read by running a login shell, which is a different kind of knowledge from
/// reading a file — and a variable exported only on an interactive path is invisible to it.
#[test]
fn the_shell_link_says_how_it_was_obtained() {
    let machine = Machine::new("es-shell-provenance").exporting(
        "ANTHROPIC_BASE_URL",
        ShellVar::Value("https://shell.test".into()),
    );

    let response = call(&machine, effective_state());

    let link = link_with_scope(&tool(&response, "claude")["endpoint"], "shell");
    assert_eq!(link["source"], json!("login shell"));
}

/// A scope tapkey cannot observe gets an entry saying so rather than being left out: silence
/// reads as "nothing there", and these are places where something may well be.
#[test]
fn scopes_that_cannot_be_observed_are_named_rather_than_omitted() {
    let machine = Machine::new("es-unobservable");
    machine.write_user_settings(json!({"env": {"ANTHROPIC_BASE_URL": "https://user.test"}}));

    let response = call(&machine, effective_state());
    let endpoint = &tool(&response, "claude")["endpoint"];

    for scope in ["command line", "cloud session"] {
        let link = link_with_scope(endpoint, scope);
        assert_eq!(link["observable"], json!(false), "scope {scope}");
        assert_eq!(
            link["wins"],
            json!(false),
            "an unobservable scope cannot be said to win"
        );
    }
}

/// A duplicate key means no one can say which value the tool reads, so effective state is
/// refused rather than promised over a guess.
#[test]
fn a_duplicate_key_refuses_the_whole_read() {
    let machine = Machine::new("es-duplicate");
    machine.write_user_settings_raw(br#"{"env": {"A": "1"}, "env": {"A": "2"}}"#);

    let response = call(&machine, effective_state());

    assert_eq!(response["ok"], json!(false));
    assert_eq!(response["failure"]["kind"], json!("unparsable"));
}

// ---------------------------------------------------------------------------------------

fn effective_state() -> Value {
    json!({"version": 1, "op": "effective_state", "params": {}})
}

fn values(resolved: &Value) -> Vec<Option<String>> {
    resolved["chain"]
        .as_array()
        .expect("chain")
        .iter()
        .map(|l| l["value"].as_str().map(str::to_owned))
        .collect()
}

fn winners(resolved: &Value) -> Vec<String> {
    resolved["chain"]
        .as_array()
        .expect("chain")
        .iter()
        .filter(|l| l["wins"] == json!(true))
        .map(|l| l["value"].as_str().unwrap_or_default().to_string())
        .collect()
}

fn link_with_scope<'a>(resolved: &'a Value, scope: &str) -> &'a Value {
    resolved["chain"]
        .as_array()
        .expect("chain")
        .iter()
        .find(|l| l["scope"] == scope)
        .unwrap_or_else(|| panic!("no link for scope {scope} in {resolved}"))
}

/// The chain is documented as ordered by precedence, so its order is part of the contract —
/// a reader decides *which file decided this* by reading down it.
#[test]
fn the_chain_is_ordered_by_precedence_even_when_scopes_are_absent() {
    let machine = Machine::new("es-order");
    machine.write_user_settings(json!({"env": {"ANTHROPIC_BASE_URL": "https://user.test"}}));

    let response = call(&machine, effective_state());
    let scopes: Vec<String> = tool(&response, "claude")["endpoint"]["chain"]
        .as_array()
        .expect("chain")
        .iter()
        .map(|l| l["scope"].as_str().expect("scope").to_string())
        .collect();

    assert_eq!(
        scopes,
        vec!["command line", "user", "shell", "cloud session"],
        "no managed or project file exists here, and the rest must keep their order"
    );
}

/// A file that exists but says nothing must not be mistaken for a file that decided something.
/// Without this case, treating "was consulted" and "had a value" as the same condition passes
/// every other test in this file.
#[test]
fn a_file_that_exists_but_is_silent_does_not_win() {
    let machine = Machine::new("es-silent-file").exporting(
        "ANTHROPIC_BASE_URL",
        ShellVar::Value("https://shell.test".into()),
    );
    machine.write_user_settings(json!({"theme": "dark"}));

    let response = call(&machine, effective_state());
    let endpoint = &tool(&response, "claude")["endpoint"];

    assert_eq!(endpoint["effective"], json!("https://shell.test"));
    assert_eq!(link_with_scope(endpoint, "user")["wins"], json!(false));
    assert_eq!(link_with_scope(endpoint, "shell")["wins"], json!(true));
}

/// The source is what a person acts on — it has to name the file, in the form they would type.
#[test]
fn a_link_names_the_file_the_way_a_person_would_write_it() {
    let machine = Machine::new("es-source-path");
    machine.write_user_settings(json!({"env": {"ANTHROPIC_BASE_URL": "https://user.test"}}));

    let response = call(&machine, effective_state());

    assert_eq!(
        link_with_scope(&tool(&response, "claude")["endpoint"], "user")["source"],
        json!("~/.claude/settings.json")
    );
}
