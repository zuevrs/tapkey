//! The TOML editor, at the seam the adapter uses it through.
//!
//! `toml_edit` keeps comments, spacing and item order. It does not keep the byte envelope: a BOM
//! is stripped, CRLF becomes LF and a missing final newline is added, all on a bare parse and
//! render with no edit at all. Measured at 0.25.13. Everything here that looks like it is testing
//! somebody else's library is testing the wrapper that puts those three back.

use tapkey_core::toml::{Document, Error, Spans};

#[test]
fn a_file_with_crlf_endings_comes_back_with_crlf_endings() {
    let source = b"model = \"a\"\r\n\r\n[t]\r\nb = \"c\"\r\n";

    let document = Document::parse(source).expect("parses");

    assert_eq!(
        document.to_bytes(),
        source,
        "the line endings the file had are not the ones it came back with"
    );
}

#[test]
fn a_file_with_no_final_newline_does_not_grow_one() {
    let source = b"model = \"a\"\n\n[t]\nb = \"c\"";

    let document = Document::parse(source).expect("parses");

    assert_eq!(
        document.to_bytes(),
        source,
        "a newline appeared from nowhere"
    );
}

#[test]
fn a_byte_order_mark_survives() {
    let source = b"\xef\xbb\xbfmodel = \"a\"\n";

    let document = Document::parse(source).expect("parses");

    assert_eq!(document.to_bytes(), source, "the mark was eaten");
}

/// The three envelope properties are independent, and a file can carry all of them at once. This
/// is the case that would pass if any two were handled and the third quietly forgotten.
#[test]
fn all_three_at_once() {
    let source = b"\xef\xbb\xbfmodel = \"a\"\r\n\r\n[t]\r\nb = \"c\"";

    let document = Document::parse(source).expect("parses");

    assert_eq!(document.to_bytes(), source);
}

/// The envelope handles the three normalisations we know about. This is the guard against the ones
/// we do not: parse, render unchanged, and refuse the file if what comes out is not what went in.
/// A file with mixed line endings is one such — our own rule calls a file CRLF if it holds a single
/// CRLF anywhere, so its LF-only lines would come back changed, by us.
#[test]
fn a_file_our_technique_would_alter_is_refused_rather_than_altered() {
    let source = b"a = \"1\"\r\nb = \"2\"\nc = \"3\"\r\n";

    let result = Document::parse(source);

    assert_eq!(
        result.err(),
        Some(Error::NotPreserved),
        "a file we cannot hand back unchanged must not be opened for editing"
    );
}

/// And the guard must not fire on ordinary files, or it would refuse everything and prove nothing.
#[test]
fn an_ordinary_file_passes_the_guard() {
    let source = b"# a comment\nmodel = \"a\"\n\n[t]\nname    = \"aligned\"  # trailing note\n";

    let document = Document::parse(source).expect("an ordinary file must parse");

    assert_eq!(document.to_bytes(), source);
}

/// The finding this editor exists to respect: `toml_edit`'s ordinary assignment keeps a key's
/// hand-alignment and **drops a trailing comment on the same line**, silently. A comment is the
/// person's content, so losing one is a `merge-never-own` violation that announces nothing. This
/// case fails on the naive path and passes only on an in-place edit that carries the decor.
#[test]
fn setting_a_value_keeps_the_comment_beside_it() {
    let source = b"[model_providers.mine]\nname    = \"Mine\"   # the one at work\nbase_url = \"https://e.invalid/v1\"\n";
    let mut document = Document::parse(source).expect("parses");

    document
        .set_string(&["model_providers", "mine", "name"], "Renamed")
        .expect("sets");

    assert_eq!(
        String::from_utf8(document.to_bytes()).unwrap(),
        "[model_providers.mine]\nname    = \"Renamed\"   # the one at work\nbase_url = \"https://e.invalid/v1\"\n"
    );
}

