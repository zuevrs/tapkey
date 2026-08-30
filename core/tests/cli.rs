//! The CLI is one envelope thick, and these tests hold it to that: argv in, the core's
//! response out, exit codes honest. The binary runs against a temporary `$HOME` because
//! `Env::real()` honours it — the same isolation every other test in this suite stands on.

use serde_json::{Value, json};
use std::path::PathBuf;
use std::process::Command;

mod support;
use support::Machine;

fn cli_binary() -> PathBuf {
    let mut path = std::env::current_exe().expect("test binary location");
    path.pop(); // deps/
    if path.ends_with("deps") {
        path.pop();
    }
    path.join(format!("tapkey{}", std::env::consts::EXE_SUFFIX))
}

/// Where `Env::real()` puts the store under a given home — mirrored here because the test
/// must write the seed where the CLI's real env will read it, not where `Machine` keeps its
/// test-time store. `LOCALAPPDATA` is set to the home too, so Windows resolves beside it.
fn home_store(home: &std::path::Path) -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        home.join("Library")
            .join("Application Support")
            .join("tapkey")
    }
    #[cfg(windows)]
    {
        home.join("tapkey")
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        home.join(".local").join("share").join("tapkey")
    }
}

fn run(machine: &Machine, args: &[&str]) -> (i32, Value, String) {
    let output = Command::new(cli_binary())
        .args(args)
        .env("HOME", machine.home())
        .env("USERPROFILE", machine.home())
        .env("LOCALAPPDATA", machine.home())
        .output()
        .expect("the CLI runs");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let parsed = serde_json::from_str(&stdout).unwrap_or(Value::Null);
    (output.status.code().unwrap_or(-1), parsed, stdout)
}

#[test]
fn list_profiles_prints_the_cores_rows_and_exits_zero() {
    let machine = Machine::new("cli-list");
    let store = home_store(&machine.home());
    std::fs::create_dir_all(&store).expect("store dir");
    std::fs::write(
        store.join("profiles.json"),
        json!({
            "providers": [{"id": "zai", "name": "Z.ai", "base_url": "https://api.z.ai/api/anthropic",
                           "formats": ["anthropic_messages"], "enabled": true}],
            "profiles": [{"id": "glm", "name": "Z.ai GLM",
                          "tools": {"claude": {"provider": "zai", "slots": {}}}}]
        })
        .to_string(),
    )
    .expect("seed the store");

    let (code, response, _) = run(&machine, &["list_profiles"]);

    assert_eq!(code, 0);
    assert_eq!(
        response["profiles"][0]["name"],
        json!("Z.ai GLM"),
        "the rows are the core's, not the CLI's"
    );
}

/// A refusal is data, not a crash: the failure's JSON is printed and the exit code is 1 —
/// the same distinction the helper's three exit codes make.
#[test]
fn a_refusal_prints_the_failure_and_exits_one() {
    let machine = Machine::new("cli-refuse");
    machine.write_profiles(json!({"providers": [], "profiles": []}));

    let (code, response, _) = run(&machine, &["switch", r#"{"profile_id":"nope"}"#]);

    assert_eq!(code, 1, "{response}");
    assert_eq!(response["ok"], json!(false));
    assert!(response["failure"]["kind"].is_string(), "{response}");
}

/// Params travel as one JSON argument; malformed params are an invocation error, not a core
/// opinion — exit 2, and nothing on stdout that a script might mistake for an answer.
#[test]
fn malformed_params_are_an_invocation_error() {
    let machine = Machine::new("cli-badparams");

    let output = Command::new(cli_binary())
        .args(["switch", "{not json"])
        .env("HOME", machine.home())
        .output()
        .expect("the CLI runs");

    assert_eq!(output.status.code(), Some(2));
    assert!(
        output.stdout.is_empty(),
        "an invocation error is not an answer"
    );
    assert!(!String::from_utf8_lossy(&output.stderr).is_empty());
}

/// `state` reads through the same envelope: effective state against the temporary home's
/// config files, chains and all — proving the CLI marshals rather than knows.
#[test]
fn effective_state_reads_the_real_files_under_the_temporary_home() {
    let machine = Machine::new("cli-state");
    machine.write_user_settings_raw(br#"{"env":{"ANTHROPIC_BASE_URL":"https://cli.test"}}"#);

    let (code, response, _) = run(&machine, &["effective_state"]);

    assert_eq!(code, 0, "{response}");
    let endpoint = response["tools"]
        .as_array()
        .and_then(|tools| tools.first())
        .and_then(|t| t["endpoint"]["effective"].as_str());
    assert_eq!(endpoint, Some("https://cli.test"), "{response}");
}
