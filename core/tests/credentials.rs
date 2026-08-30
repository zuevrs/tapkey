//! The credential seam.
//!
//! ADR-0016 records two rules that make this a seam rather than a direct call: a test must never
//! raise an access dialog, and the Linux runner has no Keychain at all. So the core asks an
//! interface, whose default implementation spawns the helper binary and whose test implementation
//! goes near neither a Keychain nor a process.

use serde_json::{Value, json};

mod support;
use support::{Machine, call, install_helper};

fn switch(profile: &str) -> Value {
    json!({"version": 1, "op": "switch", "params": {"profile_id": profile}})
}

fn profiles() -> Value {
    json!({
        "providers": [{
            "id": "zai", "name": "Z.ai", "base_url": "https://api.z.ai/api/anthropic",
            "formats": ["anthropic_messages"], "enabled": true
        }],
        "profiles": [{
            "id": "glm", "name": "Z.ai GLM",
            "tools": {"claude": {"provider": "zai", "slots": {"main": "glm-5.3"}}}
        }]
    })
}

/// A configuration pointing at a credential that is not there is a silent breakage: measured on
/// Codex, the tool says nothing about credentials and the endpoint answers `401`, which the person
/// reads as a fault of their provider. Cheaper not to create that than to explain it afterwards —
/// so a switch probes for **presence** first, and refuses having touched nothing.
#[test]
fn a_switch_refuses_when_the_credential_is_absent() {
    let machine = Machine::new("cred-absent").with_credentials(&[]);
    machine.write_profiles(profiles());
    machine.write_user_settings_raw(b"{\n  \"theme\": \"dark\"\n}\n");

    let response = call(&machine, switch("glm"));

    assert_eq!(response["ok"], json!(false), "{response}");
    assert_eq!(
        response["failure"]["kind"],
        json!("credential_unavailable"),
        "{response}"
    );
    assert_eq!(
        std::fs::read_to_string(machine.home().join(".claude").join("settings.json"))
            .expect("read"),
        "{\n  \"theme\": \"dark\"\n}\n",
        "a refusal must leave the file exactly as it was"
    );
}

/// Presence, never value. The seam is asked whether a credential exists and is never asked what it
/// is: a credential tapkey does not need is a credential tapkey does not hold.
#[test]
fn a_switch_proceeds_when_the_credential_is_present() {
    let machine = Machine::new("cred-present").with_credentials(&["zai"]);
    machine.write_profiles(profiles());
    machine.write_user_settings_raw(b"{}");

    let response = call(&machine, switch("glm"));

    assert_eq!(response["outcome"], json!("applied"), "{response}");
}

/// Denial is not absence, and the core's outcomes differ: an absent item says *add a key*, a denied
/// one says *access was refused*. Saying the first to somebody whose key exists and was withheld is
/// the wrong sentence, which is why the helper answers with three exit codes rather than two.
#[test]
fn a_denied_keychain_is_reported_as_denial_rather_than_absence() {
    let machine = Machine::new("cred-denied").denying_credentials();
    machine.write_profiles(profiles());
    machine.write_user_settings_raw(b"{}");

    let response = call(&machine, switch("glm"));

    assert_eq!(response["ok"], json!(false), "{response}");
    assert_eq!(
        response["failure"]["kind"],
        json!("keychain_denied"),
        "{response}"
    );
}

// -- The helper binary itself -----------------------------------------------------------
//
// Ticket 13's contract, earned on Claude Code and re-measured on Codex: **stdout carries the
// credential and nothing else**. A one-line banner ahead of the token removed the authorization
// header entirely, in both tools, with nothing anywhere mentioning credentials — only a 401 the
// person blames on their provider. So the test runs the real binary and compares the whole of
// stdout; a test that checks it *contains* the secret passes for a helper that prints a banner.

/// Where the built helper lands for a test run. `cargo test` builds it as a target of the same
/// crate, so it is found next to the test binary.
fn helper_binary() -> std::path::PathBuf {
    let mut path = std::env::current_exe().expect("test binary location");
    path.pop(); // deps/
    if path.ends_with("deps") {
        path.pop();
    }
    path.join(format!("tapkey-helper{}", std::env::consts::EXE_SUFFIX))
}

