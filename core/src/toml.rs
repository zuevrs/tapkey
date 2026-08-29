//! Codex's `config.toml`, edited without becoming the change that shows up.
//!
//! `toml_edit` does the structural work — it keeps comments, spacing and item order, and it is the
//! editor cargo uses on its own manifests. What it does not keep is the **byte envelope**: measured
//! at 0.25.13, a bare parse-and-render with no edit at all strips a BOM, rewrites CRLF as LF and
//! adds a final newline the file did not have. All three are preserved by decision, and "the tool
//! would normalise it anyway" is the argument ADR-0010 already rejected when it refused a
//! re-serialiser for Claude Code: it justifies *tapkey* being the one who caused the diff.
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
    /// A step on the way to the key we own is occupied by something that is not a table, so
    /// writing would mean replacing whatever the person put there.
    NotATable(String),
}

/// The three things `toml_edit` normalises away, kept so they can be put back.
struct Envelope {
    bom: bool,
    crlf: bool,
    final_newline: bool,
    /// Offsets of the newlines in the stripped body, so a span over it can be mapped back to the
    /// file the caller handed us. Only meaningful when `crlf`, where each one stood for two bytes.
    newlines: Vec<usize>,
}

impl Envelope {
    /// Maps an offset in the stripped body to the same place in the original bytes.
    fn to_original(&self, offset: usize) -> usize {
        let bom = if self.bom { 3 } else { 0 };
        let carriage_returns = if self.crlf {
            self.newlines.partition_point(|n| *n < offset)
        } else {
            0
        };
        offset + bom + carriage_returns
    }

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

