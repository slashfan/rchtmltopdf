# rchtmltopdf

> `wkhtmltopdf` interface, modern Chromium rendering.

A drop-in replacement for [`wkhtmltopdf`](https://github.com/wkhtmltopdf/wkhtmltopdf),
archived since 2023, that keeps the command line and swaps the rendering engine for a
recent headless Chromium driven over the Chrome DevTools Protocol.

The promise is **functional CLI compatibility, not pixel-perfect output**. See
[docs/brief.md](docs/brief.md) for the scope, [docs/decisions.md](docs/decisions.md) for
the design decisions and the alternatives that were rejected, and
[docs/migration.md](docs/migration.md) for what changes when you switch.

Those two design documents are written in French; the code, the tests and everything on
GitHub are in English.

## State

Early. The pieces work; **the command line does not drive them yet**, so the binary still
refuses to convert.

| Layer | Crate | State |
| --- | --- | --- |
| Grammar and option table | `crates/cli` | Working |
| Command line to settings | `crates/cli` | Not started |
| Units, page sizes, settings model | `crates/core` | Working |
| Chrome DevTools Protocol client | `crates/browser` | Working |
| Finding and launching Chromium | `crates/browser` | Working |
| Waiting for a page to settle | `crates/browser` | Working |
| Printing to PDF | `crates/browser` | Working |
| Merge, metadata, outlines | `crates/pdf` | Not started |

The browser layer produces real PDFs today, with the right paper size, margins, orientation
and stylesheets. What is missing is the translation from a command line into the settings
that drive it, which is why `rchtmltopdf page.html out.pdf` still tells you conversion is
not implemented.

`--help` lists the options the parser accepts. Until the translation layer lands, options
marked as implemented describe the target rather than current behaviour; `--extended-help`
flags the ones that are accepted and ignored on purpose.

What you can do today:

```bash
cargo run -- --page-size A4 --footer-center 'Page [page] / [topage]' \
    https://example.com/invoice/42 invoice.pdf --dump-parse
```

`--dump-parse` prints how a command line was understood and exits. It is the quickest
way to check that an existing `wkhtmltopdf` invocation will be accepted.

## Why the parser is hand-written

wkhtmltopdf's grammar is not a conventional CLI:

```text
rchtmltopdf [GLOBAL OPTION]... [OBJECT]... <output file>
```

Objects are `page <input>`, `cover <input>`, `toc`, or a bare input. The last positional
argument is always the output. Object options attach to the object they follow, and
object options written before the first object become defaults for all of them. Options
take zero, one or two values, and a bare number in a length argument means millimetres.

Every option wkhtmltopdf documents is recognised, whether or not it does anything.
Unimplemented ones warn on stderr and are ignored, because a wrapper such as KnpSnappy
emitting an unexpected flag must not break a production application.

## Development

```bash
cargo test --workspace
cargo clippy --all-targets
cargo fmt --all
```

The grammar contract lives in `crates/cli/tests/grammar.rs`. New compatibility findings
belong there first. [CONTRIBUTING.md](CONTRIBUTING.md) has the rest, and
[SECURITY.md](SECURITY.md) carries the threat model, which is worth reading before you
render HTML you did not write.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or
  <https://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or <https://opensource.org/licenses/MIT>)

at your option.

`SPDX-License-Identifier: MIT OR Apache-2.0`

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted for
inclusion in the work by you, as defined in the Apache-2.0 license, shall be dual licensed
as above, without any additional terms or conditions.
