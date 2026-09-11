# Contributing

## The one rule that matters

**A compatibility finding becomes a test before it becomes a fix.** The command line
grammar contract lives in `crates/cli/tests/grammar.rs`, and the option table in
`crates/cli/src/table.rs`. When you discover that a real wkhtmltopdf command line behaves
differently here, add the failing case there first. That file is the specification; the
code is an attempt at it.

## Getting set up

```bash
cargo test --workspace          # everything, no browser needed
cargo run -- --extended-help    # every option the parser accepts
cargo run -- <args> --dump-parse  # how a command line was understood
```

Tests that need a real browser resolve one the same way the product does: the
`RCHTMLTOPDF_TEST_CHROMIUM` variable, then the system locations described in D09. Without a
browser they print a skip line and pass, so `cargo test` works anywhere. CI sets
`RCHTMLTOPDF_REQUIRE_CHROMIUM=1`, which turns that skip into a failure, so a pinned browser
that quietly disappears is caught rather than ignored.

## Before you open a pull request

```bash
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

## Pull requests

Merges are squashed, and the pull request **title** becomes the commit subject on `main`.
So the title follows [Conventional Commits](https://www.conventionalcommits.org/) and is
linted in CI, while the commits inside your branch are yours to make as messy as you like.

```
feat(browser): wait for document.fonts.ready before printing
fix(cli): take option values positionally so -h can be a footer
compat(table): correct the short alias for --page-size
```

Types: `feat`, `fix`, `compat`, `perf`, `docs`, `refactor`, `test`, `ci`, `build`, `chore`.
Scopes: `cli`, `core`, `browser`, `pdf`, `table`, `ci`, `docker`, `pkg`, `deps`.

`compat` is specific to this project. Use it for any change driven by matching wkhtmltopdf
behaviour, so the release notes can carry a compatibility section that migrating users read
first.

## Design decisions

Decisions live in `docs/decisions.md`, numbered `D01` onward, written in French. Amend them
by **adding a new numbered entry**, never by rewriting an old one. The record of what was
rejected and why is the point.

If your change contradicts an existing decision, say so in the pull request and add the new
entry in the same change.

## Minimum supported Rust version

1.85, matching `rust-version` in the workspace manifest and verified by CI. Raising it is a
deliberate decision, not a side effect of reaching for a new language feature.

## Licence

Contributions are dual licensed under MIT and Apache-2.0, as described in the README.