/// Reading is how the adapter learns what is there before deciding anything.
#[test]
fn a_string_can_be_read_at_a_path() {
    let source =
        b"model = \"gpt-5.6\"\n\n[model_providers.mine]\nbase_url = \"https://e.invalid/v1\"\n";
    let document = Document::parse(source).expect("parses");

    assert_eq!(document.get_string(&["model"]), Some("gpt-5.6"));
    assert_eq!(
        document.get_string(&["model_providers", "mine", "base_url"]),
        Some("https://e.invalid/v1")
    );
    assert_eq!(document.get_string(&["model_providers", "absent"]), None);
    assert_eq!(document.get_string(&["nowhere", "at", "all"]), None);
}

/// A key that is not a string is not a string, and reporting it as one would put a number into a
/// place the tool expects a model name. Codex refuses the whole file over a type error, so the
/// distinction is not academic.
#[test]
fn a_value_that_is_not_a_string_reads_as_absent() {
    let document = Document::parse(b"model = 42\n").expect("parses");

    assert_eq!(document.get_string(&["model"]), None);
}

/// Removal is what a slot with no assignment does to Codex's file. Unlike Claude Code, nothing
/// fires from the environment underneath, so deleting is enough — see ADR-0014.
#[test]
fn a_key_can_be_removed_and_takes_its_line_with_it() {
    let source = b"# kept\nmodel = \"a\"\n\n[t]\nb = \"1\"\nc = \"2\"\n";
    let mut document = Document::parse(source).expect("parses");

    document.remove(&["t", "b"]).expect("removes");

    assert_eq!(
        String::from_utf8(document.to_bytes()).unwrap(),
        "# kept\nmodel = \"a\"\n\n[t]\nc = \"2\"\n"
    );
}

#[test]
fn removing_something_that_was_never_there_is_not_an_error() {
    let mut document = Document::parse(b"model = \"a\"\n").expect("parses");

    document.remove(&["nothing"]).expect("not an error");
    document.remove(&["no", "such", "path"]).expect("nor this");

    assert_eq!(document.to_bytes(), b"model = \"a\"\n");
}

/// The golden harness states `merge-never-own` mechanically: `before` minus the spans of the keys
/// tapkey owns must equal `after` minus the same. It slices the **original** bytes, so a span must
/// index them — not the body the editor strips the envelope off. A BOM shifts every offset by
/// three and each CRLF by one more, and getting that wrong would silently excise the wrong bytes
/// and leave the property passing while checking nothing.
///
/// Spans live on their own type because `toml_edit`'s editable document does not carry them at
/// all — not merely after an edit, but ever. Reading where things are and changing them are two
/// jobs there, and pretending otherwise cost this test one red run to discover.
#[test]
fn a_span_indexes_the_original_bytes_envelope_and_all() {
    let source = b"\xef\xbb\xbfmodel = \"a\"\r\n\r\n[t]\r\nb = \"c\"\r\n";
    let spans = Spans::of(source).expect("parses");

    let span = spans.member(&["t", "b"]).expect("the member is there");

    assert_eq!(&source[span], b"b = \"c\"");
}

#[test]
fn a_span_covers_the_key_as_well_as_the_value() {
    let source = b"model = \"gpt-5.6\"\n";
    let spans = Spans::of(source).expect("parses");

    let span = spans.member(&["model"]).expect("the member is there");

    assert_eq!(&source[span], b"model = \"gpt-5.6\"");
}

/// An empty input is not a file without a trailing newline — it is a file with no envelope at all,
/// and a created file should look like every other file on the disk. Codex writes one; so do we.
#[test]
fn a_file_created_from_nothing_ends_with_a_newline() {
    let mut document = Document::parse(b"").expect("an absent file parses as an empty one");

    document.set_string(&["model"], "glm-5.3").expect("sets");

    assert_eq!(document.to_bytes(), b"model = \"glm-5.3\"\n");
}
