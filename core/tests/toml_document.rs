//! The TOML editor, at the seam the adapter uses it through.
//!
//! `toml_edit` keeps comments, spacing and item order. It does not keep the byte envelope: a BOM
//! is stripped, CRLF becomes LF and a missing final newline is added, all on a bare parse and
//! render with no edit at all. Measured at 0.25.13. Everything here that looks like it is testing
//! somebody else's library is testing the wrapper that puts those three back.

use tapkey_core::toml::{Document, Error};

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
