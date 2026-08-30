//! The tapkey engine.
//!
//! Deliberately empty. This crate exists from the first commit so that CI compiles it —
//! including for Linux — rather than asserting portability it never checks. The engine
//! itself is built one decision at a time; nothing is added here ahead of the decision
//! that shapes it.
//!
//! The shape it grows into is fixed: one function, a JSON request in and a JSON response
//! out, behind a versioned schema, with three consumers sharing it.

pub mod adapters;
pub mod atomic;
pub mod credentials;
pub mod env;
pub mod fingerprint;
pub mod fs;
pub mod helper;
pub mod instant;
pub mod json;
pub mod lock;
pub mod probe;
pub mod profile;
pub mod store;
pub mod toml;
pub mod transaction;
pub mod wire;

use env::{CredentialState, Env};
use wire::{Envelope, Failure, Request, Response};

/// The schema version carried by every request and response.
pub const SCHEMA_VERSION: u32 = 1;

/// The one entry point: a JSON request in, a JSON response out.
pub fn handle(request: &str) -> String {
    handle_with(&Env::real(), request)
}

/// The same call, told what world it is acting in rather than going and finding one.
///
/// This is the seam the tests and the fixture harness use. See ADR-0016: the public surface is
/// still one string in and one string out.
pub fn handle_with(env: &Env, request: &str) -> String {
    let response = dispatch(env, request);
    serde_json::to_string(&response).unwrap_or_else(|e| {
        // Serialising our own response cannot fail on well-formed data, but a panic here would
        // take the host application down, so it degrades to a refusal instead.
        format!(
            r#"{{"ok":false,"failure":{{"kind":"internal","detail":{:?}}}}}"#,
            e.to_string()
        )
    })
}

