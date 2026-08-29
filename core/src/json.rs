//! A JSON document that remembers where every value sits in the original bytes.
//!
//! Edits are byte-range splices, never a re-serialisation: a tool's own layout, comments, key
//! order, BOM and line endings all survive, because tapkey must not be the edit that shows up
//! in a diff. Splices are collected against the original coordinates and applied together, so
//! the ranges a caller is told about mean the same thing before and after.

use std::ops::Range;

/// A parsed document. Every edit is applied to the bytes and the document is re-parsed, so a
/// second edit sees the first — which is what setting a key that a previous call created, or
/// twice over, requires. The original bytes are kept for comparison.
pub struct Document {
    original: Vec<u8>,
    bytes: Vec<u8>,
    root: Node,
    /// The format this file was opened as, so an edit reparses the way it was read.
    tolerance: Tolerance,
}

/// Why a document could not be parsed, or an edit could not be made.
#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    /// The bytes are not the format they claim to be.
    Syntax { offset: usize, message: String },
    /// The same key appears twice in one object. Parsers disagree about which one wins, so
    /// effective state cannot be promised and the file is refused rather than guessed at.
    DuplicateKey { offset: usize, key: String },
    /// Something of another shape is where a value or an object should go. An unexpected type
    /// on a key tapkey owns is corrected; on the way to one, it is refused, because replacing
    /// it would be rewriting something we do not own.
    NotAnObject { path: String },
}

enum Node {
    Object {
        members: Vec<Member>,
        span: Range<usize>,
    },
    /// An array or a scalar. tapkey never reaches inside one, so its contents are not modelled
    /// — only where it begins and ends, which is what a splice needs, plus a string's decoded
    /// value, which is what reading effective state needs.
    Opaque {
        span: Range<usize>,
        text: Option<String>,
    },
}

struct Member {
    key: String,
    key_span: Range<usize>,
    value: Node,
    /// Key start through value end, not including the comma that separates it from the next.
    span: Range<usize>,
}

impl Document {
    /// Parse `bytes` as strict JSON.
    /// Strict JSON. Comments and a trailing comma are refused, because the tools whose files are
    /// strict refuse them too, and a file we spliced but the tool ignores is worse than a refusal.
    pub fn parse(bytes: &[u8]) -> Result<Self, Error> {
        Self::parse_with(bytes, Tolerance::Strict)
    }

    /// JSONC. Comments and a trailing comma are content, not errors.
    pub fn parse_jsonc(bytes: &[u8]) -> Result<Self, Error> {
        Self::parse_with(bytes, Tolerance::Jsonc)
    }

    fn parse_with(bytes: &[u8], tolerance: Tolerance) -> Result<Self, Error> {
        let root = parse_root(bytes, tolerance)?;
        Ok(Document {
            original: bytes.to_vec(),
            bytes: bytes.to_vec(),
            root,
            tolerance,
        })
    }

    /// The bytes this document was parsed from, before any edit.
    pub fn original(&self) -> &[u8] {
        &self.original
    }

    /// The string at `path`, or `None` if there is nothing there or it is not a string.
    ///
    /// Reading goes through this reader rather than a general-purpose one so that the refusals
    /// apply on the read path too: a duplicate key means effective state cannot be promised,
    /// and promising it from a second parser that silently picks a winner would be worse than
    /// not answering.
    pub fn get_string(&self, path: &[&str]) -> Option<&str> {
        match resolve(&self.root, path)? {
            Node::Opaque { text, .. } => text.as_deref(),
            Node::Object { .. } => None,
        }
    }

    /// The keys of the object at `path`, in the order the file has them. Empty when there is
    /// no object there.
    pub fn keys_at(&self, path: &[&str]) -> Vec<String> {
        match resolve(&self.root, path) {
            Some(Node::Object { members, .. }) => members.iter().map(|m| m.key.clone()).collect(),
            _ => Vec::new(),
        }
    }

    /// Key start through value end for `path`, excluding the separating comma. What a caller
    /// needs to excise the regions tapkey owns and compare everything else byte for byte.
    pub fn member_span(&self, path: &[&str]) -> Option<Range<usize>> {
        member(&self.root, path).map(|m| m.span.clone())
    }

    /// Set the string at `path`, creating whatever part of it is missing.
    ///
    /// A key that is not there yet is the ordinary case on a fresh machine, and a missing
    /// intermediate object is created in the file's own style, recursively.
    pub fn set_string(&mut self, path: &[&str], value: &str) -> Result<(), Error> {
        let (range, replacement) = self.plan_set(path, value)?;
        self.splice(range, &replacement)
    }

