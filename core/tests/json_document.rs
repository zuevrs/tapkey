//! Tests for the JSON document, at its public interface.
//!
//! The document exists to change the few keys tapkey owns and leave every other byte of the
//! file exactly as it was found. Nearly every test here is therefore a byte comparison.

use tapkey_core::json::Document;

#[test]
fn replaces_a_top_level_string_and_changes_nothing_else() {
    let mut doc = Document::parse(br#"{"theme": "dark"}"#).expect("parse");

    doc.set_string(&["theme"], "light").expect("set");

    assert_eq!(doc.to_bytes(), &br#"{"theme": "light"}"#[..]);
}

/// The shape a real `~/.claude/settings.json` has: an `env` block holding a hand-written
/// endpoint and pins beside the two keys tapkey would set.
const REAL: &[u8] = br#"{
  "env": {
    "ANTHROPIC_BASE_URL": "https://api.a6api.com",
    "ANTHROPIC_AUTH_TOKEN": "sk-PLACEHOLDER-NOT-A-REAL-KEY",
    "ANTHROPIC_DEFAULT_OPUS_MODEL": "claude-opus-5",
    "ANTHROPIC_DEFAULT_HAIKU_MODEL": "claude-haiku-4-5"
  },
  "skipDangerousModePermissionPrompt": true,
  "theme": "dark"
}
"#;

#[test]
fn replaces_a_nested_value_and_leaves_every_other_byte_alone() {
    let mut doc = Document::parse(REAL).expect("parse");

    doc.set_string(
        &["env", "ANTHROPIC_BASE_URL"],
        "https://openrouter.ai/api/v1",
    )
    .expect("set");

    let after = doc.to_bytes();
    let expected = String::from_utf8_lossy(REAL)
        .replace("https://api.a6api.com", "https://openrouter.ai/api/v1");
    assert_eq!(String::from_utf8_lossy(&after), expected);
}

#[test]
fn a_byte_order_mark_survives() {
    let source = b"\xEF\xBB\xBF{\"theme\": \"dark\"}".to_vec();
    let mut doc = Document::parse(&source).expect("parse");

    doc.set_string(&["theme"], "light").expect("set");

    assert_eq!(
        &doc.to_bytes()[..3],
        b"\xEF\xBB\xBF",
        "the tool accepts it, so it stays"
    );
}

#[test]
fn carriage_returns_survive() {
    let source = b"{\r\n  \"theme\": \"dark\"\r\n}\r\n";
    let mut doc = Document::parse(source).expect("parse");

    doc.set_string(&["theme"], "light").expect("set");

    assert_eq!(doc.to_bytes(), b"{\r\n  \"theme\": \"light\"\r\n}\r\n");
}

#[test]
fn a_missing_trailing_newline_is_not_added() {
    let mut doc = Document::parse(b"{\"theme\": \"dark\"}").expect("parse");

    doc.set_string(&["theme"], "light").expect("set");

    assert!(
        !doc.to_bytes().ends_with(b"\n"),
        "tapkey does not tidy files it does not own"
    );
}

#[test]
fn a_minified_file_stays_minified() {
    let mut doc =
        Document::parse(br#"{"env":{"ANTHROPIC_MODEL":"a"},"theme":"dark"}"#).expect("parse");

    doc.set_string(&["env", "ANTHROPIC_MODEL"], "b")
        .expect("set");

    assert_eq!(
        doc.to_bytes(),
        &br#"{"env":{"ANTHROPIC_MODEL":"b"},"theme":"dark"}"#[..]
    );
}

#[test]
fn non_ascii_passes_through_as_utf8_rather_than_being_escaped() {
    let mut doc = Document::parse(br#"{"note":"x"}"#).expect("parse");

    doc.set_string(&["note"], "модель «быстрая»").expect("set");

    assert_eq!(doc.to_bytes(), "{\"note\":\"модель «быстрая»\"}".as_bytes());
}

#[test]
fn setting_the_same_value_twice_is_byte_identical() {
    let once = {
        let mut doc = Document::parse(REAL).expect("parse");
        doc.set_string(&["env", "ANTHROPIC_BASE_URL"], "https://x.test")
            .expect("set");
        doc.to_bytes()
    };
    let twice = {
        let mut doc = Document::parse(&once).expect("reparse");
        doc.set_string(&["env", "ANTHROPIC_BASE_URL"], "https://x.test")
            .expect("set");
        doc.to_bytes()
    };

    assert_eq!(
        once, twice,
        "applying the same profile twice must not move a byte"
    );
}

/// Claude Code reports a file with comments as a Settings Error and ignores it entirely, so
/// splicing one would mean writing into something the tool never reads and calling it success.
#[test]
fn strict_json_refuses_a_comment() {
    let source = b"{\n  // the endpoint\n  \"theme\": \"dark\"\n}";
    assert!(Document::parse(source).is_err());
}

/// Parsers disagree about which duplicate wins, so effective state cannot be promised. That
/// makes it an invariant rather than a convenience.
#[test]
fn a_duplicate_key_is_refused() {
    let source = br#"{"theme": "dark", "theme": "light"}"#;
    assert!(
        Document::parse(source).is_err(),
        "we cannot know which one the tool reads"
    );
}

/// Escapes were entirely unexercised until a mutation run said so: every arm of the escape
/// decoder could be deleted without a test noticing. A Windows path and a quoted phrase are
/// what actually turns up in these files.
#[test]
fn strings_holding_escapes_survive_untouched() {
    let source = br#"{"a":"C:\\Users\\me","b":"say \"hi\"","c":"tab\there","d":"\u00e9\u0041","f":"\/slash \b \f \n \r","e":"x"}"#;
    let mut doc = Document::parse(source).expect("parse");

    doc.set_string(&["e"], "y").expect("set");

    let expected = String::from_utf8_lossy(source).replace(r#""e":"x""#, r#""e":"y""#);
    assert_eq!(String::from_utf8_lossy(&doc.to_bytes()), expected);
}

/// A string may hold the very characters that delimit structure. A scanner that treats them as
/// structure walks off the end of the object.
#[test]
fn braces_and_commas_inside_a_string_are_not_structure() {
    let source = br#"{"tricky":"} , { \" ]","theme":"dark"}"#;
    let mut doc = Document::parse(source).expect("parse");

    doc.set_string(&["theme"], "light").expect("set");

    assert_eq!(
        doc.to_bytes(),
        &br#"{"tricky":"} , { \" ]","theme":"light"}"#[..]
    );
}

/// `permissions.allow` is a list tapkey has no business touching, and lists were unparsed
/// territory: nothing asserted that one survives a switch.
#[test]
fn arrays_are_preserved_including_nested_ones() {
    let source = br#"{
  "permissions": { "allow": ["Bash(ls:*)", "Read", { "tool": "Edit", "paths": [1, 2.5, -3e2] }], "deny": [] },
  "theme": "dark"
}"#;
    let mut doc = Document::parse(source).expect("parse");

    doc.set_string(&["theme"], "light").expect("set");

    let expected =
        String::from_utf8_lossy(source).replace(r#""theme": "dark""#, r#""theme": "light""#);
    assert_eq!(String::from_utf8_lossy(&doc.to_bytes()), expected);
}

#[test]
fn an_unterminated_array_is_a_syntax_error() {
    assert!(Document::parse(br#"{"a":[1,2"#).is_err());
}

#[test]
fn an_array_missing_its_separator_is_a_syntax_error() {
    assert!(Document::parse(br#"{"a":[1 2]}"#).is_err());
}

/// A value carrying a control character must leave as a valid JSON escape, or tapkey writes a
/// file the tool then refuses to read.
#[test]
fn a_control_character_in_a_written_value_is_escaped() {
    let mut doc = Document::parse(br#"{"note":"x"}"#).expect("parse");

    doc.set_string(&["note"], "one\u{1}two").expect("set");

    assert_eq!(doc.to_bytes(), &br#"{"note":"one\u0001two"}"#[..]);
}

#[test]
fn setting_one_key_twice_keeps_only_the_last_value() {
    let mut doc = Document::parse(br#"{"theme":"dark"}"#).expect("parse");

    doc.set_string(&["theme"], "light").expect("first");
    doc.set_string(&["theme"], "system").expect("second");

    assert_eq!(doc.to_bytes(), &br#"{"theme":"system"}"#[..]);
}

#[test]
fn a_path_that_is_not_there_is_reported_rather_than_created() {
    let mut doc = Document::parse(br#"{"theme":"dark"}"#).expect("parse");

    assert!(doc.set_string(&["env", "ANTHROPIC_MODEL"], "x").is_err());
    assert_eq!(
        doc.to_bytes(),
        &br#"{"theme":"dark"}"#[..],
        "and nothing moved"
    );
}

#[test]
fn every_truncated_array_refuses() {
    for source in [
        &b"{\"a\":["[..],
        b"{\"a\":[1,",
        b"{\"a\":[1,2",
        b"{\"a\":[ ",
    ] {
        assert!(
            Document::parse(source).is_err(),
            "{:?} was accepted",
            String::from_utf8_lossy(source)
        );
    }
}