        // An empty input is not a file that lacks a trailing newline; it is a file with no
        // envelope at all. A file tapkey creates should look like every other file on the disk,
        // and Codex writes a final newline too.
        let final_newline = unified.is_empty() || unified.last() == Some(&b'\n');
        let newlines = unified
            .iter()
            .enumerate()
            .filter(|(_, b)| **b == b'\n')
            .map(|(i, _)| i)
            .collect();
        (
            Envelope {
                bom,
                crlf,
                final_newline,
                newlines,
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

    /// The string at `path`, or `None` if nothing is there or what is there is not a string.
    /// A number read as a model name would be a worse answer than no answer.
    pub fn get_string(&self, path: &[&str]) -> Option<&str> {
        let mut item: &toml_edit::Item = self.inner.as_item();
        for step in path {
            item = item.get(step)?;
        }
        item.as_str()
    }

    /// The keys one level below `path`, for enumerating a table whose entries are named by the
    /// person — a registry being the ordinary case.
    pub fn keys_at(&self, path: &[&str]) -> Vec<String> {
        let mut table = self.inner.as_table();
        for step in path {
            table = match table.get(step).and_then(|item| item.as_table()) {
                Some(t) => t,
                None => return Vec::new(),
            };
        }
        table.iter().map(|(key, _)| key.to_string()).collect()
    }

    /// Sets the string at `path`, creating the tables along the way.
    ///
    /// The value is replaced **in place, carrying its `decor`** — never by assigning a fresh item.
    /// Measured at `toml_edit` 0.25.13: an ordinary assignment keeps the key's alignment and
    /// silently drops a trailing comment on the same line, which is somebody's content.
    pub fn set_string(&mut self, path: &[&str], value: &str) -> Result<(), Error> {
        let (last, parents) = path.split_last().expect("a path has at least one step");

        let mut table = self.inner.as_table_mut();
        for step in parents {
            let entry = table.entry(step).or_insert_with(|| {
                let mut created = toml_edit::Table::new();
                // Implicit, so `[model_providers]` is not rendered as an empty section above
                // `[model_providers.tapkey-zai]`. Codex would not have written that header, and a
                // section that appears in somebody's file because tapkey needed a parent is
                // exactly the kind of visible change ADR-0004 exists to prevent.
                created.set_implicit(true);
                toml_edit::Item::Table(created)
            });
            table = match entry.as_table_mut() {
                Some(t) => t,
                None => return Err(Error::NotATable(step.to_string())),
            };
        }

        match table.get_mut(last).and_then(|item| item.as_value_mut()) {
            Some(existing) => {
                let decor = existing.decor().clone();
                let mut replacement = toml_edit::Value::from(value);
                *replacement.decor_mut() = decor;
                *existing = replacement;
            }
            None => {
                table.insert(last, toml_edit::value(value));
            }
        }
        Ok(())
    }

    /// Removes the key at `path`. A key that was never there is not an error: "no assignment"
    /// means the same thing whether or not the tool had written one.
    pub fn remove(&mut self, path: &[&str]) -> Result<(), Error> {
        let (last, parents) = path.split_last().expect("a path has at least one step");

        let mut table = self.inner.as_table_mut();
        for step in parents {
            table = match table.get_mut(step).and_then(|item| item.as_table_mut()) {
                Some(t) => t,
                None => return Ok(()),
            };
        }
        table.remove(last);
        Ok(())
    }
}

/// Where the members of a TOML file are, in the coordinates of the bytes it was read from.
///
/// This is the golden harness's tool, not the adapter's: the harness states `merge-never-own`
/// mechanically as `before` minus the owned spans equalling `after` minus the same. It is a
/// separate type because `toml_edit`'s **editable** document carries no spans at all — not merely
/// after an edit, but ever. Only the read-only document has them, so reading where things are and
/// changing them are two different objects there.
pub struct Spans {
    envelope: Envelope,
    inner: toml_edit::Document<String>,
}

impl Spans {
    pub fn of(bytes: &[u8]) -> Result<Self, Error> {
        let (envelope, body) = Envelope::strip(bytes);
        let text = String::from_utf8(body).map_err(|e| Error::Syntax(e.to_string()))?;
        let inner: toml_edit::Document<String> =
            text.parse().map_err(|e: toml_edit::TomlError| {
                Error::Syntax(e.to_string().lines().next().unwrap_or("").to_string())
            })?;
        Ok(Spans { envelope, inner })
    }

    /// The byte range of the whole member at `path` — key, separator and value — mapped back to
    /// the bytes handed in. `toml_edit` reports over the body, which has had its envelope stripped,
    /// so every offset is short by the BOM and by one more for each CRLF before it. An unmapped
    /// span would excise the wrong bytes and leave the property passing while checking nothing.
    /// The byte range of a whole **table**: its `[header]` line and everything under it.
    ///
    /// A member's span covers a key and its value, which for a table is only the header — the body
    /// sits outside it. The harness needs the whole thing, because a table tapkey created is ours
    /// entirely while every key in it is ours, and cutting only the header would leave our own
    /// keys behind on one side of the comparison.
    pub fn table(&self, path: &[&str]) -> Option<std::ops::Range<usize>> {
        let mut item = self.inner.as_item();
        for step in path {
            item = item.get(step)?;
        }
        let table = item.as_table()?;

        // Walk back from the header key to the start of its line, so the `[` and any indentation
        // go with it.
        let header = item.span()?.start;
        let body = self.inner.raw().as_bytes();
        let line_start = body[..header]
            .iter()
            .rposition(|b| *b == b'\n')
            .map_or(0, |i| i + 1);

        let end = table
            .iter()
            .filter_map(|(key, _)| table.get_key_value(key))
            .filter_map(|(_, value)| value.span())
            .map(|s| s.end)
            .max()
            .unwrap_or(item.span()?.end);

        Some(self.envelope.to_original(line_start)..self.envelope.to_original(end))
    }

    pub fn member(&self, path: &[&str]) -> Option<std::ops::Range<usize>> {
        let (last, parents) = path.split_last()?;

        let mut table = self.inner.as_table();
        for step in parents {
            table = table.get(step)?.as_table()?;
        }
        let (key, item) = table.get_key_value(last)?;
        Some(
            self.envelope.to_original(key.span()?.start)
                ..self.envelope.to_original(item.span()?.end),
        )
    }
}