    /// Remove the key at `path`, taking the separator and whitespace its insertion added.
    /// Anything less and adding then removing the same key would leave a trail, so idempotence
    /// would hold only by luck. A path that is not there is a no-op, not a failure.
    pub fn remove(&mut self, path: &[&str]) -> Result<(), Error> {
        let Some((parent, m)) = parent_and_member(&self.root, path) else {
            return Ok(());
        };
        let Node::Object { members, span } = parent else {
            return Ok(());
        };
        let index = members
            .iter()
            .position(|x| x.span == m.span)
            .expect("the member came from these members");

        let range = if members.len() == 1 {
            // The only member: take everything between the braces, leaving `{}` as authored.
            span.start + 1..span.end - 1
        } else if index + 1 < members.len() {
            // Not the last: swallow forward to the next member's start, comma included.
            m.span.start..members[index + 1].span.start
        } else {
            // The last: swallow backwards to the previous member's end.
            members[index - 1].span.end..m.span.end
        };
        self.splice(range, b"")
    }

    /// The document's current bytes.
    pub fn to_bytes(&self) -> Vec<u8> {
        self.bytes.clone()
    }

    fn splice(&mut self, range: Range<usize>, replacement: &[u8]) -> Result<(), Error> {
        let mut next = Vec::with_capacity(self.bytes.len() + replacement.len());
        next.extend_from_slice(&self.bytes[..range.start]);
        next.extend_from_slice(replacement);
        next.extend_from_slice(&self.bytes[range.end..]);
        // Reparsed with the tolerance it was opened with. Reparsing a JSONC document strictly
        // would turn its own comments into an error on the second edit.
        self.root = parse_root(&next, self.tolerance)?;
        self.bytes = next;
        Ok(())
    }

    /// Where to splice, and what to put there, for one set.
    fn plan_set(&self, path: &[&str], value: &str) -> Result<(Range<usize>, Vec<u8>), Error> {
        // The whole path is already there: replace the value in place.
        if let Some(existing) = member(&self.root, path) {
            return Ok((existing.value.span(), encode_string(value)));
        }

        // Walk as far as the file goes, then build what is missing as one nested literal.
        // The full path having been ruled out above, this loop always breaks before `depth`
        // reaches the end — which is why a mutation widening the bound survives it.
        let mut node = &self.root;
        let mut depth = 0;
        while depth < path.len() {
            let Node::Object { members, .. } = node else {
                return Err(Error::NotAnObject {
                    path: path[..depth].join("."),
                });
            };
            match members.iter().find(|m| m.key == path[depth]) {
                Some(m) => {
                    node = &m.value;
                    depth += 1;
                }
                None => break,
            }
        }
        let Node::Object { members, span } = node else {
            return Err(Error::NotAnObject {
                path: path[..depth].join("."),
            });
        };

        let style = Style::of(
            &self.bytes,
            members,
            span,
            document_step(&self.bytes, &self.root),
        );
        let literal = style.nested(&path[depth..], value);
        let at = match members.last() {
            Some(last) => last.span.end,
            None => span.start + 1,
        };
        let mut out = Vec::new();
        if !members.is_empty() {
            out.push(b',');
        }
        out.extend_from_slice(&literal);
        if members.is_empty() {
            out.extend_from_slice(style.close.as_bytes());
        }
        Ok((at..at, out))
    }
}

/// How this file writes an object member, learned from the members already in it.
struct Style {
    /// What comes after the separating comma and before the next key: a newline and
    /// indentation in a laid-out file, a space or nothing in a one-line one.
    lead: String,
    /// What sits between a key and its value: a colon, and a space if the file uses one.
    colon: String,
    /// What comes before the closing brace, when this insertion has to write one.
    close: String,
    /// One level of indentation, for objects this insertion has to create.
    step: String,
}