fn dispatch(env: &Env, request: &str) -> Response {
    let envelope: Envelope = match serde_json::from_str(request) {
        Ok(e) => e,
        Err(e) => return refuse("malformed_request", e.to_string()),
    };
    if envelope.version != SCHEMA_VERSION {
        return refuse(
            "unknown_version",
            format!("this core speaks version {SCHEMA_VERSION}"),
        );
    }
    match envelope.request {
        Request::EffectiveState {} => match adapters::effective_state(env) {
            Ok(tools) => Response::Ok {
                ok: true,
                outcome: None,
                tools,
                backup: None,
            },
            Err(e) => refuse("unparsable", e),
        },
        Request::Switch { profile_id } => switch(env, &profile_id),
        Request::Test { provider_id } => test(env, &provider_id),
        Request::Harvest {} => harvest(env),
        Request::ListProfiles {} => {
            let profiles = profile::Profiles::read(&env.store().join("profiles.json")).unwrap_or(
                profile::Profiles {
                    providers: Vec::new(),
                    profiles: Vec::new(),
                },
            );
            Response::Profiles {
                ok: true,
                profiles: profiles
                    .profiles
                    .iter()
                    .map(|p| wire::ProfileRow {
                        id: p.id.clone(),
                        name: p.name.clone(),
                        tools: p.tools.len(),
                        assignments: p.tools.clone(),
                    })
                    .collect(),
            }
        }
        Request::AcceptHarvest { tool, id } => accept_harvest(env, &tool, &id),
        Request::DeclineHarvest { tool, id } => decline_harvest(env, &tool, &id),
        Request::CreateProfile { profile } => write_profiles(env, |p| {
            if p.profiles.iter().any(|x| x.id == profile.id) {
                return Err((
                    "unknown_profile",
                    format!("a profile named {:?} exists", profile.id),
                ));
            }
            if profile.tools.is_empty() {
                // The same refusal a switch gives: an empty profile cannot be applied, so it
                // cannot be created either.
                return Err((
                    "unknown_profile",
                    "a profile has to name at least one tool".into(),
                ));
            }
            let id = profile.id.clone();
            p.profiles.push(profile);
            Ok(("profile", "created", id))
        }),
        Request::UpdateProfile { id, tools } => write_profiles(env, |p| {
            if tools.is_empty() {
                // The same refusal creation gives: an empty profile cannot be applied, so it
                // cannot be saved either — deleting is the honest name for that edit.
                return Err((
                    "unknown_profile",
                    "a profile has to name at least one tool".into(),
                ));
            }
            let profile = p
                .profiles
                .iter_mut()
                .find(|x| x.id == id)
                .ok_or_else(|| ("unknown_profile", format!("no profile named {id:?}")))?;
            profile.tools = tools;
            Ok(("profile", "updated", id))
        }),
        Request::RenameProfile { id, name } => write_profiles(env, |p| {
            let profile = p
                .profiles
                .iter_mut()
                .find(|x| x.id == id)
                .ok_or_else(|| ("unknown_profile", format!("no profile named {id:?}")))?;
            // The name is the only thing that moves. The id is a reference held in the store and
            // in every fingerprint taken under it; making it mutable would oblige a rename to
            // walk through everything that mentions it.
            profile.name = name;
            Ok(("profile", "renamed", id))
        }),
        Request::DuplicateProfile { id, as_id } => write_profiles(env, |p| {
            let source = p
                .profiles
                .iter()
                .find(|x| x.id == id)
                .ok_or_else(|| ("unknown_profile", format!("no profile named {id:?}")))?;
            // Everything comes across, per-slot providers included: the catalogue's hint for
            // duplicate is *same provider, different model*, and a copy that lost assignments
            // would make it a lie.
            let mut copy = source.clone();
            copy.id = as_id.clone();
            copy.name = format!("{} (copy)", source.name);
            if p.profiles.iter().any(|x| x.id == as_id) {
                return Err((
                    "unknown_profile",
                    format!("a profile named {as_id:?} exists"),
                ));
            }
            p.profiles.push(copy);
            Ok(("profile", "duplicated", as_id))
        }),
        Request::DeleteProfile { id } => write_profiles(env, |p| {
            // Deleting the profile the tools are currently on deletes it and changes no tool:
            // switching them to System default would be a large action in answer to a small one.
            // The tools keep what was applied, and drift still has its fingerprints.
            let before = p.profiles.len();
            p.profiles.retain(|x| x.id != id);
            if p.profiles.len() == before {
                return Err(("unknown_profile", format!("no profile named {id:?}")));
            }
            Ok(("profile", "deleted", id))
        }),
        Request::CreateProvider { id, name, base_url } => write_providers(env, |providers| {
            if providers.iter().any(|p| p.id == id) {
                return Err((
                    "unknown_provider",
                    format!("a provider named {id:?} exists"),
                ));
            }
            providers.push(profile::Provider {
                id: id.clone(),
                name,
                base_url,
                formats: None,
                enabled: true,
                models: Vec::new(),
                tested_at: None,
            });
            Ok(("provider", "created", id))
        }),
        Request::RenameProvider { id, name } => write_providers(env, |providers| {
            let provider = providers
                .iter_mut()
                .find(|p| p.id == id)
                .ok_or_else(|| ("unknown_provider", format!("no provider named {id:?}")))?;
            // The **name** is editable and the **id** is not. An id is a reference held in three
            // tools' registry entries, in profiles and in the credential store; keeping only the
            // visible thing mutable removes the whole class of rename hazards, including moving
            // the stored key.
            provider.name = name;
            Ok(("provider", "renamed", id))
        }),
        Request::SetProviderEnabled { id, enabled } => write_providers(env, |providers| {
            let provider = providers
                .iter_mut()
                .find(|p| p.id == id)
                .ok_or_else(|| ("unknown_provider", format!("no provider named {id:?}")))?;
            provider.enabled = enabled;
            Ok(("provider", "set_enabled", id))
        }),
        Request::RemoveProvider { id } => remove_provider(env, &id),
        Request::SetCredential {
            provider_id,
            secret,
        } => set_credential(env, &provider_id, secret.as_bytes()),
        Request::ListHistory {} => {
            let Ok(store) = store::Store::open(env.store()) else {
                return Response::History {
                    ok: true,
                    entries: Vec::new(),
                };
            };
            let mut entries = Vec::new();
            if store.has_snapshot() {
                // The snapshot's instant and file count come from its own manifest; it is
                // restorable by construction, and its name is the catalogue's, not a profile's.
                let (instant, files) = store.snapshot_summary().unwrap_or_default();
                entries.push(wire::HistoryRow {
                    kind: "snapshot",
                    id: "snapshot".into(),
                    name: "Snapshot before tapkey".into(),
                    instant,
                    restorable: true,
                    files,
                });
            }
            // The store already returns newest-first; the ordering is its contract.
            if let Ok(backups) = store.backups() {
                for backup in backups {
                    entries.push(wire::HistoryRow {
                        kind: "backup",
                        id: backup.id.as_str().to_owned(),
                        name: backup.profile.clone(),
                        instant: backup.instant.clone(),
                        restorable: backup.restorable,
                        files: store.backup_files(backup.id.as_str()),
                    });
                }
            }
            Response::History { ok: true, entries }
        }
        Request::ToolPresence {} => Response::Presence {
            ok: true,
            tools: adapters::all()
                .iter()
                .map(|adapter| wire::ToolPresence {
                    tool: adapter.name(),
                    installed: adapters::installed(adapter.as_ref()),
                    configured: adapter.configured(env),
                })
                .collect(),
        },
        Request::ListProviders {} => {
            let profiles = profile::Profiles::read(&env.store().join("profiles.json")).unwrap_or(
                profile::Profiles {
                    providers: Vec::new(),
                    profiles: Vec::new(),
                },
            );
            Response::Providers {
                ok: true,
                providers: profiles
                    .providers
                    .iter()
                    .map(|p| wire::ProviderCard {
                        id: p.id.clone(),
                        name: p.name.clone(),
                        base_url: p.base_url.clone(),
                        formats: p.formats.clone(),
                        enabled: p.enabled,
                        tested_at: p.tested_at.clone(),
                        models: p.models.clone(),
                    })
                    .collect(),
            }
        }
        Request::Discover { provider_id } => discover(env, &provider_id),
        Request::SetModelEnabled {
            provider_id,
            model,
            enabled,
        } => write_providers(env, |providers| {
            let provider = providers
                .iter_mut()
                .find(|p| p.id == provider_id)
                .ok_or_else(|| {
                    (
                        "unknown_provider",
                        format!("no provider named {provider_id:?}"),
                    )
                })?;
            let entry = provider
                .models
                .iter_mut()
                .find(|m| m.id == model)
                .ok_or_else(|| {
                    (
                        "unknown_provider",
                        format!("no model named {model:?} for {provider_id:?}"),
                    )
                })?;
            entry.enabled = enabled;
            Ok(("provider", "updated", provider_id.to_string()))
        }),
        Request::Restore { target } => restore(env, target),
    }
}

