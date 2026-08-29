//! The golden fixture harness.
//!
//! A case is a self-contained directory: the real wire envelope, its own profiles, a mirrored
//! `before/` tree, and an `after/` tree that is always complete even when identical. Files are
//! compared byte for byte, because a file reads in a pull request diff and a snapshot blob does
//! not.
//!
//! Four properties run on **every** case, rather than in cases written specially for each: a
//! property that has to be remembered will be forgotten exactly where it mattered.
//!
//! Blessing: `UPDATE_GOLDEN=1 cargo test --test golden` rewrites the expectations and **still
//! fails**, so the diff has to be read and the run repeated. It never touches an input.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

mod support;
use support::TempDir;
use tapkey_core::env::Env;

// The generated names carry the case directory, which is where a failure has to point. Rust
// would rather they were snake case; legibility at the failure wins.
#[allow(non_snake_case)]
mod generated {
    use super::run_case;
    include!(concat!(env!("OUT_DIR"), "/golden_cases.rs"));
}

const CLOCK_MS: u64 = 1_787_866_640_123;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn blessing() -> bool {
    std::env::var_os("UPDATE_GOLDEN").is_some()
}

struct Case {
    name: String,
    dir: PathBuf,
    work: TempDir,
}

impl Case {
    fn home(&self) -> PathBuf {
        self.work.path().join("home")
    }

    fn store(&self) -> PathBuf {
        self.work.path().join("store")
    }

    fn env(&self) -> Env {
        let env = Env::for_test(self.home(), self.store())
            .with_clock(std::time::UNIX_EPOCH + std::time::Duration::from_millis(CLOCK_MS));
        match read_optional(&self.dir.join("fail.json")) {
            Some(bytes) => {
                let spec: serde_json::Value =
                    serde_json::from_slice(&bytes).expect("fail.json is JSON");
                let after = spec["after"].as_u64().expect("fail.json names `after`") as usize;
                env.with_filesystem(Box::new(tapkey_core::fs::FailOnce::after(after)))
            }
            None => env,
        }
    }

    fn request(&self) -> String {
        String::from_utf8(std::fs::read(self.dir.join("request.json")).expect("request.json"))
            .expect("request.json is UTF-8")
    }

    /// Lay the `before/` tree and the profiles into a fresh working directory.
    fn lay_out(&self) {
        copy_tree(&self.dir.join("before"), self.work.path());
        std::fs::create_dir_all(self.store()).expect("store");
        if let Some(profiles) = read_optional(&self.dir.join("profiles.json")) {
            std::fs::write(self.store().join("profiles.json"), profiles).expect("profiles");
        }
    }
}

fn run_case(tool: &str, name: &str) {
    let case = Case {
        name: format!("{tool}/{name}"),
        dir: fixtures().join(tool).join(name),
        work: TempDir::new(&format!("golden-{tool}-{name}")),
    };
    case.lay_out();

    let response = tapkey_core::handle_with(&case.env(), &case.request());
    let produced = collect(&case, &response);

    check_expectations(&case, &produced);
    check_properties(&case, &produced);
}

/// Everything the case produced, keyed by its path relative to the `after/` tree.
fn collect(case: &Case, response: &str) -> BTreeMap<String, Vec<u8>> {
    let mut out = BTreeMap::new();
    gather(case.work.path(), case.work.path(), &mut out, &["store"]);
    // The store is compared only where a case says so, by carrying an `after/store/` of its
    // own. Comparing it everywhere would put backup manifests in every diff, and one change to
    // that format would rewrite every fixture in the repository.
    if case.dir.join("after").join("store").is_dir() {
        gather(case.work.path(), case.work.path(), &mut out, &[]);
    }
    out.insert("response.json".into(), pretty(case, response));
    out
}

fn check_expectations(case: &Case, produced: &BTreeMap<String, Vec<u8>>) {
    let after = case.dir.join("after");
    let mut expected = BTreeMap::new();
    gather(&after, &after, &mut expected, &[]);

    if blessing() {
        bless(&after, produced);
        panic!(
            "{}: expectations rewritten. Read the diff, then run again — blessing and passing \
             are never the same keystroke.",
            case.name
        );
    }

    if expected.is_empty() {
        let dump = save_failure(case, produced);
        panic!(
            "{}: no expectations recorded. What the case produced is in {}. Review it, then \
             bless with UPDATE_GOLDEN=1 — a case must never be blessed by its first run.",
            case.name,
            dump.display()
        );
    }

    let mut problems = Vec::new();
    for (path, want) in &expected {
        match produced.get(path) {
            None => problems.push(format!("  missing: {path}")),
            Some(got) if got != want => problems.push(describe(path, want, got)),
            Some(_) => {}
        }
    }
    for path in produced.keys() {
        if !expected.contains_key(path) {
            problems.push(format!("  unexpected: {path}"));
        }
    }
    if !problems.is_empty() {
        let dump = save_failure(case, produced);
        panic!(
            "{}:\n{}\nactual tree written to {}",
            case.name,
            problems.join("\n"),
            dump.display()
        );
    }
}