impl Style {
    fn of(bytes: &[u8], members: &[Member], span: &Range<usize>, step: String) -> Style {
        let text = |r: Range<usize>| String::from_utf8_lossy(&bytes[r]).into_owned();

        if let Some(first) = members.first() {
            let colon = text(first.key_span.end..first.value.span().start);
            let opening = text(span.start + 1..first.span.start);
            // Two members show exactly what goes between them; one member can only be
            // inferred from. A file that writes `"a": 1` would write `, "b": 2`, and a
            // minified one would not.
            let lead = match members.get(1) {
                Some(second) => text(first.span.end + 1..second.span.start),
                None if !opening.is_empty() => opening.clone(),
                None if colon.ends_with(' ') => " ".to_string(),
                None => String::new(),
            };
            return Style {
                lead,
                colon,
                // Only an object with no members needs a closing lead; this one has some.
                close: String::new(),
                step,
            };
        }

        // An empty object has no sibling to copy, so the layout comes from the line the object
        // itself sits on: one step further in, and the brace back where the line began.
        let line_indent = indentation_before(bytes, span.start);
        let multiline = bytes.contains(&b'\n');
        if multiline {
            Style {
                lead: format!("\n{line_indent}{step}"),
                colon: ": ".to_string(),
                close: format!("\n{line_indent}"),
                step,
            }
        } else {
            Style {
                lead: String::new(),
                colon: ":".to_string(),
                close: String::new(),
                step,
            }
        }
    }

    /// `"a": {"b": {"c": value}}`, laid out the way this file lays things out.
    fn nested(&self, path: &[&str], value: &str) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(self.lead.as_bytes());
        self.build(path, value, &self.lead, &mut out);
        out
    }

    fn build(&self, path: &[&str], value: &str, lead: &str, out: &mut Vec<u8>) {
        let (head, rest) = path
            .split_first()
            .expect("a path with at least one segment");
        out.extend_from_slice(&encode_string(head));
        out.extend_from_slice(self.colon.as_bytes());
        if rest.is_empty() {
            out.extend_from_slice(&encode_string(value));
            return;
        }
        let inner = if lead.contains('\n') {
            format!("{lead}{}", self.step)
        } else {
            lead.to_string()
        };
        out.push(b'{');
        out.extend_from_slice(inner.as_bytes());
        self.build(rest, value, &inner, out);
        out.extend_from_slice(lead.as_bytes());
        out.push(b'}');
    }
}

/// The whitespace run immediately before `offset`, back to the start of its line.
fn indentation_before(bytes: &[u8], offset: usize) -> String {
    let line_start = bytes[..offset]
        .iter()
        .rposition(|b| *b == b'\n')
        .map(|i| i + 1)
        .unwrap_or(0);
    let indent: Vec<u8> = bytes[line_start..offset]
        .iter()
        .take_while(|b| **b == b' ' || **b == b'\t')
        .copied()
        .collect();
    String::from_utf8_lossy(&indent).into_owned()
}

/// One level of indentation, taken from the document's own first level rather than from the
/// object being edited: a nested object's indentation is several steps, and dividing it by the
/// depth would be guessing. Two spaces when the file offers no example, which is what Claude
/// Code writes anyway.
///
/// A mutation run reports this function's guard as survivable, and it is: the step is consumed
/// only where the surrounding lead carries a newline, so in a file with none it is computed and
/// never read. Verified by reachability rather than assumed.
fn document_step(bytes: &[u8], root: &Node) -> String {
    let Node::Object { members, span } = root else {
        return "  ".to_string();
    };
    let Some(first) = members.first() else {
        return "  ".to_string();
    };
    let opening = String::from_utf8_lossy(&bytes[span.start + 1..first.span.start]);
    match opening.rsplit('\n').next() {
        Some(indent) if opening.contains('\n') && !indent.is_empty() => indent.to_string(),
        _ => "  ".to_string(),
    }
}

fn parse_root(bytes: &[u8], tolerance: Tolerance) -> Result<Node, Error> {
    let start = if bytes.starts_with(b"\xEF\xBB\xBF") {
        3
    } else {
        0
    };
    let mut parser = Parser {
        b: bytes,
        i: start,
        tolerance,
    };
    parser.skip_whitespace();
    let root = parser.value()?;
    parser.skip_whitespace();
    if parser.i != bytes.len() {
        return Err(parser.error("trailing content after the top-level value"));
    }
    Ok(root)
}

impl Node {
    fn span(&self) -> Range<usize> {
        match self {
            Node::Object { span, .. } | Node::Opaque { span, .. } => span.clone(),
        }
    }
}

fn member<'a>(node: &'a Node, path: &[&str]) -> Option<&'a Member> {
    let (last, parents) = path.split_last()?;
    let parent = resolve(node, parents)?;
    let Node::Object { members, .. } = parent else {
        return None;
    };
    members.iter().find(|m| m.key == *last)
}

fn parent_and_member<'a>(node: &'a Node, path: &[&str]) -> Option<(&'a Node, &'a Member)> {
    let (last, parents) = path.split_last()?;
    let parent = resolve(node, parents)?;
    let Node::Object { members, .. } = parent else {
        return None;
    };
    members.iter().find(|m| m.key == *last).map(|m| (parent, m))
}