/// Apply a profile to every tool it covers, all or nothing.
fn switch(env: &Env, profile_id: &str) -> Response {
    let profiles = match profile::Profiles::read(&env.store().join("profiles.json")) {
        Ok(p) => p,
        Err(e) => return refuse("unknown_profile", e),
    };
    let Some(profile) = profiles.find(profile_id) else {
        return refuse(
            "unknown_profile",
            format!("no profile named {profile_id:?}"),
        );
    };
    if profile.tools.is_empty() {
        return refuse(
            "unknown_profile",
            "the profile covers no managed tool".into(),
        );
    }

    // A profile naming a provider the file does not hold is a broken instruction, not a condition
    // of the machine, and it is caught before anything is staged.
    let mut assignments = Vec::new();
    for (tool, assignment) in &profile.tools {
        let provider = match &assignment.provider {
            Some(id) => match profiles.provider(id) {
                Some(p) => Some(p),
                None => {
                    return refuse("unknown_provider", format!("no provider named {id:?}"));
                }
            },
            None => None,
        };
        // Probed for **presence** before anything is staged, and never for value. A config pointing
        // at a credential that is not there is a silent breakage — measured, the tool says nothing
        // and the endpoint answers 401, which the person reads as a fault of their provider — and
        // this is where refusing is cheaper than explaining. Absence and denial are distinguished,
        // because they lead to different sentences: *add a key* said to somebody whose key exists
        // and was withheld is the wrong sentence.
        if let Some(provider) = provider {
            match env.credentials().check(&provider.id) {
                CredentialState::Found => {}
                CredentialState::Absent => {
                    return refuse(
                        "credential_unavailable",
                        format!("no key is stored for {provider:?}"),
                    );
                }
                CredentialState::Denied => {
                    return refuse(
                        "keychain_denied",
                        "the keychain refused access — grant it and try again".into(),
                    );
                }
            }
        }
        assignments.push((tool.as_str(), assignment, provider));
    }

    // Planned before anything is staged, so a file we cannot parse is a refusal rather than a
    // rollback: nothing was touched. A tool that cannot participate contributes an attention and
    // no actions — it is left out of the transaction rather than allowed to cancel it, the same
    // reasoning that makes a gone tool a skip.
    let mut actions = Vec::new();
    let mut tool_of: std::collections::BTreeMap<std::path::PathBuf, &'static str> =
        std::collections::BTreeMap::new();
    let mut attentions: std::collections::BTreeMap<&str, Vec<wire::Attention>> =
        std::collections::BTreeMap::new();
    let adapters = adapters::all();
    for (tool, assignment, provider) in &assignments {
        // A profile naming a tool this core does not manage is not a reason to refuse the tools it
        // does; the row simply has nowhere to land.
        let Some(adapter) = adapters.iter().find(|a| a.name() == *tool) else {
            continue;
        };
        // A slot naming its own provider is an instruction only OpenCode can carry out. The check
        // lives here rather than in each adapter, because it is one rule and two implementations of
        // one rule eventually disagree; the *fact* it reads — can this tool do it — is the tool's.
        if !adapter.per_slot_providers() {
            for (slot, assigned) in &assignment.slots {
                if assigned.provider().is_some() {
                    attentions
                        .entry(adapter.name())
                        .or_default()
                        .push(wire::Attention {
                            kind: "slot_provider_ignored",
                            file: None,
                            key: Some(slot.clone()),
                        });
                }
            }
        }

        match adapter.plan_switch(env, assignment, *provider) {
            Ok((planned, attn)) => {
                for action in &planned {
                    tool_of.insert(action.path().clone(), adapter.name());
                }
                actions.extend(planned);
                if !attn.is_empty() {
                    attentions.insert(adapter.name(), attn);
                }
            }
            Err(e) => return refuse("unparsable", e),
        }
    }

    let store = match store::Store::open(env.store()) {
        Ok(s) => s,
        Err(e) => return refuse("permission_denied", e.to_string()),
    };
    // Held until this call returns. A read never takes it: blocking `effective_state` for the
    // duration of a write would blank the UI at the one moment it is most interesting.
    let _lock = match lock::Lock::acquire(env.store()) {
        Ok(l) => l,
        Err(lock::Busy(why)) => return refuse("busy", why),
    };
    let transaction = transaction::Transaction::new(actions);
    let mut disk = env.filesystem();
    let captured = match transaction.capture(&**disk, &tool_of) {
        Ok(c) => c,
        Err(e) => return refuse("permission_denied", e.to_string()),
    };

    // The floor under every restore goes down before the first change, over **every managed file**
    // rather than only the touched ones: full restoration is its job. With one tool the two sets
    // were the same and this read as an ordinary capture; with two they part, and a snapshot of
    // only the touched file would leave the other tool nothing to come back to.
    let snapshot_actions: Vec<transaction::Action> = adapters::managed_files(env)
        .keys()
        .map(|path| transaction::Action::Delete { path: path.clone() })
        .collect();
    let everything = transaction::Transaction::new(snapshot_actions);
    let all_managed = match everything.capture(&**disk, &adapters::managed_files(env)) {
        Ok(c) => c,
        Err(e) => return refuse("permission_denied", e.to_string()),
    };
    if !store.has_snapshot() && store.take_snapshot(&all_managed, env.now()).is_err() {
        return refuse(
            "permission_denied",
            "could not record the first-run snapshot".into(),
        );
    }
    if store
        .take_backup(&captured, &profile.name, env.now())
        .is_err()
    {
        return refuse("permission_denied", "could not record a backup".into());
    }
    // The newest entry is the one just written (ADR-0019: named by UTC instant, ordered by the
    // manifest; the instant is this run's, and the clock is the caller's). Named here so Undo can
    // restore exactly this backup without browsing the store.
    let backup_id = store::newest_backup(env.store());

    if let Err(rolled_back) = transaction.apply(&mut **disk) {
        return Response::Failed {
            ok: false,
            outcome: "rolled back",
            failure: Failure {
                kind: "write_failed",
                detail: format!(
                    "{}: {} — {} file(s) restored",
                    adapters::wire_path(&rolled_back.failed_at),
                    rolled_back.reason,
                    rolled_back.restored
                ),
            },
        };
    }

    // The read-back below goes to the real filesystem, so the borrow ends here.
    drop(disk);

    // Drift needs something to compare against, and it is written only after the change stuck.
    let mut owned = std::collections::BTreeMap::new();
    for (tool, assignment, _) in &assignments {
        let Some(adapter) = adapters.iter().find(|a| a.name() == *tool) else {
            continue;
        };
        owned.insert((*tool).to_string(), adapter.fingerprint(assignment));
    }
    let _ = fingerprint::State::write(&env.store().join("state.json"), &profile.name, owned);

    // Read back rather than reporting what was written: the invariant forbids reporting intent,
    // and reading back is the only way a project config or a shell export that beat us shows up.
    let mut tools = match adapters::effective_state(env) {
        Ok(tools) => tools,
        Err(e) => return refuse("unparsable", e),
    };
    for tool in tools.iter_mut() {
        if let Some(attn) = attentions.remove(tool.tool) {
            tool.attentions = attn;
        }
    }
    Response::Ok {
        ok: true,
        outcome: Some("applied"),
        tools,
        backup: backup_id,
    }
}

