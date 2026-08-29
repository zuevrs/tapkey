//! Test: establishing which wire formats an endpoint answers.
//!
//! The method was measured before it was built, against real providers. An absent path answers 404
//! and a present one answers 401, with no credential at all, so a format is established by probing
//! its own path and reading the **status** — never the body, never a real completion, which cost
//! tokens and a key. `/v1/models`, which ADR-0015 had built discovery on, cannot tell the two
//! OpenAI shapes apart and is therefore not a format test.
//!
//! The method carries a **control probe** on a path that cannot exist. A gateway that
//! authenticates before routing answers 401 to everything, and would report every format as
//! served; when the control answers too, the honest verdict is *cannot tell*, which leaves the
//! provider untested rather than convenient.
//!
//! The suffixes belong to the formats and copy what a tool of that shape appends. The base URL is
//! used verbatim — normalising it means repairing what we did not write — and the one slash this
//! code adds to join its own suffix is ours.

use crate::env::{Env, ProbeStatus};
use crate::profile::Provider;

/// The three canonical formats and the paths that answer for them.
pub const FORMATS: &[(&str, &str)] = &[
    ("anthropic_messages", "v1/messages"),
    ("openai_responses", "responses"),
    ("openai_chat", "chat/completions"),
];

/// A path that cannot exist, on any endpoint that answers by routes. Its job is to disagree.
const CONTROL: &str = "tapkey-probe-4x9";

/// What a Test established.
#[derive(Debug, PartialEq, Eq)]
pub enum Verdict {
    /// The control agreed to be absent, so the per-format answers mean something.
    Served(Vec<(&'static str, bool)>),
    /// The method cannot tell, and says why. The provider stays untested — which is `None`, not
    /// an empty set: *serves nothing we speak* is a verdict, *we could not ask* is a report.
    CannotTell(&'static str),
}

/// Ask the endpoint, once per format plus once for the control. Five requests, a few seconds.
pub fn run(provider: &Provider, env: &Env) -> Verdict {
    let base = provider.base_url.trim_end_matches('/');

    // The control first: if it cannot be absent, nothing after it is evidence.
    match env.http().post(&format!("{base}/{CONTROL}")) {
        Ok(ProbeStatus::Answered(404)) => {}
        Ok(ProbeStatus::Answered(_)) => return Verdict::CannotTell("every path answers"),
        Ok(ProbeStatus::NoAnswer) | Err(_) => return Verdict::CannotTell("no answer"),
    }

    let mut served = Vec::new();
    for (format, path) in FORMATS {
        match env.http().post(&format!("{base}/{path}")) {
            Ok(ProbeStatus::Answered(404)) => served.push((*format, false)),
            Ok(ProbeStatus::Answered(_)) => served.push((*format, true)),
            // Halfway through a run the network went away: an answer exists for some formats and
            // not others, and a half-verdict is worse than none.
            Ok(ProbeStatus::NoAnswer) | Err(_) => return Verdict::CannotTell("no answer"),
        }
    }
    Verdict::Served(served)
}