fn resolve<'a>(node: &'a Node, path: &[&str]) -> Option<&'a Node> {
    let Some((head, rest)) = path.split_first() else {
        return Some(node);
    };
    let Node::Object { members, .. } = node else {
        return None;
    };
    let member = members.iter().find(|m| m.key == *head)?;
    resolve(&member.value, rest)
}

/// Only what the format requires: a value tapkey writes must not arrive in the file more
/// escaped than the user's own values are.
fn encode_string(value: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(value.len() + 2);
    out.push(b'"');
    for ch in value.chars() {
        match ch {
            '"' => out.extend_from_slice(b"\\\""),
            '\\' => out.extend_from_slice(b"\\\\"),
            '\n' => out.extend_from_slice(b"\\n"),
            '\r' => out.extend_from_slice(b"\\r"),
            '\t' => out.extend_from_slice(b"\\t"),
            c if (c as u32) < 0x20 => {
                out.extend_from_slice(format!("\\u{:04x}", c as u32).as_bytes())
            }
            c => {
                let mut buf = [0u8; 4];
                out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes())
            }
        }
    }
    out.push(b'"');
    out
}

/// What this file's format allows, declared by the adapter that owns the file.
///
/// ADR-0010 settles that tolerance belongs to the format rather than to the reader, and the reason
/// is measured: a `//` or a trailing comma in Claude Code's `settings.json` makes that tool report a
/// Settings Error and ignore the file entirely. A reader that accepted them there would let tapkey
/// splice something the tool never reads and then report success.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tolerance {
    /// Strict JSON: no comments, no trailing comma. Claude Code's `settings.json`.
    Strict,
    /// JSONC: line and block comments and a trailing comma. Every one of OpenCode's config files —
    /// measured, the tolerance there belongs to the tool and not to the file's extension.
    Jsonc,
}

struct Parser<'a> {
    b: &'a [u8],
    i: usize,
    tolerance: Tolerance,
}

impl<'a> Parser<'a> {
    fn error(&self, message: &str) -> Error {
        Error::Syntax {
            offset: self.i,
            message: message.to_string(),
        }
    }

    fn skip_whitespace(&mut self) {
        loop {
            while matches!(self.b.get(self.i), Some(b' ' | b'\t' | b'\n' | b'\r')) {
                self.i += 1;
            }
            if self.tolerance != Tolerance::Jsonc || self.b.get(self.i) != Some(&b'/') {
                return;
            }
            match self.b.get(self.i + 1) {
                Some(b'/') => {
                    self.i += 2;
                    while !matches!(self.b.get(self.i), None | Some(b'\n')) {
                        self.i += 1;
                    }
                }
                Some(b'*') => {
                    self.i += 2;
                    while self.b.get(self.i).is_some()
                        && !(self.b.get(self.i) == Some(&b'*')
                            && self.b.get(self.i + 1) == Some(&b'/'))
                    {
                        self.i += 1;
                    }
                    // An unterminated block comment runs to the end, and the value that was being
                    // read is then missing — reported where it is missing rather than here.
                    self.i = (self.i + 2).min(self.b.len());
                }
                _ => return,
            }
        }
    }

    fn value(&mut self) -> Result<Node, Error> {
        match self.b.get(self.i) {
            Some(b'{') => self.object(),
            Some(b'[') => self.array(),
            Some(b'"') => {
                let start = self.i;
                let text = self.string()?;
                Ok(Node::Opaque {
                    span: start..self.i,
                    text: Some(text),
                })
            }
            Some(_) => {
                let start = self.i;
                self.scalar()?;
                Ok(Node::Opaque {
                    span: start..self.i,
                    text: None,
                })
            }
            None => Err(self.error("a value was expected")),
        }
    }