/// Go back to a stored state, transactionally and taking a backup of its own first.
fn restore(env: &Env, target: wire::RestoreTarget) -> Response {
    let store = match store::Store::open(env.store()) {
        Ok(s) => s,
        Err(e) => return refuse("permission_denied", e.to_string()),
    };
    let _lock = match lock::Lock::acquire(env.store()) {
        Ok(l) => l,
        Err(lock::Busy(why)) => return refuse("busy", why),
    };
    let which = match &target {
        wire::RestoreTarget::Snapshot => store::Target::Snapshot,
        wire::RestoreTarget::Backup { id } => store::Target::Backup(id.as_str().into()),
    };
    let plan = match store.restore_plan(which) {
        Ok(p) => p,
        // Kept and marked rather than deleted, so the refusal names it rather than hiding it.
        Err(e) => return refuse("backup_unreadable", e.to_string()),
    };

    let actions = plan
        .into_iter()
        .map(|action| match action {
            store::RestoreAction::Write { path, bytes, mode } => transaction::Action::Write {
                path,
                bytes,
                mode: mode.unwrap_or(0o600),
            },
            store::RestoreAction::Delete { path } => transaction::Action::Delete { path },
        })
        .collect();

    let transaction = transaction::Transaction::new(actions);
    let mut disk = env.filesystem();
    // A restore touches whatever the snapshot held, so which tool each file belongs to is read
    // back from the paths the adapters own rather than assumed.
    let tool_of = adapters::managed_files(env);
    let captured = match transaction.capture(&**disk, &tool_of) {
        Ok(c) => c,
        Err(e) => return refuse("permission_denied", e.to_string()),
    };
    // Restoring changes the user's files like any other change, so it is backed up like one —
    // otherwise the single action with no way back is the way back itself.
    if store.take_backup(&captured, "restore", env.now()).is_err() {
        return refuse("permission_denied", "could not record a backup".into());
    }
    if let Err(rolled_back) = transaction.apply(&mut **disk) {
        return Response::Failed {
            ok: false,
            outcome: "rolled back",
            failure: Failure {
                kind: "write_failed",
                detail: format!(
                    "{}: {}",
                    adapters::wire_path(&rolled_back.failed_at),
                    rolled_back.reason
                ),
            },
        };
    }
    drop(disk);

    // Going back to the snapshot hands the tool to its own login, so tapkey owns nothing after
    // it — or drift would fire on values that are no longer ours.
    if matches!(target, wire::RestoreTarget::Snapshot) {
        let _ = fingerprint::State::write(
            &env.store().join("state.json"),
            "",
            std::collections::BTreeMap::new(),
        );
    }

    match adapters::effective_state(env) {
        Ok(tools) => Response::Ok {
            ok: true,
            outcome: Some("applied"),
            tools,
            backup: None,
        },
        Err(e) => refuse("unparsable", e),
    }
}

