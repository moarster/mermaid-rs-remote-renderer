## Summary
<!-- What does this PR change, and why? Keep it short — diffs explain the what. -->

## Test plan
<!-- How did you verify this works? -->
- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --all-targets --locked -- -D warnings`
- [ ] `cargo test --locked --all-targets`
- [ ] Other (describe):

## Related
<!-- Closes #123, references #456, etc. -->

## Checklist
- [ ] Change is focused — one logical thing per PR.
- [ ] New behavior is covered by a test (unit or `tests/http_endpoints.rs`).
- [ ] No deployment specifics (reverse proxy, hostnames, ACME) in repo files.
- [ ] If this affects the renderer itself, I've considered whether the fix
      belongs in [`mermaid-rs-renderer`](https://github.com/1jehuang/mermaid-rs-renderer)
      instead.