/// A byte difference, rendered so the invisible parts are visible: those are exactly what the
/// splicer promises to preserve, and a plain diff shows them as no difference at all.
fn describe(path: &str, want: &[u8], got: &[u8]) -> String {
    let at = want
        .iter()
        .zip(got)
        .position(|(a, b)| a != b)
        .unwrap_or(want.len().min(got.len()));
    format!(
        "  {path}: first differs at byte {at}\n    expected: {}\n    actual:   {}",
        visible(want),
        visible(got)
    )
}

fn visible(bytes: &[u8]) -> String {
    let mut out = String::new();
    if bytes.starts_with(b"\xEF\xBB\xBF") {
        out.push_str("<BOM>");
    }
    for b in bytes {
        match b {
            b'\r' => out.push_str("<CR>"),
            b'\n' => out.push_str("<LF>\n              "),
            b'\t' => out.push_str("<TAB>"),
            _ => out.push(*b as char),
        }
    }
    if !bytes.ends_with(b"\n") {
        out.push_str("<no trailing newline>");
    }
    out
}

// --- the four properties -----------------------------------------------------------------

fn check_properties(case: &Case, produced: &BTreeMap<String, Vec<u8>>) {
    no_stray_files(case);
    idempotent(case, produced);
    untouched_bytes_survive(case, produced);
    restore_returns_the_original(case);
}

/// A failed or finished write must leave nothing behind in a destination directory.
fn no_stray_files(case: &Case) {
    let mut all = BTreeMap::new();
    gather(case.work.path(), case.work.path(), &mut all, &[]);
    let strays: Vec<&String> = all.keys().filter(|p| p.contains(".tapkey-")).collect();
    assert!(
        strays.is_empty(),
        "{}: temp files left behind: {strays:?}",
        case.name
    );
}

/// Applying the same request twice must not move a byte.
fn idempotent(case: &Case, first: &BTreeMap<String, Vec<u8>>) {
    let again = Case {
        name: case.name.clone(),
        dir: case.dir.clone(),
        work: TempDir::new("golden-idempotent"),
    };
    again.lay_out();
    tapkey_core::handle_with(&again.env(), &again.request());
    let response = tapkey_core::handle_with(&again.env(), &again.request());
    let second = collect(&again, &response);

    for (path, bytes) in first {
        // The store grows by one backup on the second run, which is correct rather than a
        // difference: it is the files belonging to the tools that must not move.
        if path.starts_with("store/") || path == "response.json" {
            continue;
        }
        assert_eq!(
            second.get(path).map(|b| String::from_utf8_lossy(b)),
            Some(String::from_utf8_lossy(bytes)),
            "{}: {path} moved on the second application",
            case.name
        );
    }
}

/// The one `CLAUDE.md` calls the test that matters most, phrased so a machine checks it: with
/// the keys tapkey may write cut out of both, what is left has to be identical.
fn untouched_bytes_survive(case: &Case, produced: &BTreeMap<String, Vec<u8>>) {
    let mut checked = 0usize;
    let files = settings_files(&case.dir.join("before"));
    for relative in &files {
        let relative = relative.clone();
        let before = std::fs::read(case.dir.join("before").join(&relative)).expect("before");
        let Ok(after) = std::fs::read(case.work.path().join(&relative)) else {
            continue; // the case is about the file going away
        };
        // Two formats, two readers, one property. They are not behind an interface: one preserves
        // bytes by construction and the other by restoration, and an interface fitted to both
        // would make each side worse. What is shared is the sentence, not the code.
        let cut = match relative.extension().and_then(|e| e.to_str()) {
            Some("toml") => excise_toml(&before, &after),
            _ => excise_json(&before, &after),
        };
        let Some((left, right)) = cut else {
            // A skip is allowed only where the case is *about* a file we could not read, and the
            // response says so. Left as a bare `continue`, a reader that stopped working would
            // take this property down with it in silence — measured: disabling the TOML half
            // outright left all seventeen cases green.
            assert!(
                refused(produced),
                "{}: {} could not be read, and the response does not say the switch was refused — \
                 a property that skips itself is not a property",
                case.name,
                relative.display()
            );
            continue;
        };
        checked += 1;
        assert_eq!(
            left,
            right,
            "{}: {} changed outside the keys tapkey owns",
            case.name,
            relative.display()
        );
    }

    // And the count is asserted, so a property that stopped running for a whole format shows up
    // as a failure rather than as a suspiciously quick pass.
    assert!(
        checked > 0 || files.is_empty() || refused(produced),
        "{}: the merge-never-own property compared nothing",
        case.name
    );
}

