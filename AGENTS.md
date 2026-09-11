# rchtmltopdf

A drop-in replacement for `wkhtmltopdf`: the same command line, Chromium rendering.
The promise is functional CLI compatibility, never pixel parity.

## Where code belongs

Dependencies point one way: `cli`, `browser` and `pdf` all depend on `core`, and never on
each other.

| Crate | Owns |
| --- | --- |
| `core` | The document and settings model, units, page sizes, exit codes |
| `cli` | The wkhtmltopdf grammar, the option table, the binary |
| `browser` | Finding and launching Chromium, the protocol, waiting, printing |
| `pdf` | Merge, metadata, outlines. Empty until V1 |

**`core` has zero dependencies and keeps them.** That is what lets the model be tested with
no browser anywhere near it. When something needs a dependency, it belongs in another crate.

**No `lopdf` type escapes `pdf`**, so the PDF library can be replaced without touching
anything else (D12).

## The gate

Run this before committing, not after:

```bash
cargo fmt --all && cargo clippy --workspace --all-targets --all-features -- -D warnings && cargo test --workspace
```

Committing first and checking second costs a CI round trip every time. It has already cost
three.

## Four rules the code will not tell you

**Decisions are append-only.** `docs/decisions.md` holds D01 onward, and they are binding.
Amend one by adding a new numbered entry. The record of what was rejected, and why, is the
point of the file.

**A compatibility finding becomes a test before it becomes a fix**, in
`crates/cli/tests/grammar.rs`. That file is the specification; the parser is an attempt at
it.

**`Support::Implemented` in the option table means targeted, not working.** Nothing
translates a command line into settings yet, so sixty-one options claim more than they do.
Treat the marker as a plan until that layer lands.

**The sandbox stays on.** The threat model is untrusted HTML with filesystem reach (D10).
Only the environment says when a sandbox is unavailable, through
`RCHTMLTOPDF_TEST_NO_SANDBOX`, and only for tests.

## Tests

Browser-backed tests skip with a message when no browser is present, so `cargo test` works
anywhere. CI sets `RCHTMLTOPDF_REQUIRE_CHROMIUM` to turn that skip into a failure.

Write the assertion that fails against the broken code. Several tests here once asserted a
count was zero without ever asserting it was one, or compared file sizes as a proxy for
behaviour. Prefer a structural signal, and prove a new test earns its place by reverting the
fix and watching it fail.

## Deeper reading

Read these before changing behaviour they describe. Both are in French; the code, the tests
and everything on GitHub are in English.

- `docs/brief.md` — scope, V0 through V3, and what compatibility does and does not mean
- `docs/decisions.md` — D01 to D24, binding, with the alternatives that were rejected
- `docs/migration.md` — why a migrated document changes size, for anything touching layout
- `CONTRIBUTING.md` — the branch, pull request and Conventional Commit workflow

Module documentation carries the traps that cost real time: the descriptor plumbing in
`crates/browser/src/launch.rs`, the framing and session rules in `crates/browser/src/cdp/`,
and the ordering of the wait ladder in `crates/browser/src/render.rs`. Read the module doc
before editing one of those.
