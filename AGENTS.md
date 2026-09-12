# rchtmltopdf

A drop-in replacement for `wkhtmltopdf`: the same command line, Chromium rendering.
The promise is functional CLI compatibility, never pixel parity.

## Where code belongs

Dependencies point one way: `cli`, `browser` and `pdf` all depend on `core`, and never on
each other. `conformance` is a test harness that sits outside that rule: it is published
nowhere, and it depends on whatever it needs to drive the binary.

| Crate | Owns |
| --- | --- |
| `core` | The document and settings model, units, page sizes, exit codes |
| `cli` | The wkhtmltopdf grammar, the option table, the binary |
| `browser` | Finding and launching Chromium, the protocol, waiting, printing |
| `pdf` | Merge, metadata, outlines. Empty until V1 |
| `conformance` | The browser-backed suite: the real binary, a pinned Chromium, fixtures |

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

## Five rules the code will not tell you

**Decisions are append-only.** `docs/decisions.md` holds D01 onward, and they are binding.
Amend one by adding a new numbered entry. The record of what was rejected, and why, is the
point of the file.

**A compatibility finding becomes a test before it becomes a fix**, in
`crates/cli/tests/grammar.rs`. That file is the specification; the parser is an attempt at
it.

**`Support::Implemented` means honoured end to end, and a test holds it there.** It used to
mean "the command line understands it", and the audit (#19) found that 38 of the 58 options
carrying the marker did nothing at all: each filled a field in the settings model that no
part of a conversion ever read. Those are now `Planned`.

What keeps it true is `crates/browser/src/plan.rs`, a pure description of everything a
conversion would do, built without starting anything. `crates/cli/tests/plan.rs` writes each
option on its own command line and requires the plan to change — and requires a `Planned`
option never to change it (D27). **So nothing above `plan` may decide anything on its own**:
`Page::prepare` sends what `plan::prepare` decided, `print_to_pdf` what `plan::print`
decided, `load` consults `LoadPlan`. Read a settings field directly in one of them and the
plan stops describing the conversion, with no test to notice.

Promoting an option to `Implemented` therefore means making the plan change, not editing the
table. A `Planned` option that fills the settings model and is ignored by the conversion is
a normal half-built state, not a bug.

**The option table has been reconciled against a real binary, and stays that way.**
`crates/cli/tests/fixtures/wkhtmltopdf-0.12.6.1-extended-help.txt` is the verbatim help of
wkhtmltopdf 0.12.6.1, and `reference_help.rs` holds the table to it: every long name, short
alias, arity, scope and section. Adding an option wkhtmltopdf does not have now fails a test.
Re-capture the fixture only from a real binary, following `fixtures/README.md`.

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

### The conformance suite

`crates/conformance` runs the built binary against a real browser and asserts on the bytes
it produces (D15). Two things about it surprise people:

```bash
cargo build -p rchtmltopdf          # the suite runs the binary, and -p will not build it
cargo test -p rchtmltopdf-conformance
```

`cargo test --workspace` does both. `cargo test -p rchtmltopdf-conformance` on its own
builds the cli *library* and not its binary, so the harness says which command is missing
rather than failing with "file not found".

**In CI the browser must be the pinned one, and the harness checks.** D09 puts system
locations above the download cache, so a runner that ships its own Chromium wins over a
freshly downloaded pin. `RCHTMLTOPDF_CHROMIUM` names the pinned build, and when
`RCHTMLTOPDF_REQUIRE_CHROMIUM` is set the harness compares the browser's reported version
against `.chromium-version` and fails if they differ. A wrong browser used to be as quiet as
a missing one.

**`inspect` is the only module that knows lopdf exists (D25).** Test bodies see `Rect`, a page
count and a `String`. `crates/pdf` stays empty until the product needs it (#29), so its first
public API is shaped by merging and metadata rather than by what a test wanted to measure.

**Two things a PDF will not tell you.** A margin has no entry of its own — it is an offset
applied to content — so a fixture paints a block filling its content area and where that
block lands *is* the margin. And `re` operands are in the current transformation matrix, not
in page space, so `inspect` tracks the matrix stack; reading the operands raw gives numbers
that look plausible and are wrong.

**Chromium does not print the paper size it was asked for.** Every media box it writes is a
multiple of 0.24 pt (1/300 inch), and the requested size is moved onto that grid — by up to
0.91 pt across twenty-four measured widths, in either direction. Which grid point it picks
was not worked out, so treat `inspect::TOLERANCE` as a measured bound rather than a model,
and read its doc comment before changing it. It is not licence to be vague: a size a
millimetre out still fails.

**Fixtures carry their own font, and must keep doing so.** Line wrapping follows font
metrics, so a fixture asking for `sans-serif` is measured against a different typeface on a
runner, on a Mac and in the Docker image, and a page count that passes locally fails in CI
for a reason unrelated to the change. `fixture::document` embeds the vendored font as a
`data:` URI and asks for a family name no system font answers to, with no fallback. See
`crates/conformance/fixtures/fonts/README.md`, and do not add a fallback to make something
render.

## Deeper reading

Read these before changing behaviour they describe. Both are in French; the code, the tests
and everything on GitHub are in English.

- `docs/brief.md` — scope, V0 through V3, and what compatibility does and does not mean
- `docs/decisions.md` — D01 to D27, binding, with the alternatives that were rejected
- `docs/migration.md` — why a migrated document changes size, for anything touching layout
- `CONTRIBUTING.md` — the branch, pull request and Conventional Commit workflow

Module documentation carries the traps that cost real time: the descriptor plumbing in
`crates/browser/src/launch.rs`, the framing and session rules in `crates/browser/src/cdp/`,
and the ordering of the wait ladder in `crates/browser/src/render.rs`. Read the module doc
before editing one of those.
