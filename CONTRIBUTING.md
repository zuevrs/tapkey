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

Each one is proved with golden fixtures: a real-world configuration file, the expected file after a
switch, and the case where the tool had already rewritten its own configuration first. They are real
files on disk compared byte for byte, because a file reads in a pull request diff and a snapshot blob
does not. Expectations can be regenerated with a named command, which rewrites only the expected
files and still fails the run, so a broken expectation cannot be blessed and go green in one step.

Where a tool's documented behaviour decides what tapkey writes, measure it against the tool rather
than reading it. Documentation states intent; the shipped binary states the resolution order, and we
have found them to disagree more than once.

## Pull requests

Small and single-purpose is easier to review than complete. Please include tests, describe what you
measured if a behavioural claim is involved, and keep the commit history readable — it does not need
to be one commit.

## Code of conduct

Be decent. Disagree about the work, not about the person.
