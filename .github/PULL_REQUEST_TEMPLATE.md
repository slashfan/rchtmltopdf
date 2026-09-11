<!-- The title becomes the commit subject on main. Conventional Commits, please. -->

## What and why

## Checklist

- [ ] `cargo fmt --all` and `cargo clippy --workspace --all-targets -- -D warnings` are clean
- [ ] Tests cover the change, and a compatibility finding was added to `crates/cli/tests/grammar.rs` first
- [ ] `crates/cli/src/table.rs` updated if an option's support changed
- [ ] A new numbered entry added to `docs/decisions.md` if this contradicts or extends a decision

Closes #
