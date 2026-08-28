//! Fixtures come from real configuration files, which guarantees somebody will one day paste a
//! real one. The gap between pasting and committing is the only place that can still be caught,
//! so this is a test rather than a step in CI: it has to fail on the contributor's machine.

use std::path::{Path, PathBuf};

/// Shapes a real credential takes in these files. Deliberately broad — a false positive costs
/// somebody a rename, a false negative costs them a key.
const SHAPES: &[&str] = &["sk-ant-", "sk-or-", "sk-proj-", "ghp_", "AKIA"];

#[test]
fn no_fixture_carries_something_shaped_like_a_real_credential() {
    let mut found = Vec::new();
    for file in walk(&fixtures()) {
        let Ok(text) = std::fs::read_to_string(&file) else {
            continue;
        };
        for shape in SHAPES {
            if let Some(at) = text.find(shape) {
                let tail = &text[at + shape.len()..];
                let body: String = tail.chars().take_while(|c| c.is_alphanumeric()).collect();
                // The placeholders these files are meant to hold say so in capitals.
                if body.to_uppercase() == body && body.contains("PLACEHOLDER") {
                    continue;
                }
                found.push(format!("{}: {shape}…", file.display()));
            }
        }
        for run in text.split(|c: char| !c.is_alphanumeric() && c != '-' && c != '_') {
            if run.len() >= 40
                && run.chars().any(|c| c.is_ascii_digit())
                && !run.contains("PLACEHOLDER")
            {
                found.push(format!("{}: a {}-character run", file.display(), run.len()));
            }
        }
    }
    assert!(
        found.is_empty(),
        "possible credentials in fixtures:\n  {}",
        found.join("\n  ")
    );
}

/// A fixture tree with no cases in it is a harness that proves nothing, and the generator is
/// happy to produce no tests at all from an empty directory.
#[test]
fn the_fixture_tree_holds_cases() {
    let cases: usize = walk_dirs(&fixtures())
        .iter()
        .map(|t| walk_dirs(t).len())
        .sum();
    assert!(
        cases >= 10,
        "expected the named set of cases, found {cases}"
    );
}

/// Every case has to be able to say what it does. A directory with no request is a case nobody
/// finished writing, and the harness would silently treat it as one that expects nothing.
#[test]
fn every_case_carries_a_request() {
    for tool in walk_dirs(&fixtures()) {
        for case in walk_dirs(&tool) {
            assert!(
                case.join("request.json").exists(),
                "{} has no request.json",
                case.display()
            );
            assert!(
                case.join("before").is_dir(),
                "{} has no before/ tree",
                case.display()
            );
        }
    }
}

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn walk_dirs(root: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(root)
        .map(|entries| {
            entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.is_dir())
                .collect()
        })
        .unwrap_or_default()
}

fn walk(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(root) else {
        return out;
    };
    for entry in entries.flatten() {
        if entry.path().is_dir() {
            out.extend(walk(&entry.path()));
        } else {
            out.push(entry.path());
        }
    }
    out
}
