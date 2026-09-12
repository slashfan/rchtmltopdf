# rchtmltopdf

> `wkhtmltopdf` interface, modern Chromium rendering.

> [!WARNING]
> **A weekend project. Do not use it in production.**
>
> This is written for the interest of writing it, and published in case the approach is
> useful to someone. It is not a maintained product: there is no release, no support, and
> no undertaking that any of it keeps working or that the command line stays as it is.
>
> It drives a browser over untrusted HTML, which is a thing to get wrong, and nobody has
> audited it. The threat model in [SECURITY.md](SECURITY.md) describes what the code tries
> to do — it is not evidence that it succeeds.
>
> If you need a supported `wkhtmltopdf` replacement, this is not one.

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

**It converts.** One document at a time, with the paper, margins, orientation, stylesheets
and headers you ask for.

```bash
rchtmltopdf --page-size A4 --margin-top 15mm \
    --footer-center 'Page [page] / [topage]' invoice.html invoice.pdf
```

A URL, a local file or standard input goes in; a file or standard output comes out. You need
a Chromium on the machine, which it finds without ever downloading one.

| Layer | Crate | State |
| --- | --- | --- |
| Grammar, option table, translation | `crates/cli` | Working |
| Units, page sizes, settings model | `crates/core` | Working |
| Finding, launching and driving Chromium | `crates/browser` | Working |
| Several documents, cover, table of contents | | V2 and V3 |
| Merge, metadata, outlines | `crates/pdf` | Not started |

Still early. Many wkhtmltopdf options are recognised and ignored with a warning rather than
honoured; `--extended-help` marks which. `--dump-parse` prints how a command line was
understood and exits, which is the quickest way to check an existing invocation before
running it.

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
