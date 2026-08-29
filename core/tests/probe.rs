//! What a Test does: establish which wire formats an endpoint answers, by status alone.
//!
//! Measured against real providers before any of this existed: an absent path answers 404 and a
//! present one answers 401 with no credential at all — OpenAI on both of its shapes, Anthropic on
//! `/v1/messages`, OpenRouter the same. And the trap: a gateway that authenticates **before
//! routing** answers 401 to everything, so the probe carries a **control** on a path that cannot
//! exist, and when the control answers too, the honest result is *cannot tell*.

use serde_json::{Value, json};

mod support;
use support::{Machine, call};
use tapkey_core::env::{Http, ProbeStatus};

/// An HTTP seam that answers from a table, so a test states exactly what the wire said.
struct Scripted(Vec<(String, u16)>);

impl Http for Scripted {
    fn post(&self, url: &str) -> Result<ProbeStatus, tapkey_core::env::NetworkUnreachable> {
        match self.0.iter().find(|(pattern, _)| url.contains(pattern)) {
            Some((_, status)) => Ok(ProbeStatus::Answered(*status)),
            None => Ok(ProbeStatus::Answered(404)),
        }
    }
}

fn machine_with(name: &str, script: Vec<(String, u16)>) -> Machine {
    Machine::new(name).http(move || Box::new(Scripted(script.clone())))
}

fn test(provider: &str) -> Value {
    json!({"version": 1, "op": "test", "params": {"provider_id": provider}})
}

fn zai() -> Value {
    json!({
        "providers": [{
            "id": "zai", "name": "Z.ai", "base_url": "https://api.z.ai/api/v1",
            "enabled": true
        }],
        "profiles": []
    })
}

fn scripted(pairs: &[(&str, u16)]) -> Vec<(String, u16)> {
    pairs
        .iter()
        .map(|(pattern, status)| ((*pattern).to_string(), *status))
        .collect()
}

/// A Responses endpoint: `/responses` exists (401), `/chat/completions` does not (404), and the
/// control path answers 404 like any other absent path — which is what makes the run believable.
#[test]
fn a_test_tells_served_from_absent_by_status_alone() {
    let machine = machine_with(
        "probe-basic",
        scripted(&[
            ("responses", 401),
            ("chat/completions", 404),
            ("v1/messages", 404),
        ]),
    );
    machine.write_profiles(zai());

    let response = call(&machine, test("zai"));

    assert_eq!(response["ok"], json!(true), "{response}");
    let formats = response["formats"].as_array().expect("formats");
    let served = |name: &str| {
        formats
            .iter()
            .find(|f| f["format"] == json!(name))
            .expect("every canonical format is reported")["served"]
            .clone()
    };
    assert_eq!(served("openai_responses"), json!(true), "{response}");
    assert_eq!(served("openai_chat"), json!(false), "{response}");
    assert_eq!(served("anthropic_messages"), json!(false), "{response}");
    assert_eq!(response["knowable"], json!(true), "{response}");
}

/// The trap, and the reason for the control: a gateway that authenticates before routing answers
/// 401 to every path including ones that cannot exist. Reporting every format as served there
/// would be manufacturing an answer, so the honest verdict is *cannot tell* — and nothing is
/// written, because `None` and `Some([])` are different facts under ticket 14's rule.
#[test]
fn a_gateway_that_answers_everything_cannot_be_interrogated() {
    let machine = machine_with(
        "probe-control",
        // The control path answers 401 too: authenticate first, route later.
        scripted(&[("tapkey-probe", 401)]),
    );
    machine.write_profiles(zai());

    let response = call(&machine, test("zai"));

    assert_eq!(response["ok"], json!(true), "{response}");
    assert_eq!(response["knowable"], json!(false), "{response}");
    assert!(
        response["formats"].is_null(),
        "no verdict may be recorded when the method cannot tell: {response}"
    );
}

/// A network failure is not an empty set, and not an error either: the endpoint said nothing, so
/// the provider stays untested rather than being convicted of anything.
#[test]
fn no_answer_leaves_the_provider_untested() {
    // No seam at all: the machine's default is the offline state.
    let machine = Machine::new("probe-offline");
    machine.write_profiles(zai());

    let response = call(&machine, test("zai"));

    assert_eq!(response["ok"], json!(true), "{response}");
    assert_eq!(response["knowable"], json!(false), "{response}");
}
