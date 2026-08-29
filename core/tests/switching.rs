//! Applying a profile: what lands in the file, what survives, and what happens when it fails.

use serde_json::{Value, json};

mod support;
use support::{Machine, call};
use tapkey_core::env::ShellVar;

fn switch(profile: &str) -> Value {
    json!({"version": 1, "op": "switch", "params": {"profile_id": profile}})
}

fn zai() -> Value {
    json!({"providers": [{"id": "zai", "name": "Z.ai", "base_url": "https://api.z.ai/api/anthropic", "formats": ["anthropic_messages"], "enabled": true}, {"id": "openrouter", "name": "OpenRouter", "base_url": "https://openrouter.ai/api/v1", "formats": ["anthropic_messages"], "enabled": true}, {"id": "x", "name": "X", "base_url": "https://x.test", "formats": ["anthropic_messages"], "enabled": true}], "profiles": [{
        "id": "zai",
        "name": "Z.ai GLM",
        "tools": {"claude": {
            "provider": "zai",
            "slots": {"main": "glm-5.3", "utility": "glm-4.6-air"}
        }}
    }]})
}

fn user_settings(machine: &Machine) -> String {
    std::fs::read_to_string(machine.home().join(".claude").join("settings.json")).expect("read")
}

#[test]
fn a_switch_writes_the_endpoint_and_the_models_into_the_env_block() {
    let machine = Machine::new("sw-basic");
    machine.write_profiles(zai());
    machine.write_user_settings_raw(b"{\n  \"theme\": \"dark\"\n}\n");

    let response = call(&machine, switch("zai"));

    assert_eq!(response["outcome"], json!("applied"), "{response}");
    let after = user_settings(&machine);
    assert!(
        after.contains(r#""ANTHROPIC_BASE_URL": "https://api.z.ai/api/anthropic""#),
        "{after}"
    );
    assert!(after.contains(r#""ANTHROPIC_MODEL": "glm-5.3""#), "{after}");
    assert!(
        after.contains(r#""ANTHROPIC_DEFAULT_HAIKU_MODEL": "glm-4.6-air""#),
        "{after}"
    );
}

/// The test that matters most: everything tapkey does not own comes through unchanged, in a
/// file that already had an env block of its own with a credential in it.
#[test]
fn every_byte_tapkey_does_not_own_survives() {
    let before = b"{\n  \"env\": {\n    \"ANTHROPIC_AUTH_TOKEN\": \"sk-PLACEHOLDER-NOT-A-REAL-KEY\",\n    \"MY_OWN\": \"leave me\"\n  },\n  \"permissions\": {\n    \"allow\": [\"Bash(ls:*)\"]\n  },\n  \"theme\": \"dark\"\n}\n";
    let machine = Machine::new("sw-untouched");
    machine.write_profiles(zai());
    machine.write_user_settings_raw(before);

    call(&machine, switch("zai"));

    let after = user_settings(&machine);
    for kept in [
        r#""ANTHROPIC_AUTH_TOKEN": "sk-PLACEHOLDER-NOT-A-REAL-KEY""#,
        r#""MY_OWN": "leave me""#,
        "\"permissions\": {\n    \"allow\": [\"Bash(ls:*)\"]\n  }",
        r#""theme": "dark""#,
    ] {
        assert!(after.contains(kept), "lost {kept:?} from:\n{after}");
    }
}

#[test]
fn switching_twice_to_the_same_profile_leaves_the_file_byte_identical() {
    let machine = Machine::new("sw-idempotent");
    machine.write_profiles(zai());
    machine.write_user_settings_raw(b"{\n  \"theme\": \"dark\"\n}\n");

    call(&machine, switch("zai"));
    let once = user_settings(&machine);
    call(&machine, switch("zai"));

    assert_eq!(user_settings(&machine), once);
}

/// Measured: the deprecated variable still wins the background path. Setting only the current
/// one while a stale one survives sends background traffic to the old provider in silence.
#[test]
fn the_utility_slot_mirrors_into_the_deprecated_variable_when_it_is_present() {
    let machine = Machine::new("sw-mirror");
    machine.write_profiles(zai());
    machine.write_user_settings_raw(br#"{"env":{"ANTHROPIC_SMALL_FAST_MODEL":"old/model"}}"#);

    call(&machine, switch("zai"));

    let after = user_settings(&machine);
    assert!(
        after.contains(r#""ANTHROPIC_SMALL_FAST_MODEL":"glm-4.6-air""#),
        "{after}"
    );
    assert!(
        after.contains(r#""ANTHROPIC_DEFAULT_HAIKU_MODEL":"glm-4.6-air""#),
        "{after}"
    );
}

#[test]
fn the_deprecated_variable_is_not_created_where_it_was_absent() {
    let machine = Machine::new("sw-no-mirror");
    machine.write_profiles(zai());
    machine.write_user_settings_raw(b"{}");

    call(&machine, switch("zai"));

    assert!(
        !user_settings(&machine).contains("ANTHROPIC_SMALL_FAST_MODEL"),
        "an unconditional write leaves tapkey's fingerprints where nobody asked for them"
    );
}

/// A pin without its companion leaves the tool's own picker announcing Opus 5 over another
/// provider's model.
#[test]
fn a_pin_carries_its_display_name() {
    let machine = Machine::new("sw-pin");
    machine.write_profiles(json!({"providers": [{"id": "zai", "name": "Z.ai", "base_url": "https://api.z.ai/api/anthropic", "formats": ["anthropic_messages"], "enabled": true}, {"id": "openrouter", "name": "OpenRouter", "base_url": "https://openrouter.ai/api/v1", "formats": ["anthropic_messages"], "enabled": true}, {"id": "x", "name": "X", "base_url": "https://x.test", "formats": ["anthropic_messages"], "enabled": true}], "profiles": [{
        "id": "or", "name": "OpenRouter",
        "tools": {"claude": {"provider": "openrouter",
                             "slots": {"opus": "deepseek/deepseek-v3.2"}}}
    }]}));
    machine.write_user_settings_raw(b"{}");

    call(&machine, switch("or"));

    let after = user_settings(&machine);
    assert!(
        after.contains(r#""ANTHROPIC_DEFAULT_OPUS_MODEL":"deepseek/deepseek-v3.2""#),
        "{after}"
    );
    assert!(
        after.contains(r#""ANTHROPIC_DEFAULT_OPUS_MODEL_NAME":"deepseek/deepseek-v3.2""#),
        "{after}"
    );
}

#[test]
fn no_assignment_removes_the_key_tapkey_wrote() {
    let machine = Machine::new("sw-remove");
    machine.write_profiles(json!({"providers": [{"id": "zai", "name": "Z.ai", "base_url": "https://api.z.ai/api/anthropic", "formats": ["anthropic_messages"], "enabled": true}, {"id": "openrouter", "name": "OpenRouter", "base_url": "https://openrouter.ai/api/v1", "formats": ["anthropic_messages"], "enabled": true}, {"id": "x", "name": "X", "base_url": "https://x.test", "formats": ["anthropic_messages"], "enabled": true}], "profiles": [{
        "id": "bare", "name": "Bare",
        "tools": {"claude": {"provider": "x", "slots": {"main": null}}}
    }]}));
    machine.write_user_settings_raw(br#"{"env":{"ANTHROPIC_MODEL":"was-here"}}"#);

    call(&machine, switch("bare"));

    assert!(!user_settings(&machine).contains("ANTHROPIC_MODEL"));
}

/// ADR-0014: a settings file can set a variable and cannot unset one, so an inherited export
/// is neutralised with an empty value — but only when there is one, or tapkey leaves
/// fingerprints in a file it claims to have cleaned.
#[test]
fn no_assignment_writes_an_empty_value_only_when_a_shell_export_exists() {
    let profiles = json!({"providers": [{"id": "zai", "name": "Z.ai", "base_url": "https://api.z.ai/api/anthropic", "formats": ["anthropic_messages"], "enabled": true}, {"id": "openrouter", "name": "OpenRouter", "base_url": "https://openrouter.ai/api/v1", "formats": ["anthropic_messages"], "enabled": true}, {"id": "x", "name": "X", "base_url": "https://x.test", "formats": ["anthropic_messages"], "enabled": true}], "profiles": [{
        "id": "bare", "name": "Bare",
        "tools": {"claude": {"provider": "x", "slots": {"main": null}}}
    }]});

    let quiet = Machine::new("sw-empty-none");
    quiet.write_profiles(profiles.clone());
    quiet.write_user_settings_raw(b"{}");
    call(&quiet, switch("bare"));
    assert!(!user_settings(&quiet).contains("ANTHROPIC_MODEL"));

    let exported = Machine::new("sw-empty-shell")
        .exporting("ANTHROPIC_MODEL", ShellVar::Value("from-shell".into()));
    exported.write_profiles(profiles);
    exported.write_user_settings_raw(b"{}");
    call(&exported, switch("bare"));
    assert!(
        user_settings(&exported).contains(r#""ANTHROPIC_MODEL":"""#),
        "{}",
        user_settings(&exported)
    );
}

#[test]
fn the_first_switch_takes_a_snapshot_and_every_switch_takes_a_backup() {
    let machine = Machine::new("sw-store");
    machine.write_profiles(zai());
    machine.write_user_settings_raw(b"{\n  \"theme\": \"dark\"\n}\n");

    call(&machine, switch("zai"));

    assert!(
        machine
            .store()
            .join("snapshot")
            .join("manifest.json")
            .exists()
    );
    let backups: Vec<_> = std::fs::read_dir(machine.store().join("backups"))
        .expect("backups")
        .collect();
    assert_eq!(backups.len(), 1);
}

#[test]
fn an_unknown_profile_is_refused_and_changes_nothing() {
    let machine = Machine::new("sw-unknown");
    machine.write_profiles(zai());
    machine.write_user_settings_raw(b"{}");

    let response = call(&machine, switch("nope"));

    assert_eq!(response["ok"], json!(false));
    assert_eq!(response["failure"]["kind"], json!("unknown_profile"));
    assert_eq!(user_settings(&machine), "{}");
}

/// A file tapkey cannot parse is refused before anything is staged, so the report is a refusal
/// and not a rollback: nothing was touched.
#[test]
fn an_unparsable_settings_file_refuses_the_switch() {
    let machine = Machine::new("sw-unparsable");
    machine.write_profiles(zai());
    machine.write_user_settings_raw(b"{ not json");

    let response = call(&machine, switch("zai"));

    assert_eq!(response["ok"], json!(false));
    assert_eq!(response["failure"]["kind"], json!("unparsable"));
}

/// A failing write through the one entry point: the file is exactly as it was, and the outcome
/// says *rolled back* rather than *refused*, because a write was attempted.
///
/// What this does **not** prove is the multi-file half of ADR-0005. Claude Code's switch touches
/// one file, so there is no earlier file here to put back; that guarantee is proved at the
/// transaction's own seam, where three can be arranged. This case becomes the real thing when a
/// second adapter arrives, and saying so beats letting the name imply more than it shows.
#[test]
fn a_failing_write_rolls_back_and_says_so() {
    let before = b"{\n  \"theme\": \"dark\"\n}\n";
    let machine = Machine::new("sw-rollback").failing_after(0);
    machine.write_profiles(zai());
    machine.write_user_settings_raw(before);

    let response = call(&machine, switch("zai"));

    assert_eq!(response["ok"], json!(false), "{response}");
    assert_eq!(response["outcome"], json!("rolled back"));
    assert_eq!(
        user_settings(&machine).as_bytes(),
        before,
        "the file the switch failed on must be exactly as it was"
    );
}