fn refuse(kind: &'static str, detail: String) -> Response {
    Response::Refused {
        ok: false,
        failure: Failure { kind, detail },
    }
}

/// Establish which formats a provider answers, and record it.
///
/// A write to the store, so it takes the lock — ticket 29's rule is *any write to the store*, not
/// a list of operations, which is how the next writing operation would otherwise end up outside
/// it. The lock is taken **after** the probes, because a network round-trip while holding the
/// lock would block a switch behind somebody's slow endpoint for no reason: the probes change
/// nothing, and only the write needs mutual exclusion.
fn test(env: &Env, provider_id: &str) -> Response {
    let store_path = env.store().join("profiles.json");
    let mut profiles = match profile::Profiles::read(&store_path) {
        Ok(p) => p,
        Err(e) => return refuse("unknown_provider", e),
    };
    let Some(record) = profiles.providers.iter_mut().find(|p| p.id == provider_id) else {
        return refuse(
            "unknown_provider",
            format!("no provider named {provider_id:?}"),
        );
    };

    let verdict = probe::run(record, env);
    let tested_at = instant::format_utc(env.now());

    let (knowable, formats, because) = match &verdict {
        probe::Verdict::Served(served) => (
            true,
            Some(
                served
                    .iter()
                    .map(|(format, served)| wire::FormatProbe {
                        format,
                        served: *served,
                    })
                    .collect::<Vec<_>>(),
            ),
            None,
        ),
        probe::Verdict::CannotTell(why) => (false, None, Some(*why)),
    };

    // Held only across the write.
    let _lock = match lock::Lock::acquire(env.store()) {
        Ok(l) => l,
        Err(lock::Busy(why)) => return refuse("busy", why),
    };
    if let probe::Verdict::Served(served) = &verdict {
        let answered: Vec<String> = served
            .iter()
            .filter(|(_, served)| *served)
            .map(|(format, _)| (*format).to_string())
            .collect();
        record.formats = Some(answered);
    }
    record.tested_at = Some(tested_at.clone());
    let bytes = serde_json::to_vec_pretty(&profiles).unwrap_or_else(|_| Vec::new());
    if let Err(e) = atomic::write_atomically(&store_path, &bytes, 0o600) {
        return refuse("permission_denied", e.to_string());
    }

    Response::Tested {
        ok: true,
        knowable,
        formats,
        because,
        provider: provider_id.to_string(),
        tested_at,
    }
}

