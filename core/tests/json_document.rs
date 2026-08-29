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
fn setting_a_key_whose_parent_is_not_an_object_is_refused() {
    let mut doc = Document::parse(br#"{"env":"a string, oddly"}"#).expect("parse");

    assert!(doc.set_string(&["env", "ANTHROPIC_MODEL"], "x").is_err());
    assert_eq!(
        doc.to_bytes(),
        &br#"{"env":"a string, oddly"}"#[..],
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

// --- inserting and removing ------------------------------------------------------------

/// A new key takes its layout from the siblings it joins, and goes last — which is also where
/// Claude Code puts its own. Alphabetical or grouped insertion would impose an order the file
/// did not have.
#[test]
fn a_new_key_joins_its_siblings_in_their_own_style_and_goes_last() {
    let source = b"{\n  \"env\": {\n    \"A\": \"1\",\n    \"B\": \"2\"\n  }\n}\n";
    let mut doc = Document::parse(source).expect("parse");

    doc.set_string(&["env", "C"], "3").expect("set");

    assert_eq!(
        String::from_utf8_lossy(&doc.to_bytes()),
        "{\n  \"env\": {\n    \"A\": \"1\",\n    \"B\": \"2\",\n    \"C\": \"3\"\n  }\n}\n"
    );
}

#[test]
fn a_minified_object_stays_on_one_line_when_a_key_is_added() {
    let mut doc = Document::parse(br#"{"env":{"A":"1"},"theme":"dark"}"#).expect("parse");

    doc.set_string(&["env", "B"], "2").expect("set");

    assert_eq!(
        doc.to_bytes(),
        &br#"{"env":{"A":"1","B":"2"},"theme":"dark"}"#[..]
    );
}

#[test]
fn a_key_and_value_separated_by_a_space_gets_a_sibling_with_one_too() {
    let mut doc = Document::parse(br#"{"a": "1"}"#).expect("parse");

    doc.set_string(&["b"], "2").expect("set");

    assert_eq!(doc.to_bytes(), &br#"{"a": "1", "b": "2"}"#[..]);
}

#[test]
fn an_empty_object_receives_its_first_key() {
    let mut doc = Document::parse(b"{\n  \"env\": {}\n}\n").expect("parse");

    doc.set_string(&["env", "A"], "1").expect("set");

    assert_eq!(
        String::from_utf8_lossy(&doc.to_bytes()),
        "{\n  \"env\": {\n    \"A\": \"1\"\n  }\n}\n"
    );
}

/// Installing Claude Code creates no settings file, and a hand-written one need not have an
/// `env` block. Creating one follows the same style rule, applied recursively.
#[test]
fn a_missing_intermediate_object_is_created_in_the_files_own_style() {
    let source = b"{\n  \"theme\": \"dark\"\n}\n";
    let mut doc = Document::parse(source).expect("parse");

    doc.set_string(&["env", "ANTHROPIC_BASE_URL"], "https://x.test")
        .expect("set");

    assert_eq!(
        String::from_utf8_lossy(&doc.to_bytes()),
        "{\n  \"theme\": \"dark\",\n  \"env\": {\n    \"ANTHROPIC_BASE_URL\": \"https://x.test\"\n  }\n}\n"
    );
}

/// Removing has to take the separator and whitespace the insertion added, or adding and
/// removing the same key would leave a trail and idempotence would hold only by luck.
#[test]
fn adding_then_removing_a_key_leaves_the_file_byte_identical() {
    let source = b"{\n  \"env\": {\n    \"A\": \"1\"\n  }\n}\n";
    let once = {
        let mut doc = Document::parse(source).expect("parse");
        doc.set_string(&["env", "B"], "2").expect("set");
        doc.to_bytes()
    };
    let mut doc = Document::parse(&once).expect("reparse");

    doc.remove(&["env", "B"]).expect("remove");

    assert_eq!(
        String::from_utf8_lossy(&doc.to_bytes()),
        String::from_utf8_lossy(source)
    );
}

#[test]
fn removing_the_only_key_leaves_an_empty_object() {
    let mut doc = Document::parse(br#"{"env":{"A":"1"}}"#).expect("parse");

    doc.remove(&["env", "A"]).expect("remove");

    assert_eq!(doc.to_bytes(), &br#"{"env":{}}"#[..]);
}

#[test]
fn removing_the_first_of_several_keys_takes_its_trailing_comma() {
    let mut doc = Document::parse(br#"{"a":"1","b":"2"}"#).expect("parse");

    doc.remove(&["a"]).expect("remove");

    assert_eq!(doc.to_bytes(), &br#"{"b":"2"}"#[..]);
}

#[test]
fn removing_something_that_is_not_there_is_not_an_error() {
    let mut doc = Document::parse(br#"{"a":"1"}"#).expect("parse");

    doc.remove(&["env", "B"]).expect("a no-op, not a failure");

    assert_eq!(doc.to_bytes(), &br#"{"a":"1"}"#[..]);
}

#[test]
fn a_four_space_file_gets_four_space_insertions() {
    let source = b"{\n    \"theme\": \"dark\"\n}\n";
    let mut doc = Document::parse(source).expect("parse");

    doc.set_string(&["env", "A"], "1").expect("set");

    assert_eq!(
        String::from_utf8_lossy(&doc.to_bytes()),
        "{\n    \"theme\": \"dark\",\n    \"env\": {\n        \"A\": \"1\"\n    }\n}\n"
    );
}

#[test]
fn a_tab_indented_file_gets_tab_indented_insertions() {
    let source = b"{\n\t\"theme\": \"dark\"\n}\n";
    let mut doc = Document::parse(source).expect("parse");

    doc.set_string(&["env", "A"], "1").expect("set");

    assert_eq!(
        String::from_utf8_lossy(&doc.to_bytes()),
        "{\n\t\"theme\": \"dark\",\n\t\"env\": {\n\t\t\"A\": \"1\"\n\t}\n}\n"
    );
}

/// What the golden harness needs to check merge-never-own by machine: the bytes as found, and
/// the exact extent of each key tapkey owns, so everything else can be compared directly.
#[test]
fn the_original_bytes_and_a_members_extent_are_available_for_comparison() {
    let source = br#"{"a":"1","b":"2"}"#;
    let mut doc = Document::parse(source).expect("parse");
    let span = doc.member_span(&["b"]).expect("b is there");

    doc.set_string(&["a"], "changed").expect("set");

    assert_eq!(
        doc.original(),
        source,
        "the reading is kept, not overwritten"
    );
    assert_eq!(&source[span], br#""b":"2""#);
    assert_eq!(doc.member_span(&["nope"]), None);
}

/// Creating an object inside a file that has no newlines at all. Expanding it would impose a
/// layout the author did not choose, and every existing insertion test had a sibling or a
/// newline to copy from.
#[test]
fn a_nested_object_created_in_a_minified_file_stays_inline() {
    let mut doc = Document::parse(br#"{"theme":"dark"}"#).expect("parse");

    doc.set_string(&["env", "ANTHROPIC_BASE_URL"], "https://x.test")
        .expect("set");

    assert_eq!(
        doc.to_bytes(),
        &br#"{"theme":"dark","env":{"ANTHROPIC_BASE_URL":"https://x.test"}}"#[..]
    );
}

/// Tolerance is a property of the format the adapter declares, not of the reader — ADR-0010 settled
/// that after measuring what Claude Code does with a comment. The same measurement condemns a
/// trailing comma: strict JSON does not permit one, and Claude Code reports a Settings Error and
/// ignores the whole file. A reader that accepts one lets tapkey splice a file the tool will never
/// read, and then report success — the "intent instead of effective state" failure the product
/// exists to prevent.
#[test]
fn strict_json_refuses_a_trailing_comma() {
    let result = Document::parse(b"{\n  \"a\": 1,\n}\n");

    assert!(
        matches!(
            result.as_ref().err(),
            Some(tapkey_core::json::Error::Syntax { .. })
        ),
        "a trailing comma is not strict JSON: {:?}",
        result.err()
    );
}

/// And JSONC accepts both of the things strict JSON refuses, because refusing there would punish
/// somebody for using a documented feature of their tool's own format.
#[test]
fn jsonc_accepts_what_the_format_allows() {
    for source in [
        &b"{\n  // a note\n  \"a\": 1\n}\n"[..],
        &b"{\n  /* a note */\n  \"a\": 1\n}\n"[..],
        &b"{\n  \"a\": 1,\n}\n"[..],
    ] {
        let parsed = Document::parse_jsonc(source);
        assert!(
            parsed.is_ok(),
            "JSONC must accept this: {}",
            String::from_utf8_lossy(source)
        );
    }
}

/// The tolerance a document was opened with has to survive an edit, or the second write to a JSONC
/// file would trip over the comments the first one carefully preserved.
#[test]
fn a_jsonc_document_stays_jsonc_across_an_edit() {
    let mut document =
        Document::parse_jsonc(b"{\n  // keep me\n  \"a\": \"one\",\n}\n").expect("parses");

    document.set_string(&["a"], "two").expect("first edit");
    document.set_string(&["a"], "three").expect("second edit");

    assert_eq!(
        String::from_utf8(document.to_bytes()).unwrap(),
        "{\n  // keep me\n  \"a\": \"three\",\n}\n"
    );
}

/// A comment is content, and content is never repeated. The style a new member copies is the
/// whitespace and punctuation between two existing ones — not whatever those two happen to have
/// written between them. Found by looking at a real file rather than by an assertion that the
/// output *contains* the right things, which every `contains` in the suite had passed.
#[test]
fn inserting_into_a_commented_file_does_not_duplicate_the_comment() {
    let mut document = Document::parse_jsonc(b"{\n  // hand written\n  \"theme\": \"dark\",\n}\n")
        .expect("parses");

    document.set_string(&["model"], "new").expect("sets");

    let after = String::from_utf8(document.to_bytes()).unwrap();
    assert_eq!(
        after.matches("// hand written").count(),
        1,
        "the comment was copied into the insertion:\n{after}"
    );
}

/// And the layout it copies is the layout, not the leftovers. Stripping a comment out of the gap
/// between two members leaves the newline that followed it, so an insertion grew a blank line —
/// invisible to every `contains` assertion and obvious the moment the whole file is asserted.
#[test]
fn an_insertion_copies_the_indentation_and_nothing_else() {
    let mut document = Document::parse_jsonc(b"{\n  // hand written\n  \"theme\": \"dark\",\n}\n")
        .expect("parses");

    document.set_string(&["model"], "new").expect("sets");

    assert_eq!(
        String::from_utf8(document.to_bytes()).unwrap(),
        "{\n  // hand written\n  \"theme\": \"dark\",\n  \"model\": \"new\",\n}\n"
    );
}

/// A minified JSONC file has no newline to take a layout from, so the comment stripper is what
/// keeps a comment out of the insertion there. Without this case, removing the stripper altogether
/// changed nothing that any test could see.
#[test]
fn a_minified_jsonc_file_does_not_carry_its_comment_into_an_insertion() {
    let mut document = Document::parse_jsonc(b"{/* note */\"a\":\"one\"}").expect("parses");

    document.set_string(&["b"], "two").expect("sets");

    assert_eq!(
        String::from_utf8(document.to_bytes()).unwrap(),
        "{/* note */\"a\":\"one\",\"b\":\"two\"}"
    );
}

/// Removal takes the element and the separator before it, so remove-then-append leaves no doubled
/// comma — the idempotence rule from ADR-0010, one level down. Absent array, absent element: both
/// the same as nothing to do.
#[test]
fn an_element_can_be_removed_from_an_array() {
    let mut document = Document::parse_jsonc(
        b"{\n  \"enabled_providers\": [\"anthropic\", \"tapkey-zai\", \"ollama\"],\n}\n",
    )
    .expect("parses");

    document
        .remove_from_array(&["enabled_providers"], "tapkey-zai")
        .expect("removes");
    document
        .remove_from_array(&["enabled_providers"], "never-there")
        .expect("absent element is nothing to do");
    document
        .remove_from_array(&["no_such_list"], "x")
        .expect("absent array too");

    assert_eq!(
        String::from_utf8(document.to_bytes()).unwrap(),
        "{\n  \"enabled_providers\": [\"anthropic\", \"ollama\"],\n}\n"
    );
}