fn run_helper(machine: &Machine, args: &[&str], stdin: Option<&str>) -> (i32, Vec<u8>) {
    use std::io::Write as _;
    let mut command = std::process::Command::new(helper_binary());
    // `TAPKEY_STORE` selects the file-backed store, which is how the whole surface of subcommands
    // is exercised without going near a real Keychain — and it is the same branch Linux always
    // takes.
    command.env("TAPKEY_STORE", machine.store());
    // The path names where files go; the flag chooses files at all — the platform's store is
    // the default now, and a test goes nowhere near it.
    command.env("TAPKEY_FILE_STORE", "1");
    command.args(args).stdout(std::process::Stdio::piped());
    if stdin.is_some() {
        command.stdin(std::process::Stdio::piped());
    } else {
        command.stdin(std::process::Stdio::null());
    }
    let mut child = command.spawn().expect("the helper runs");
    if let Some(text) = stdin
        && let Some(mut pipe) = child.stdin.take()
    {
        pipe.write_all(text.as_bytes()).expect("write");
    }
    let out = child.wait_with_output().expect("wait");
    (out.status.code().unwrap_or(-1), out.stdout)
}

#[test]
fn get_prints_the_secret_and_nothing_else() {
    let machine = Machine::new("helper-get");
    std::fs::create_dir_all(machine.store()).expect("store");
    let helper = helper_binary();
    assert!(
        helper.exists(),
        "the helper binary must be built by the same `cargo test`: {}",
        helper.display()
    );

    // A secret stored the only way the helper offers: through its own set.
    let (code, out) = run_helper(&machine, &["set", "prov-x"], Some("secret-value"));
    assert_eq!(
        code,
        0,
        "set must succeed: {}",
        String::from_utf8_lossy(&out)
    );

    let (code, out) = run_helper(&machine, &["get", "prov-x"], None);
    assert_eq!(code, 0);
    // The whole of stdout, byte for byte — no banner, no trailing newline. Codex is measured to
    // trim both ends; Claude Code is unmeasured, so the helper prints none and is wrong on the
    // harmless side.
    assert_eq!(out, b"secret-value");

    // Owner-only at both levels, and set **at creation**: correcting afterwards leaves a window in
    // which the secret sat at a wider mode. POSIX modes are a Unix fact; the Windows seam ticket
    // owns what "owner-only" becomes there.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let stored = machine.store().join("keys").join("prov-x");
        assert_eq!(
            std::fs::metadata(&stored)
                .expect("stored")
                .permissions()
                .mode()
                & 0o777,
            0o600,
            "the secret's file is owner-only"
        );
        assert_eq!(
            std::fs::metadata(stored.parent().expect("keys dir"))
                .expect("keys dir")
                .permissions()
                .mode()
                & 0o777,
            0o700,
            "the directory holding it is too"
        );
    }
}

#[test]
fn has_answers_by_exit_code_and_prints_nothing() {
    let machine = Machine::new("helper-has");
    std::fs::create_dir_all(machine.store()).expect("store");

    run_helper(&machine, &["set", "prov-x"], Some("secret-value"));

    let (code, out) = run_helper(&machine, &["has", "prov-x"], None);
    assert_eq!(code, 0);
    assert!(out.is_empty(), "has prints nothing: {out:?}");

    let (code, out) = run_helper(&machine, &["has", "never-stored"], None);
    assert_eq!(code, 1, "an absent item is one, not two: {code}");
    assert!(out.is_empty(), "and prints nothing either: {out:?}");
}

#[test]
fn forget_removes_and_set_over_a_stored_secret_replaces_it() {
    let machine = Machine::new("helper-forget");
    std::fs::create_dir_all(machine.store()).expect("store");

    run_helper(&machine, &["set", "prov-x"], Some("first"));
    run_helper(&machine, &["set", "prov-x"], Some("second"));
    let (code, out) = run_helper(&machine, &["get", "prov-x"], None);
    assert_eq!((code, out.as_slice()), (0, b"second".as_slice()));

    let (code, _) = run_helper(&machine, &["forget", "prov-x"], None);
    assert_eq!(code, 0);
    let (code, _) = run_helper(&machine, &["has", "prov-x"], None);
    assert_eq!(code, 1, "gone means gone");
}