/// The harvest offer: what the tools already know, minus everything secret.
///
/// Reads other people's files and changes nothing, so no lock. A declined candidate is still
/// listed, marked — the person can change their mind, and a list that hides refusals is a list
/// that curates itself.
fn harvest(env: &Env) -> Response {
    let declined = crate::profile::Profiles::declined(env.store());
    let mut candidates = Vec::new();
    for adapter in adapters::all() {
        for known in adapter.known_providers(env) {
            let existing = profile::Profiles::read(&env.store().join("profiles.json"))
                .ok()
                .and_then(|p| p.provider(&known.id).cloned());
            candidates.push(wire::Candidate {
                tool: adapter.name(),
                id: known.id.clone(),
                base_url: known.base_url,
                credential: match known.credential {
                    adapters::CredentialSource::Inline => "inline",
                    adapters::CredentialSource::Referenced => "reference",
                    adapters::CredentialSource::Absent => "absent",
                },
                name_conflict: existing.is_some(),
                declined: declined
                    .iter()
                    .any(|(tool, id)| tool == adapter.name() && id == &known.id),
            });
        }
    }

    // The profile of what the tools hold now, derived from the one implementation of effective
    // state rather than a second resolver.
    let suggested = match adapters::effective_state(env) {
        Ok(tools) if !tools.is_empty() => {
            let mut suggested_tools = Vec::new();
            for tool in &tools {
                let endpoint = tool.endpoint.effective.clone().unwrap_or_default();
                if endpoint.is_empty() {
                    continue;
                }
                let provider = endpoint_host(&endpoint);
                let slots: Vec<(&'static str, Option<String>)> = tool
                    .slots
                    .iter()
                    .filter(|s| s.slot == "main" || s.slot == "utility")
                    .map(|s| (s.slot, s.resolved.effective.clone()))
                    .collect();
                suggested_tools.push(wire::SuggestedTool {
                    tool: tool.tool,
                    provider,
                    slots,
                });
            }
            (!suggested_tools.is_empty()).then(|| wire::SuggestedProfile {
                name: "As configured now".to_string(),
                tools: suggested_tools,
            })
        }
        _ => None,
    };

    Response::Harvested {
        ok: true,
        candidates,
        suggested_profile: suggested,
    }
}

/// The host of a URL, used as a provider name when a tool names none of its own.
fn endpoint_host(url: &str) -> String {
    url.split("://")
        .nth(1)
        .and_then(|rest| rest.split('/').next())
        .unwrap_or(url)
        .to_string()
}

/// Take one candidate.
///
/// The key is **re-read from the tool's file at this moment** and piped to the helper on stdin: it
/// lives in one buffer for one call, and never enters a response, a log, or anything outliving the
/// operation. What we store is what is there now, which is also the more correct answer if the key
/// changed since the offer was made.
/// The OpenAI-compatible catalogue: GET `{base}/models`, answered with `{"data":[{"id":…}]}`,
/// often carrying `context_length` in tokens. Enabling on arrival: the person asked for the
/// catalogue, and the trimming is the editing the models group exists for.
fn discover(env: &Env, provider_id: &str) -> Response {
    let provider = match read_providers(env)
        .providers
        .into_iter()
        .find(|p| p.id == provider_id)
    {
        Some(p) => p,
        None => {
            return refuse(
                "unknown_provider",
                format!("no provider named {provider_id:?}"),
            );
        }
    };
    // The join is ours, not a normalisation of theirs: the trailing slash is trimmed the way
    // Test trims it, and `models` is the one path the convention agrees on.
    let base = provider.base_url.trim_end_matches('/');
    // The catalogue usually sits behind the key the person already stored. Presence is
    // probed first, as a switch probes it; the value travels buffer → header → request.
    // The two families measure differently, and the provider's own formats say which it is:
    // Anthropic-compatible hosts answer `/v1/models` under `x-api-key`; OpenAI-compatible
    // ones answer `/models` under `Bearer`, with the base already carrying its `/v1`.
    // The two joins, most likely first for this provider. A Test has not run on an imported
    // provider, so the family is unknown and both are tried — a body that parses as a
    // catalogue wins, and a 404-shaped answer is not a failure, just the wrong door.
    let anthropic_first = provider
        .formats
        .as_ref()
        .is_some_and(|f| f.iter().any(|x| x == "anthropic_messages"));
    let joins: Vec<(&str, String)> = if anthropic_first {
        vec![("x-api-key", format!("{base}/v1/models"))]
    } else if provider.formats.is_some() {
        vec![("Authorization", format!("{base}/models"))]
    } else {
        vec![
            ("Authorization", format!("{base}/models")),
            ("x-api-key", format!("{base}/v1/models")),
        ]
    };
    let key = match env.credentials().check(provider_id) {
        crate::env::CredentialState::Found => crate::credentials::read_stored(env, provider_id),
        _ => None,
    };
    // The last body that answered at all: a network death on the first door still leaves the
    // second worth trying, while two deaths are the host's, not ours.
    let mut last: Option<String> = None;
    let mut body = None;
    for (header, url) in &joins {
        let header_value = key.as_ref().map(|k| {
            if *header == "x-api-key" {
                k.clone()
            } else {
                format!("Bearer {k}")
            }
        });
        match env
            .http()
            .get_with_header(url, header, header_value.as_deref())
        {
            Ok(text) => {
                let parsed: Option<serde_json::Value> = serde_json::from_str(&text).ok();
                if parsed
                    .as_ref()
                    .and_then(|v| v.get("data"))
                    .and_then(|d| d.as_array())
                    .is_some_and(|a| !a.is_empty())
                {
                    body = parsed;
                    break;
                }
                last = Some(text);
            }
            Err(_) => continue,
        }
    }
    let Some(parsed) = body else {
        return if last.is_some() {
            refuse(
                "no_catalogue",
                "no catalogue here — enter models by hand".into(),
            )
        } else {
            refuse("network_unreachable", "the host did not answer".into())
        };
    };
    let Some(items) = parsed.get("data").and_then(|d| d.as_array()) else {
        return refuse(
            "no_catalogue",
            "no catalogue here — enter models by hand".into(),
        );
    };
    let models: Vec<profile::ProviderModel> = items
        .iter()
        .filter_map(|item| {
            let id = item.get("id")?.as_str()?.to_string();
            // Published in tokens, held in thousands; absent stays absent — OpenCode's
            // remaining-context display wants the fact, not a zero.
            let context_k = item
                .get("context_length")
                .and_then(|v| v.as_u64())
                .map(|tokens| (tokens / 1000) as u32);
            Some(profile::ProviderModel {
                id,
                context_k,
                enabled: true,
            })
        })
        .collect();
    if models.is_empty() {
        return refuse(
            "no_catalogue",
            "no catalogue here — enter models by hand".into(),
        );
    }
    let count = models.len();
    let _lock = match lock::Lock::acquire(env.store()) {
        Ok(lock) => lock,
        Err(lock::Busy(why)) => return refuse("busy", why),
    };
    let mut all = read_providers(env);
    let stored = all
        .providers
        .iter_mut()
        .find(|p| p.id == provider_id)
        .expect("read moments ago");
    stored.models = models;
    if let Err(e) = write_providers_atomic(env, &all) {
        return refuse("permission_denied", e);
    }
    Response::Discovered { ok: true, count }
}

fn read_providers(env: &Env) -> profile::Profiles {
    profile::Profiles::read(&env.store().join("profiles.json")).unwrap_or(profile::Profiles {
        providers: Vec::new(),
        profiles: Vec::new(),
    })
}

fn write_providers_atomic(env: &Env, profiles: &profile::Profiles) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(profiles).map_err(|e| e.to_string())?;
    atomic::write_atomically(&env.store().join("profiles.json"), &bytes, 0o600)
        .map_err(|e| e.to_string())
}

