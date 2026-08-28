<!-- What this changes, and why. If it fixes an issue, link it. -->

## Checks

- [ ] `cargo test --workspace --all-targets` passes
- [ ] `cargo fmt --all --check` and `cargo clippy --workspace --all-targets -- -D warnings` pass
- [ ] Tests cover the change; an adapter change comes with golden fixtures
- [ ] No credential appears in a fixture, a log line, an error message or the diff
- [ ] Behavioural claims about a managed tool were measured against it, not read from its docs