    fn object(&mut self) -> Result<Node, Error> {
        let start = self.i;
        self.i += 1; // '{'
        let mut members = Vec::new();
        let mut after_comma = false;
        loop {
            self.skip_whitespace();
            match self.b.get(self.i) {
                Some(b'}') => {
                    if after_comma && self.tolerance == Tolerance::Strict {
                        return Err(self.error("strict JSON has no trailing comma"));
                    }
                    self.i += 1;
                    return Ok(Node::Object {
                        members,
                        span: start..self.i,
                    });
                }
                Some(b'"') => {}
                _ => return Err(self.error("a key or a closing brace was expected")),
            }
            let key_start = self.i;
            let key = self.string()?;
            let key_span = key_start..self.i;

            self.skip_whitespace();
            if self.b.get(self.i) != Some(&b':') {
                return Err(self.error("a colon was expected after a key"));
            }
            self.i += 1;
            self.skip_whitespace();

            if let Some(previous) = members.iter().find(|m: &&Member| m.key == key) {
                let _ = previous;
                return Err(Error::DuplicateKey {
                    offset: key_start,
                    key,
                });
            }

            let value = self.value()?;
            let member_end = self.i;
            members.push(Member {
                key,
                key_span: key_span.clone(),
                span: key_span.start..member_end,
                value,
            });

            self.skip_whitespace();
            match self.b.get(self.i) {
                Some(b',') => {
                    self.i += 1;
                    after_comma = true;
                }
                Some(b'}') => after_comma = false,
                _ => return Err(self.error("a comma or a closing brace was expected")),
            }
        }
    }

    fn array(&mut self) -> Result<Node, Error> {
        let start = self.i;
        self.i += 1; // '['
        loop {
            self.skip_whitespace();
            match self.b.get(self.i) {
                Some(b']') => {
                    self.i += 1;
                    return Ok(Node::Opaque {
                        span: start..self.i,
                        text: None,
                    });
                }
                // Reaching the end mid-array is caught by `value()` below in any case; this
                // arm exists for the better message, which is why a mutation run reports it as
                // survivable. Verified: with it removed, every truncated form still refuses.
                None => return Err(self.error("an unterminated array")),
                _ => {}
            }
            self.value()?;
            self.skip_whitespace();
            match self.b.get(self.i) {
                Some(b',') => self.i += 1,
                Some(b']') => {}
                _ => return Err(self.error("a comma or a closing bracket was expected")),
            }
        }
    }

    fn string(&mut self) -> Result<String, Error> {
        if self.b.get(self.i) != Some(&b'"') {
            return Err(self.error("a string was expected"));
        }
        self.i += 1;
        let mut out = String::new();
        loop {
            match self.b.get(self.i) {
                None => return Err(self.error("an unterminated string")),
                Some(b'"') => {
                    self.i += 1;
                    return Ok(out);
                }
                Some(b'\\') => {
                    self.i += 1;
                    let escaped = *self
                        .b
                        .get(self.i)
                        .ok_or_else(|| self.error("a truncated escape"))?;
                    self.i += 1;
                    match escaped {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let hex = self
                                .b
                                .get(self.i..self.i + 4)
                                .ok_or_else(|| self.error("a truncated \\u escape"))?;
                            let code = u32::from_str_radix(
                                std::str::from_utf8(hex)
                                    .map_err(|_| self.error("a bad \\u escape"))?,
                                16,
                            )
                            .map_err(|_| self.error("a bad \\u escape"))?;
                            self.i += 4;
                            // A lone surrogate is kept as the replacement character: the key it
                            // names still matches, and tapkey never writes this value back.
                            out.push(char::from_u32(code).unwrap_or('\u{FFFD}'));
                        }
                        _ => return Err(self.error("an unknown escape")),
                    }
                }
                Some(&byte) => {
                    let start = self.i;
                    while matches!(self.b.get(self.i), Some(&c) if c != b'"' && c != b'\\') {
                        self.i += 1;
                    }
                    let raw = &self.b[start..self.i];
                    out.push_str(std::str::from_utf8(raw).map_err(|_| Error::Syntax {
                        offset: start,
                        message: "a string holds bytes that are not UTF-8".to_string(),
                    })?);
                    let _ = byte;
                }
            }
        }
    }

    fn scalar(&mut self) -> Result<(), Error> {
        let start = self.i;
        while matches!(
            self.b.get(self.i),
            Some(c) if !matches!(c, b',' | b'}' | b']' | b' ' | b'\t' | b'\n' | b'\r')
        ) {
            self.i += 1;
        }
        if self.i == start {
            return Err(self.error("a value was expected"));
        }
        match &self.b[start..self.i] {
            b"true" | b"false" | b"null" => Ok(()),
            other => {
                let text = std::str::from_utf8(other).map_err(|_| Error::Syntax {
                    offset: start,
                    message: "a value that is not UTF-8".to_string(),
                })?;
                text.parse::<f64>().map(|_| ()).map_err(|_| Error::Syntax {
                    offset: start,
                    message: format!("{text:?} is not a JSON value"),
                })
            }
        }
    }
}
