//! Codex's `config.toml`, edited without becoming the change that shows up.
//!
//! `toml_edit` does the structural work — it keeps comments, spacing and item order, and it is the
//! editor cargo uses on its own manifests. What it does not keep is the **byte envelope**: measured
//! at 0.25.13, a bare parse-and-render with no edit at all strips a BOM, rewrites CRLF as LF and
//! adds a final newline the file did not have. [ADR-0003 in the ticket series] settled that all
//! three are preserved, and "the tool would normalise it anyway" is the argument ADR-0010 already
//! rejected when it refused a re-serialiser for Claude Code: it justifies *tapkey* causing the diff.
//!
//! So this module is a wrapper that takes the envelope off before parsing and puts it back after
//! rendering. Where `json` preserves those bytes for free — by never touching them — this preserves
//! them by restoration, and that difference is why the two are not behind one interface.

/// A parsed `config.toml`, remembering the byte envelope its bytes arrived in.
pub struct Document {
    envelope: Envelope,
    inner: toml_edit::DocumentMut,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    /// The file is not TOML. Codex reports these as `path:line:col` with a caret and refuses to
    /// start, so a file in this state is one tapkey must not write to either.
    Syntax(String),
    /// Parsing and rendering it unchanged did not give back the bytes it came in as, so this
    /// technique cannot promise `merge-never-own` here. The envelope covers the three
    /// normalisations we measured; this covers the ones we have not met yet.
    NotPreserved,
}

/// The three things `toml_edit` normalises away, kept so they can be put back.
struct Envelope {
    bom: bool,
    crlf: bool,
    final_newline: bool,
}

impl Envelope {
    /// Reads the envelope off the source and hands back the body with it removed.
    fn strip(bytes: &[u8]) -> (Envelope, Vec<u8>) {
        let bom = bytes.starts_with(&[0xEF, 0xBB, 0xBF]);
        let body = if bom { &bytes[3..] } else { bytes };

        // One CRLF anywhere makes the file a CRLF file. A file mixing both is already
        // inconsistent, and picking the majority would make our output depend on a count.
        let crlf = body.windows(2).any(|w| w == b"\r\n");
        let unified = if crlf {
            body.iter().copied().filter(|b| *b != b'\r').collect()
        } else {
            body.to_vec()
        };

        let final_newline = unified.last() == Some(&b'\n');
        (
            Envelope {
                bom,
                crlf,
                final_newline,
            },
            unified,
        )
    }

    /// Puts it back on a rendered body.
    fn restore(&self, rendered: String) -> Vec<u8> {
        let mut body = rendered;
        if !self.final_newline {
            while body.ends_with('\n') {
                body.pop();
            }
        }

        let mut out = Vec::new();
        if self.bom {
            out.extend_from_slice(&[0xEF, 0xBB, 0xBF]);
        }
        if self.crlf {
            for byte in body.bytes() {
                if byte == b'\n' {
                    out.push(b'\r');
                }
                out.push(byte);
            }
        } else {
            out.extend_from_slice(body.as_bytes());
        }
        out
    }
}

impl Document {
    pub fn parse(bytes: &[u8]) -> Result<Self, Error> {
        let (envelope, body) = Envelope::strip(bytes);
        let text = String::from_utf8(body).map_err(|e| Error::Syntax(e.to_string()))?;
        let inner: toml_edit::DocumentMut = text.parse().map_err(|e: toml_edit::TomlError| {
            Error::Syntax(e.to_string().lines().next().unwrap_or("").to_string())
        })?;

        // The pre-flight round trip. Rendering before any edit must reproduce the source exactly;
        // if it does not, this file holds something our technique silently changes, and the honest
        // answer is to refuse it rather than to hand back a file we damaged.
        let document = Document { envelope, inner };
        if document.to_bytes() != bytes {
            return Err(Error::NotPreserved);
        }
        Ok(document)
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        self.envelope.restore(self.inner.to_string())
    }
}