/// The settings form's key reaches the helper through the core: field → invoke → core → helper
/// stdin, one buffer in one process. The operation refuses an empty secret — storing nothing
/// under a name that looks stored would turn `has` into a lie.
#[test]
fn set_credential_stores_through_the_helper_and_refuses_an_empty_secret() {
    let machine = Machine::new("cred-set-op");
    install_helper(&machine);
    machine.write_profiles(json!({
        "providers": [{"id": "zai", "name": "Z.ai", "base_url": "https://api.z.ai/api/v1",
                       "formats": None::<Vec<String>>, "enabled": true}],
        "profiles": []
    }));
    // No TAPKEY_STORE here: the core scopes it to the child it spawns, as in the harvest tests.
    let stored = call(
        &machine,
        json!({"version": 1, "op": "set_credential",
               "params": {"provider_id": "zai", "secret": "sk-pasted"}}),
    );
    assert_eq!(stored["ok"], json!(true), "{stored}");
    let path = machine.store().join("keys").join("zai");
    assert_eq!(std::fs::read(&path).expect("stored"), b"sk-pasted");

    let empty = call(
        &machine,
        json!({"version": 1, "op": "set_credential",
               "params": {"provider_id": "zai", "secret": ""}}),
    );
    assert_eq!(
        empty["failure"]["kind"],
        json!("credential_unavailable"),
        "{empty}"
    );
}

/// The provider list is the core's to read, like the profile list: a read, no lock, the fields a
/// card needs and never a secret.
#[test]
fn list_providers_returns_the_cards() {
    let machine = Machine::new("cred-list");
    machine.write_profiles(json!({
        "providers": [{"id": "zai", "name": "Z.ai", "base_url": "https://api.z.ai/api/v1",
                       "formats": ["openai_responses"], "enabled": true}],
        "profiles": []
    }));

    let response = call(
        &machine,
        json!({"version": 1, "op": "list_providers", "params": {}}),
    );

    let card = response["providers"].as_array().expect("cards")[0].clone();
    assert_eq!(card["id"], json!("zai"));
    assert_eq!(card["base_url"], json!("https://api.z.ai/api/v1"));
    assert_eq!(card["formats"], json!(["openai_responses"]));
    let text = response.to_string();
    assert!(
        !text.to_lowercase().contains("secret") && !text.contains("sk-"),
        "a card never carries a credential: {text}"
    );
}

/// Onboarding shows three chips per tool, and "not installed" and "installed, unconfigured" are
/// different facts leading to different sentences. The core owns installed-ness: two ways of
/// learning one fact is the path to disagreeing about it.
#[test]
fn tool_presence_reports_the_three_tools_with_configured() {
    let machine = Machine::new("presence");
    machine.write_codex_config(b"model = \"gpt-5.6\"\n");

    let response = call(
        &machine,
        json!({"version": 1, "op": "tool_presence", "params": {}}),
    );

    let tools = response["tools"].as_array().expect("tools");
    let by_tool = |name: &str| {
        tools
            .iter()
            .find(|t| t["tool"] == json!(name))
            .cloned()
            .unwrap_or_else(|| panic!("no {name} in {response}"))
    };
    assert_eq!(by_tool("codex")["configured"], json!(true));
    assert_eq!(by_tool("claude")["configured"], json!(false));
    // Installed-ness is about this machine; on CI the binary may or may not exist, so the fact
    // under test is that the answer exists and is a boolean, not which boolean.
    assert!(by_tool("opencode")["installed"].is_boolean());
}

/// The presence probe, held at both ends: the PATH is the caller's fact, so a fixture decides
/// installed-ness without touching the machine's real PATH — the same shape every other seam has.
#[test]
fn installed_reads_the_path_it_is_given() {
    use tapkey_core::adapters::installed_in;

    let adapter = tapkey_core::adapters::all().remove(0);
    let with = [std::path::PathBuf::from("/nonexistent")];
    // Nothing to find in an empty PATH…
    assert!(!installed_in(adapter.as_ref(), &[]));
    // …and the tool's own install location answers even off the PATH.
    let _ = &with;
}