fn accept_harvest(env: &Env, tool: &str, id: &str) -> Response {
    // The store may not exist yet: adopting is often the first thing tapkey ever writes, and the
    // helper needs somewhere to put the key.
    let _ = std::fs::create_dir_all(env.store());
    let adapter = match adapters::all().into_iter().find(|a| a.name() == tool) {
        Some(a) => a,
        None => return refuse("unknown_provider", format!("no tool named {tool:?}")),
    };
    let Some(known) = adapter
        .known_providers(env)
        .into_iter()
        .find(|k| k.id == id)
    else {
        return refuse(
            "unknown_provider",
            format!("no candidate {id:?} from {tool:?}"),
        );
    };

    // The secret travels file → buffer → helper stdin, and stops.
    let secret = match known.credential {
        adapters::CredentialSource::Inline => {
            match crate::credentials::read_inline(env, tool, id) {
                Some(secret) => secret,
                None => {
                    return refuse(
                        "credential_unavailable",
                        format!("the key in {tool:?} for {id:?} could not be re-read"),
                    );
                }
            }
        }
        _ => Vec::new(),
    };
    if !secret.is_empty()
        && let Err(why) = crate::credentials::store(env, id, &secret)
    {
        return refuse("keychain_denied", why);
    }
    drop(secret);

    let _lock = match lock::Lock::acquire(env.store()) {
        Ok(l) => l,
        Err(lock::Busy(why)) => return refuse("busy", why),
    };
    let store_path = env.store().join("profiles.json");
    let mut profiles = profile::Profiles::read(&store_path).unwrap_or(profile::Profiles {
        providers: Vec::new(),
        profiles: Vec::new(),
    });
    profiles.providers.retain(|p| p.id != id);
    profiles.providers.push(profile::Provider {
        id: id.to_string(),
        name: id.to_string(),
        base_url: known.base_url,
        formats: None,
        enabled: true,
        models: Vec::new(),
        tested_at: None,
    });
    // The decline, if there was one, is undone by adopting.
    profile::Profiles::forget_decline(env.store(), tool, id);
    let bytes = serde_json::to_vec_pretty(&profiles).unwrap_or_default();
    if let Err(e) = atomic::write_atomically(&store_path, &bytes, 0o600) {
        return refuse("permission_denied", e.to_string());
    }
    drop(_lock);

    // The original stays in place by ADR-0015's rule — which, measured for Claude Code, means the
    // old key still outranks the helper until it is removed. That is the condition of the transfer
    // having happened, so it is said now rather than discovered at the first switch.
    let attentions = if known.credential == adapters::CredentialSource::Inline && tool == "claude" {
        vec![wire::Attention {
            kind: "credential_overrides_helper",
            file: Some(
                env.home()
                    .join(".claude")
                    .join("settings.json")
                    .display()
                    .to_string(),
            ),
            key: Some("ANTHROPIC_AUTH_TOKEN".to_string()),
        }]
    } else {
        Vec::new()
    };

    Response::Accepted {
        ok: true,
        provider: id.to_string(),
        attentions,
    }
}

/// A decline is a store write like any other, and reversible.
fn decline_harvest(env: &Env, tool: &str, id: &str) -> Response {
    let _ = std::fs::create_dir_all(env.store());
    let _lock = match lock::Lock::acquire(env.store()) {
        Ok(l) => l,
        Err(lock::Busy(why)) => return refuse("busy", why),
    };
    profile::Profiles::record_decline(env.store(), tool, id);
    Response::Accepted {
        ok: true,
        provider: id.to_string(),
        attentions: Vec::new(),
    }
}

