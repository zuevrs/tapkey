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
            // Git stores no empty directories, so a `before/` tree that holds only directories
            // exists on the machine that wrote it and nowhere else — the same shape as a fixture
            // that passes locally while not being in the repository at all. A case that needs a
            // file to be *absent* must still put some other file in the tree.
            assert!(
                walk_files(&case.join("before")).next().is_some(),
                "{}: before/ holds no files, so git cannot carry it",
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

/// A fixture that exists on disk and not in the repository is invisible: every test passes on
/// the machine that wrote it and the case arrives in CI with no input at all.
///
/// This is not hypothetical. The ignore rule keeping the agent's own `.claude/` directory out of
/// the repository was unanchored, so it matched `.claude` at any depth and silently swallowed
/// every `before/home/.claude/settings.json` in this tree.
#[test]
fn every_fixture_file_is_tracked_by_git() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let listing = std::process::Command::new("git")
        .arg("-C")
        .arg(&root)
        .args(["ls-files", "core/tests/fixtures"])
        .output();
    let Ok(listing) = listing else {
        // No git, no repository to check against — a source tarball, not a working copy.
        return;
    };
    if !listing.status.success() {
        return;
    }
    let tracked: Vec<String> = String::from_utf8_lossy(&listing.stdout)
        .lines()
        .map(|l| l.to_string())
        .collect();
    assert!(
        !tracked.is_empty(),
        "git reports no fixtures tracked at all"
    );

    let mut untracked = Vec::new();
    for file in walk(&fixtures()) {
        let relative = file
            .strip_prefix(root.canonicalize().unwrap_or(root.clone()))
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|_| file.to_string_lossy().into_owned());
        if !tracked.iter().any(|t| relative.ends_with(t.as_str())) {
            untracked.push(relative);
        }
    }
    assert!(
        untracked.is_empty(),
        "fixtures on disk but not in the repository:\n  {}",
        untracked.join("\n  ")
    );
}

/// Every file under `root`, however deep.
fn walk_files(root: &Path) -> Box<dyn Iterator<Item = PathBuf>> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Box::new(std::iter::empty());
    };
    Box::new(entries.flatten().flat_map(|e| {
        let path = e.path();
        if path.is_dir() {
            walk_files(&path)
        } else {
            Box::new(std::iter::once(path))
        }
    }))
}
