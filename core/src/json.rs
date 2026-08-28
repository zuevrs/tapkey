//! A JSON document that remembers where every value sits in the original bytes.
//!
//! Edits are byte-range splices, never a re-serialisation: a tool's own layout, comments, key
//! order, BOM and line endings all survive, because tapkey must not be the edit that shows up
//! in a diff. Splices are collected against the original coordinates and applied together, so
//! the ranges a caller is told about mean the same thing before and after.

use std::ops::Range;

/// A parsed document and the pending changes to it.
pub struct Document {
    original: Vec<u8>,
    root: Node,
    splices: Vec<Splice>,
}

/// Why a document could not be parsed, or an edit could not be made.
#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    /// The bytes are not the format they claim to be.
    Syntax { offset: usize, message: String },
    /// The same key appears twice in one object. Parsers disagree about which one wins, so
    /// effective state cannot be promised and the file is refused rather than guessed at.
    DuplicateKey { offset: usize, key: String },
    /// Nothing at that path, and this operation does not create.
    Missing { path: String },
    /// Something is there, but not of a shape this operation can replace.
    NotAString { path: String },
}

struct Splice {
    range: Range<usize>,
    replacement: Vec<u8>,
}

enum Node {
    Object {
        members: Vec<Member>,
        #[allow(dead_code)]
        span: Range<usize>,
    },
    /// An array or a scalar. tapkey never reaches inside one, so its contents are not modelled
    /// — only where it begins and ends, which is what a splice needs.
    Opaque { span: Range<usize>, is_string: bool },
}

struct Member {
    key: String,
    #[allow(dead_code)]
    key_span: Range<usize>,
    value: Node,
}

impl Document {
    /// Parse `bytes` as strict JSON.
    pub fn parse(bytes: &[u8]) -> Result<Self, Error> {
        let start = if bytes.starts_with(b"\xEF\xBB\xBF") {
            3
        } else {
            0
        };
        let mut parser = Parser { b: bytes, i: start };
        parser.skip_whitespace();
        let root = parser.value()?;
        parser.skip_whitespace();
        if parser.i != bytes.len() {
            return Err(parser.error("trailing content after the top-level value"));
        }
        Ok(Document {
            original: bytes.to_vec(),
            root,
            splices: Vec::new(),
        })
    }

    /// Replace the string at `path`. Creates nothing: a path that is not already there is an
    /// error, because creating is a different decision with its own rules about placement.
    pub fn set_string(&mut self, path: &[&str], value: &str) -> Result<Range<usize>, Error> {
        let node = resolve(&self.root, path).ok_or_else(|| Error::Missing {
            path: path.join("."),
        })?;
        match node {
            Node::Opaque {
                span,
                is_string: true,
            } => {
                let range = span.clone();
                self.record(range.clone(), encode_string(value));
                Ok(range)
            }
            _ => Err(Error::NotAString {
                path: path.join("."),
            }),
        }
    }

    /// The document's bytes with every pending splice applied.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut ordered: Vec<&Splice> = self.splices.iter().collect();
        ordered.sort_by_key(|s| s.range.start);

        let mut out = Vec::with_capacity(self.original.len());
        let mut cursor = 0usize;
        for splice in ordered {
            out.extend_from_slice(&self.original[cursor..splice.range.start]);
            out.extend_from_slice(&splice.replacement);
            cursor = splice.range.end;
        }
        out.extend_from_slice(&self.original[cursor..]);
        out
    }

    fn record(&mut self, range: Range<usize>, replacement: Vec<u8>) {
        // Setting the same place twice replaces the earlier intent rather than stacking on it.
        self.splices.retain(|s| s.range != range);
        self.splices.push(Splice { range, replacement });
    }
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

struct Parser<'a> {
    b: &'a [u8],
    i: usize,
}

impl<'a> Parser<'a> {
    fn error(&self, message: &str) -> Error {
        Error::Syntax {
            offset: self.i,
            message: message.to_string(),
        }
    }

    fn skip_whitespace(&mut self) {
        while matches!(self.b.get(self.i), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.i += 1;
        }
    }

    fn value(&mut self) -> Result<Node, Error> {
        match self.b.get(self.i) {
            Some(b'{') => self.object(),
            Some(b'[') => self.array(),
            Some(b'"') => {
                let start = self.i;
                self.string()?;
                Ok(Node::Opaque {
                    span: start..self.i,
                    is_string: true,
                })
            }
            Some(_) => {
                let start = self.i;
                self.scalar()?;
                Ok(Node::Opaque {
                    span: start..self.i,
                    is_string: false,
                })
            }
            None => Err(self.error("a value was expected")),
        }
    }

    fn object(&mut self) -> Result<Node, Error> {
        let start = self.i;
        self.i += 1; // '{'
        let mut members = Vec::new();
        loop {
            self.skip_whitespace();
            match self.b.get(self.i) {
                Some(b'}') => {
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
            members.push(Member {
                key,
                key_span,
                value,
            });

            self.skip_whitespace();
            match self.b.get(self.i) {
                Some(b',') => self.i += 1,
                Some(b'}') => {}
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
                        is_string: false,
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
