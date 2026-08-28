# Contributing to tapkey

Thanks for looking. The project is in early development and the engine is being built before the
apps, so most of what is here is `core/`.

## Layout

- `core/` — the engine: profiles, adapters, splicers, atomic writes, backups, credentials. It knows
  nothing about any user interface, which is what makes it testable on its own.
- `app/` — the user interface, once the engine is proven. It holds no config-writing logic.

## Building and testing

Rust stable, edition 2024, MSRV 1.85. `rust-toolchain.toml` pins the channel, so `rustup` will fetch
what is needed.

```sh
cargo test --workspace --all-targets
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
```

There is a pre-commit hook that runs the format check, since a formatting failure in CI tells
you nothing about your change. Enable it with `git config core.hooksPath .githooks`.

CI runs all three on Ubuntu and macOS, plus the doc tests. Linux is built from the first commit even
though macOS ships first: portability that is not compiled has already rotted by the time anyone
checks. A pull request is expected to be green on both.

## What the engine guarantees

These hold on every path, and a change that breaks one is a bug however convenient it looks.

**It changes only the keys it owns.** Every managed file is read live, the keys that select a
provider or a model are replaced in place as byte ranges, and everything else is written back
unchanged — including layout, comments, key order, BOM and line endings. Tools rewrite their own
configuration constantly; a whole-file template would silently destroy whatever the user had. The
test that matters most asserts that the untouched bytes survive exactly.

**A switch is all or nothing.** One switch usually touches several files across several tools. Any
failure restores every file already written and reports that it rolled back. Half-applied is the
worst outcome available: the person believes they moved, and one tool is still billing the old
provider.

**Writes are atomic.** A temporary file in the destination's own directory, flushed to the medium,
renamed over the target, and then the directory itself is flushed — with the data durable but the
directory entry still cached, a power loss returns the file to its old contents. `F_FULLFSYNC` on
macOS, `fdatasync` on Linux, `FlushFileBuffers` on Windows; plain `fsync` is not enough on macOS,
where it returns once the data reaches the drive's cache rather than the platter.

**It reports what is in effect, not what it wrote.** State is resolved across every scope a tool
reads and reported as the resolved chain. Where tapkey's intent and the tool's reality disagree,
that disagreement is the useful thing, and hiding it would defeat the point of the app.

**Secrets stay out of the repository.** Credentials live in the system keychain. They never appear
in logs, in error messages, or in test fixtures — fixtures come from real configuration files with
every credential replaced by an obviously fake placeholder, and a test fails the build if something
key-shaped is ever committed.

## Adding or changing an adapter

An adapter is finished when it back-fills a missing file, writes atomically, takes a backup, reads
effective state, detects drift and restores — each under test.

Each one is proved with golden fixtures in `core/tests/fixtures/<tool>/<case>/`. A case is a
self-contained directory: `request.json` is the real wire envelope, `profiles.json` its own
profiles, `before/` a small mirrored filesystem, `after/` the expected result — always complete,
even when identical — and `fail.json` optionally declares that one write refuses. A directory
exists, therefore a test exists; `build.rs` generates one per case so a failure names its case and
`cargo test crlf` selects it.

Four properties run on every case, not only in cases written for them: applying twice moves no
byte, everything outside the keys tapkey owns is identical, restoring the backup returns the tree
exactly, and no temporary file is left behind.

```sh
UPDATE_GOLDEN=1 cargo test --test golden     # rewrites expectations, and still fails
```

It rewrites only expectations, never an input, and the run it happens in is red — read the diff and
run again. A new case with no expectation fails too, printing where its output was written, because
otherwise a case can arrive in a pull request carrying an expectation nobody chose.

Where a tool's documented behaviour decides what tapkey writes, measure it against the tool rather
than reading it. Documentation states intent; the shipped binary states the resolution order, and we
have found them to disagree more than once.

## Proving a test can fail

A test that has never been red is not known to work. Before trusting one, break the thing it
guards and check that *that* test is the one which fails. This is not ceremony: writing the atomic
write seam, three of five deliberately injected defects went undetected on the first attempt — a
test staged the wrong claim, another never reached the code path it named, and a third asserted
something no implementation could get wrong.

`cargo mutants` automates the same idea and is worth running over a module you have just written.
Two things to know before reading its output. It does not respect `cfg`, so on macOS it reports
every `#[cfg(not(unix))]` and Linux-only branch as untested — on a project built around a platform
seam that is most of the noise. And a mutation that makes a parser loop forever is reported as a
timeout rather than a survivor; those are not gaps either. What is left after removing both is
usually real: a first run over the JSON reader reported 36 survivors, of which the genuine ones
were that no test parsed a string containing an escape, and none parsed an array at all.

## Pull requests

Small and single-purpose is easier to review than complete. Please include tests, describe what you
measured if a behavioural claim is involved, and keep the commit history readable — it does not need
to be one commit.

## Code of conduct

Be decent. Disagree about the work, not about the person.