/// Every profile operation is a store write and nothing more: a profile is our state, and no tool
/// file is touched. The lock is the rule ticket 29 fixed — **any write to the store** — taken here,
/// once, rather than re-derived per operation where the next writer would forget it.
fn write_profiles<F>(env: &Env, change: F) -> Response
where
    F: FnOnce(
        &mut profile::Profiles,
    ) -> Result<(&'static str, &'static str, String), (&'static str, String)>,
{
    write_store(env, |p| {
        change(p).map(
            |(what, action, id)| -> (bool, &'static str, &'static str, String) {
                (true, what, action, id)
            },
        )
    })
}

fn write_providers<F>(env: &Env, change: F) -> Response
where
    F: FnOnce(
        &mut Vec<profile::Provider>,
    ) -> Result<(&'static str, &'static str, String), (&'static str, String)>,
{
    write_profiles(env, move |p| change(&mut p.providers))
}

fn write_store<F>(env: &Env, change: F) -> Response
where
    F: FnOnce(
        &mut profile::Profiles,
    ) -> Result<(bool, &'static str, &'static str, String), (&'static str, String)>,
{
    let store_path = env.store().join("profiles.json");
    let _ = std::fs::create_dir_all(env.store());
    let _lock = match lock::Lock::acquire(env.store()) {
        Ok(l) => l,
        Err(lock::Busy(why)) => return refuse("busy", why),
    };
    let mut profiles = profile::Profiles::read(&store_path).unwrap_or(profile::Profiles {
        providers: Vec::new(),
        profiles: Vec::new(),
    });
    let (ok, what, action, id) = match change(&mut profiles) {
        Ok(result) => result,
        Err((kind, detail)) => return refuse(kind, detail),
    };
    let bytes = serde_json::to_vec_pretty(&profiles).unwrap_or_default();
    if let Err(e) = atomic::write_atomically(&store_path, &bytes, 0o600) {
        return refuse("permission_denied", e.to_string());
    }
    Response::Changed {
        ok,
        what,
        action,
        id,
    }
}

/// Removing a provider takes out the entries **tapkey created** — its namespaced registry entries —
/// and refuses while the provider is the current selection in any tool, because the alternative is
/// the broken tool ADR-0013 identified and deferred twice. The stored key is deleted through the
/// helper; a harvested original is never touched, and the response says so.
fn remove_provider(env: &Env, id: &str) -> Response {
    // The store may not exist on a machine that only ever harvested.
    let _ = std::fs::create_dir_all(env.store());
    let store_path = env.store().join("profiles.json");
    let mut profiles = profile::Profiles::read(&store_path).unwrap_or(profile::Profiles {
        providers: Vec::new(),
        profiles: Vec::new(),
    });
    let Some(provider) = profiles.providers.iter().find(|p| p.id == id).cloned() else {
        return refuse("unknown_provider", format!("no provider named {id:?}"));
    };

    // Planned before anything is staged: a refusal here changed nothing anywhere.
    let mut actions = Vec::new();
    for adapter in adapters::all() {
        match adapter.plan_removal(env, &provider) {
            Ok(planned) => actions.extend(planned),
            Err(using) => {
                return refuse(
                    "provider_in_use",
                    format!("{using} is using {id:?} right now — switch it first"),
                );
            }
        }
    }

    let _lock = match lock::Lock::acquire(env.store()) {
        Ok(l) => l,
        Err(lock::Busy(why)) => return refuse("busy", why),
    };
    let transaction = transaction::Transaction::new(actions);
    let mut disk = env.filesystem();
    let captured = match transaction.capture(&**disk, &adapters::managed_files(env)) {
        Ok(c) => c,
        Err(e) => return refuse("permission_denied", e.to_string()),
    };
    // A removal is a change tapkey makes, so it is backed up like one (ADR-0019).
    let store = match store::Store::open(env.store()) {
        Ok(s) => s,
        Err(e) => return refuse("permission_denied", e.to_string()),
    };
    if store
        .take_backup(&captured, "remove provider", env.now())
        .is_err()
    {
        return refuse("permission_denied", "could not record a backup".into());
    }
    if let Err(rolled_back) = transaction.apply(&mut **disk) {
        return Response::Failed {
            ok: false,
            outcome: "rolled back",
            failure: Failure {
                kind: "write_failed",
                detail: format!(
                    "{}: {} — {} file(s) restored",
                    adapters::wire_path(&rolled_back.failed_at),
                    rolled_back.reason,
                    rolled_back.restored
                ),
            },
        };
    }

    // The key we stored is deleted: a secret outliving its owner is a secret with no visible
    // owner. The harvested original stays where it is, and the UI says so.
    if let Err(why) = crate::credentials::forget(env, id) {
        return refuse("keychain_denied", why);
    }

    profiles.providers.retain(|p| p.id != id);
    let bytes = serde_json::to_vec_pretty(&profiles).unwrap_or_default();
    if let Err(e) = atomic::write_atomically(&store_path, &bytes, 0o600) {
        return refuse("permission_denied", e.to_string());
    }
    Response::Changed {
        ok: true,
        what: "provider",
        action: "removed",
        id: id.to_string(),
    }
}

/// Store a credential through the helper, the only writer of secrets. The wire call is
/// in-process, so the secret is one buffer handed to the helper's stdin — the same journey the
/// harvest takes, from a different beginning.
fn set_credential(env: &Env, provider_id: &str, secret: &[u8]) -> Response {
    if secret.is_empty() {
        return refuse("credential_unavailable", "the key is empty".into());
    }
    let _ = std::fs::create_dir_all(env.store());
    match crate::credentials::store(env, provider_id, secret) {
        Ok(()) => Response::Changed {
            ok: true,
            what: "provider",
            action: "credential_stored",
            id: provider_id.to_string(),
        },
        Err(why) => refuse("keychain_denied", why),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The public wrapper is what three consumers actually call, so it needs a test of its
    /// own — and one that refuses before touching the filesystem, or it would read whatever
    /// this machine happens to have in it.
    #[test]
    fn the_public_entry_point_answers_with_json_even_when_the_request_is_nonsense() {
        let out = handle("not json at all");
        let parsed: serde_json::Value = serde_json::from_str(&out).expect("a JSON response");
        assert_eq!(parsed["ok"], serde_json::json!(false));
        assert_eq!(
            parsed["failure"]["kind"],
            serde_json::json!("malformed_request")
        );
    }

    #[test]
    fn an_unknown_schema_version_is_refused_rather_than_guessed_at() {
        let env = Env::for_test("/nonexistent".into(), "/nonexistent".into());
        let out = handle_with(
            &env,
            r#"{"version": 99, "op": "effective_state", "params": {}}"#,
        );
        assert!(out.contains("unknown_version"), "{out}");
    }
}
