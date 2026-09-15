# Contributing

Working with a coding agent: `AGENTS.md` carries what it needs and points back here.

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

The conformance suite runs the built binary against a real browser, so it needs the binary
to exist. `cargo test --workspace` builds it for you; running the package on its own does
not, because cargo builds a dependency's library and not its binaries:

```bash
cargo build -p rchtmltopdf && cargo test -p rchtmltopdf-conformance
```

Tests that need a real browser resolve one the same way the product does. In order:
`--chromium-path`, then `RCHTMLTOPDF_CHROMIUM`, `CHROME_PATH`, `CHROMIUM_PATH` or
`PUPPETEER_EXECUTABLE_PATH`, then the system locations described in D09.

Without a browser they print a skip line and pass, so `cargo test` works anywhere. CI sets
`RCHTMLTOPDF_REQUIRE_CHROMIUM=1`, which turns that skip into a failure, so a browser that
quietly disappears is caught rather than ignored for months.

CI also sets `RCHTMLTOPDF_TEST_NO_SANDBOX=1`, because a GitHub runner cannot give Chromium a
sandbox and it refuses to start rather than run without one. That is a property of the
runner. The product never gives the sandbox up on its own (D10).

## Before you open a pull request

```bash
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
```

The last one is easy to forget and CI runs it: rustdoc rejects a broken or redundant
intra-doc link, and no other command here looks at one.

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
Scopes: `cli`, `core`, `browser`, `pdf`, `table`, `conformance`, `ci`, `docker`, `pkg`, `deps`.

`compat` is specific to this project. Use it for any change driven by matching wkhtmltopdf
behaviour, so the release notes can carry a compatibility section that migrating users read
first.

## Design decisions

Decisions live in `docs/decisions.md`, numbered `D01` onward, written in French. Amend them
by **adding a new numbered entry**, never by rewriting an old one. The record of what was
rejected and why is the point.

If your change contradicts an existing decision, say so in the pull request and add the new
entry in the same change.

## Publishing a version

**The version in `Cargo.toml` commands** (D50). Publishing means raising it in a pull request
like any other change; the merge does the rest: tag, four binaries, the Docker image, the
release, the checksums.

```bash
git switch -c version-0.2.0 main
# raise `version` in the workspace manifest and the three internal dependency pins that
# repeat it, move the `[Unreleased]` section of CHANGELOG.md under the new number, then
# reflect the version in Cargo.lock:
cargo update --workspace
```

There is **no tag to push**: `gh release create` lays it on the merge commit itself. The
gesture that could be forgotten is gone, and with it any drift between what the binary
announces and what is published.

Three guards, each on a real failure mode:

| What could happen | What catches it |
| --- | --- |
| Raising the version without updating `Cargo.lock` | `cargo build --locked`, everywhere in CI |
| Moving the version back below the latest release | the `version` job, on the pull request |
| Pushing a tag that does not match `Cargo.toml` | the `version` job of the release, before any build |

A merge that does not touch the version publishes nothing: the workflow sees the tag already
exists and stops without building. A hand-pushed tag is still accepted, to republish, but it
must match `Cargo.toml`.

The release notes are the README's caveats followed by that version's section of
`CHANGELOG.md`, which is why the changelog is kept as it is written: the **Compatibility**
section is the one a migrating user reads first.

The release workflow also runs, without publishing, on any pull request that touches it, the
manifests, the Dockerfile or the browser pin, and it can be dispatched by hand: the archives
come out as workflow artifacts and no release is created.

## Minimum supported Rust version

1.88, matching `rust-version` in the workspace manifest and verified by CI. Raising it is a
deliberate decision, not a side effect of reaching for a new language feature — the move from
1.85 is D30, and it was forced by lopdf rather than chosen for the syntax.

## Licence

Contributions are dual licensed under MIT and Apache-2.0, as described in the README.