/// Whether the response reports a refusal, which is the only licence to skip a file.
fn refused(produced: &BTreeMap<String, Vec<u8>>) -> bool {
    produced
        .get("response.json")
        .and_then(|b| serde_json::from_slice::<serde_json::Value>(b).ok())
        .map(|v| v["ok"] == serde_json::json!(false))
        .unwrap_or(false)
}

fn excise_json(before: &[u8], after: &[u8]) -> Option<(String, String)> {
    let b = tapkey_core::json::Document::parse(before).ok()?;
    let a = tapkey_core::json::Document::parse(after).ok()?;
    Some((excise(before, &b), excise(after, &a)))
}

/// The TOML half of the same sentence. `toml_edit`'s **editable** document carries no spans at all
/// — not merely after an edit, but ever — so the spans come from a read-only type, and they are
/// mapped back to the coordinates of the original bytes, which a BOM shifts by three and each CRLF
/// by one more.
fn excise_toml(before: &[u8], after: &[u8]) -> Option<(String, String)> {
    let b = tapkey_core::toml::Spans::of(before).ok()?;
    let a = tapkey_core::toml::Spans::of(after).ok()?;
    let owned = tapkey_core::adapters::codex::owned_paths(Some("zai"));
    Some((cut_spans(before, &b, &owned), cut_spans(after, &a, &owned)))
}

fn cut_spans(bytes: &[u8], spans: &tapkey_core::toml::Spans, owned: &[Vec<String>]) -> String {
    let mut cuts: Vec<std::ops::Range<usize>> = owned
        .iter()
        .filter_map(|path| {
            let steps: Vec<&str> = path.iter().map(String::as_str).collect();
            // A table tapkey created is cut whole, header line included: a member's span covers a
            // key and its value, which for a table is only the `[header]`, and leaving the body
            // behind would compare our own keys against nothing.
            spans.table(&steps).or_else(|| spans.member(&steps))
        })
        .collect();
    cuts.sort_by_key(|r| r.start);

    let mut kept = Vec::new();
    let mut cursor = 0;
    for cut in cuts {
        if cut.start < cursor {
            continue;
        }
        kept.extend_from_slice(&bytes[cursor..cut.start]);
        cursor = cut.end;
    }
    kept.extend_from_slice(&bytes[cursor..]);
    String::from_utf8_lossy(&kept)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Everything but the members tapkey may write, with the separators around them collapsed so
/// an added or removed key does not read as a difference in what is left.
fn excise(bytes: &[u8], doc: &tapkey_core::json::Document) -> String {
    let owned = tapkey_core::adapters::claude::owned_paths();
    let mut cuts: Vec<std::ops::Range<usize>> = owned
        .iter()
        .filter_map(|path| doc.member_span(&path[..]))
        .collect();

    // A switch can create the `env` block itself, and then the block is ours as much as the
    // keys in it. It stops being ours the moment it holds one key we did not write — which is
    // the ordinary case on a machine somebody has configured by hand.
    let inside = doc.keys_at(&["env"]);
    let all_ours = !inside.is_empty()
        && inside
            .iter()
            .all(|k| owned.iter().any(|p| p.len() == 2 && p[1] == k));
    if inside.is_empty() || all_ours {
        if let Some(span) = doc.member_span(&["env"]) {
            cuts.push(span);
        }
    }
    cuts.sort_by_key(|r| r.start);

    let mut kept = Vec::new();
    let mut cursor = 0;
    for cut in cuts {
        if cut.start < cursor {
            continue;
        }
        kept.extend_from_slice(&bytes[cursor..cut.start]);
        cursor = cut.end;
    }
    kept.extend_from_slice(&bytes[cursor..]);

    // Whitespace and separators around the removed members are not evidence of anything.
    String::from_utf8_lossy(&kept)
        .chars()
        .filter(|c| !c.is_whitespace() && *c != ',')
        .collect()
}

/// Restoring the backup this run produced must return the tree exactly as it was found.
fn restore_returns_the_original(case: &Case) {
    let store = case.store();
    let Ok(entries) = std::fs::read_dir(store.join("backups")) else {
        return; // a refusal takes no backup, and has nothing to restore
    };
    let Some(id) = entries
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .max()
    else {
        return;
    };

    let request =
        format!(r#"{{"version":1,"op":"restore","params":{{"target":"backup","id":"{id}"}}}}"#);
    tapkey_core::handle_with(&case.env(), &request);

    for relative in settings_files(&case.dir.join("before")) {
        let before = std::fs::read(case.dir.join("before").join(&relative)).expect("before");
        let now = std::fs::read(case.work.path().join(&relative)).unwrap_or_default();
        assert_eq!(
            String::from_utf8_lossy(&now),
            String::from_utf8_lossy(&before),
            "{}: restoring did not return {} byte for byte",
            case.name,
            relative.display()
        );
    }
}

// --- plumbing ------------------------------------------------------------------------------

fn settings_files(root: &Path) -> Vec<PathBuf> {
    let mut out = BTreeMap::new();
    gather(root, root, &mut out, &[]);
    out.keys().map(PathBuf::from).collect()
}

fn gather(root: &Path, dir: &Path, out: &mut BTreeMap<String, Vec<u8>>, skip: &[&str]) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let relative = path
            .strip_prefix(root)
            .expect("under root")
            .to_string_lossy()
            .replace('\\', "/");
        if skip.iter().any(|s| relative.starts_with(s)) {
            continue;
        }
        if path.is_dir() {
            gather(root, &path, out, skip);
        } else if let Ok(bytes) = std::fs::read(&path) {
            out.insert(relative, bytes);
        }
    }
}

