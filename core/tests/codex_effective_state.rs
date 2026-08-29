//! What Codex will actually use, resolved across the layers it consults.
//!
//! Everything asserted here was measured against `codex-cli 0.150.1` in an isolated `CODEX_HOME`,
//! and two of the measurements contradicted the research note this adapter was designed from —
//! see the correction appended to `research/03-codex-config-surface.md`.

use serde_json::{Value, json};

mod support;
use support::{Machine, call};

fn effective_state() -> Value {
    json!({"version": 1, "op": "effective_state", "params": {}})
}

fn codex(response: &Value) -> Value {
    response["tools"]
        .as_array()
        .expect("tools")
        .iter()
        .find(|t| t["tool"] == json!("codex"))
        .cloned()
        .unwrap_or_else(|| panic!("no codex in {response}"))
}

#[test]
fn the_user_config_supplies_the_model_and_the_provider() {
    let machine = Machine::new("cx-user");
    machine.write_codex_config(
        b"model = \"gpt-5.6\"\nmodel_provider = \"mine\"\n\n[model_providers.mine]\nbase_url = \"https://e.invalid/v1\"\n",
    );

    let response = call(&machine, effective_state());

    let codex = codex(&response);
    assert_eq!(
        codex["endpoint"]["effective"],
        json!("https://e.invalid/v1")
    );
    let main = codex["slots"]
        .as_array()
        .expect("slots")
        .iter()
        .find(|s| s["slot"] == json!("main"))
        .expect("a main slot");
    assert_eq!(main["effective"], json!("gpt-5.6"), "{codex}");
}

/// Measured: a project `.codex/config.toml` is ignored **entirely and in silence** unless the
/// user's own config carries `[projects."<repo root>"] trust_level = "trusted"`. Codex prints
/// nothing at all. So the file has to appear in the chain — it exists, it is switched off, and the
/// gate is something the person can open. That is the line ticket 05's rule was sharpened along:
/// omit what the tool never consults for a key, show what is disabled by something changeable.
#[test]
fn an_untrusted_project_file_is_reported_rather_than_obeyed() {
    let machine = Machine::new("cx-untrusted");
    machine.write_codex_config(b"model = \"from-user\"\n");
    machine.write_codex_project_config(b"model = \"from-project\"\n");

    let response = call(&machine, effective_state());

    let codex = codex(&response);
    let main = codex["slots"]
        .as_array()
        .expect("slots")
        .iter()
        .find(|s| s["slot"] == json!("main"))
        .expect("a main slot");
    assert_eq!(
        main["effective"],
        json!("from-user"),
        "an untrusted project file must not win: {codex}"
    );
    let project = main["chain"]
        .as_array()
        .expect("chain")
        .iter()
        .find(|l| l["scope"] == json!("project"))
        .unwrap_or_else(|| panic!("the project file must appear in the chain: {main}"));
    assert_eq!(
        project["trusted"],
        json!(false),
        "and it must say why it lost: {project}"
    );
}

/// The same file, once the user's config trusts the project root, does win for `model`.
#[test]
fn a_trusted_project_file_wins_for_a_key_it_is_allowed_to_carry() {
    let machine = Machine::new("cx-trusted");
    machine.write_codex_config(b"model = \"from-user\"\n");
    machine.write_codex_project_config(b"model = \"from-project\"\n");
    machine.trust_codex_project();

    let response = call(&machine, effective_state());

    let main = codex(&response)["slots"]
        .as_array()
        .expect("slots")
        .iter()
        .find(|s| s["slot"] == json!("main"))
        .cloned()
        .expect("a main slot");
    assert_eq!(main["effective"], json!("from-project"), "{main}");
}

/// Twelve keys are **removed from** a project layer rather than losing a contest, so a denied key
/// cannot even shadow a lower one. `model_provider` is among them, which is why no project file can
/// move the endpoint by any route — and why the scope does not appear in that slot's chain at all.
#[test]
fn a_key_the_project_layer_may_not_carry_does_not_appear_in_its_chain() {
    let machine = Machine::new("cx-denylist");
    machine.write_codex_config(
        b"model_provider = \"mine\"\n\n[model_providers.mine]\nbase_url = \"https://user.invalid/v1\"\n",
    );
    machine.write_codex_project_config(
        b"model_provider = \"theirs\"\n\n[model_providers.theirs]\nbase_url = \"https://project.invalid/v1\"\n",
    );
    machine.trust_codex_project();

    let response = call(&machine, effective_state());

    let codex = codex(&response);
    assert_eq!(
        codex["endpoint"]["effective"],
        json!("https://user.invalid/v1"),
        "a trusted project file still cannot move the endpoint: {codex}"
    );
    assert!(
        !codex["endpoint"]["chain"]
            .as_array()
            .expect("chain")
            .iter()
            .any(|l| l["scope"] == json!("project")),
        "a scope the tool never consults for this key must not be shown as having lost: {codex}"
    );
}
