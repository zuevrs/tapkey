//! The CLI: argv marshalled into the one call (ADR-0016).
//!
//! No operation has CLI-specific code — the first argument names the operation, the second
//! carries its params as JSON, and the response is printed exactly as the core produced it.
//! A terminal and the menu bar app are the same client; anything this binary knows about
//! switching is one envelope older than the truth.
//!
//!   tapkey list_profiles
//!   tapkey switch '{"profile_id":"glm"}'
//!   tapkey state | jq
//!
//! Exit 0 when the core answered `ok`, 1 when it refused, 2 when the invocation itself was
//! wrong — the same three-way honesty the helper's exit codes chose.

use serde_json::{Value, json};

fn main() {
    let mut args = std::env::args_os().skip(1);
    let Some(op) = args.next() else {
        usage();
        std::process::exit(2);
    };
    let params: Value = match args.next() {
        Some(raw) => {
            let text = raw.to_string_lossy();
            match serde_json::from_str(&text) {
                Ok(value) => value,
                Err(e) => {
                    eprintln!("params are not JSON: {e}");
                    std::process::exit(2);
                }
            }
        }
        None => json!({}),
    };
    let request = json!({"version": 1, "op": op.to_string_lossy(), "params": params});
    let response = tapkey_core::handle_with(&tapkey_core::env::Env::real(), &request.to_string());
    println!("{response}");
    let ok = serde_json::from_str::<Value>(&response)
        .ok()
        .and_then(|v| v.get("ok").cloned())
        .is_some_and(|ok| ok == json!(true));
    std::process::exit(if ok { 0 } else { 1 });
}

fn usage() {
    eprintln!(
        "tapkey — switch which AI provider each coding tool talks to\n\
         \n\
         usage: tapkey <op> [params-as-json]\n\
         \n\
         the operations are the wire's: switch, restore, test, harvest, accept_harvest,\n\
         decline_harvest, effective_state, list_profiles, list_providers, list_history,\n\
         tool_presence, set_credential, create_profile, rename_profile, duplicate_profile,\n\
         delete_profile, create_provider, rename_provider, set_provider_enabled,\n\
         remove_provider\n\
         \n\
         examples:\n\
         \x20 tapkey list_profiles\n\
         \x20 tapkey switch '{{\"profile_id\":\"glm\"}}'\n\
         \x20 tapkey effective_state | jq '.tools[].endpoint'"
    );
}