fn copy_tree(from: &Path, to: &Path) {
    let Ok(entries) = std::fs::read_dir(from) else {
        return;
    };
    for entry in entries.flatten() {
        let target = to.join(entry.file_name());
        if entry.path().is_dir() {
            std::fs::create_dir_all(&target).expect("mkdir");
            copy_tree(&entry.path(), &target);
        } else {
            std::fs::create_dir_all(target.parent().expect("parent")).expect("mkdir");
            std::fs::copy(entry.path(), &target).expect("copy");
        }
    }
}

fn read_optional(path: &Path) -> Option<Vec<u8>> {
    std::fs::read(path).ok()
}

/// Absolute paths in a response are real and have to be, so the harness replaces the roots it
/// knows with tokens — and **only** those. A path that does not start with one stays raw and
/// therefore fails the comparison, which is correct: a path leaking past the roots is the bug.
fn pretty(case: &Case, response: &str) -> Vec<u8> {
    let value: serde_json::Value = serde_json::from_str(response).expect("response is JSON");
    let text = serde_json::to_string_pretty(&value).expect("serialise");
    let text = text
        .replace(&case.store().to_string_lossy().into_owned(), "<store>")
        .replace(&case.home().to_string_lossy().into_owned(), "<home>")
        .replace(&case.work.path().to_string_lossy().into_owned(), "<work>");
    let mut bytes = text.into_bytes();
    bytes.push(b'\n');
    bytes
}

/// Rewrite the expectations — and only the expectations. A tool that rewrites the input to
/// match the output has stopped being a test.
fn bless(after: &Path, produced: &BTreeMap<String, Vec<u8>>) {
    let _ = std::fs::remove_dir_all(after);
    for (path, bytes) in produced {
        let target = after.join(path);
        std::fs::create_dir_all(target.parent().expect("parent")).expect("mkdir");
        std::fs::write(target, bytes).expect("write expectation");
    }
}

/// A byte diff in the output is fine for one line and useless for a tree, so the actual tree is
/// put somewhere a real diff tool can be pointed at it.
fn save_failure(case: &Case, produced: &BTreeMap<String, Vec<u8>>) -> PathBuf {
    let dump = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../target/golden-failures")
        .join(case.name.replace('/', "-"));
    let _ = std::fs::remove_dir_all(&dump);
    for (path, bytes) in produced {
        let target = dump.join(path);
        std::fs::create_dir_all(target.parent().expect("parent")).expect("mkdir");
        std::fs::write(target, bytes).expect("write");
    }
    dump
}
