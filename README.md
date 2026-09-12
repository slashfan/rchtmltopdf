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

**It converts.** One document or several, with the paper, margins, orientation, stylesheets,
and headers and footers you ask for — placeholders included. Several documents on one
command line come out as one PDF, in that order, each printed with its own options.

```bash
rchtmltopdf --page-size A4 --margin-top 15mm \
    --footer-center 'Page [page] / [topage]' invoice.html invoice.pdf
```

A URL, a local file or standard input goes in; a file or standard output comes out.

## Docker

The image carries the program, a pinned `chrome-headless-shell` and fonts, so nothing needs
installing:

```bash
docker run --rm -i --cap-add=SYS_ADMIN ghcr.io/slashfan/rchtmltopdf - - < page.html > out.pdf
```

It answers to `wkhtmltopdf` too, which is the point of the symlink:

```bash
docker run --rm -i --cap-add=SYS_ADMIN --entrypoint wkhtmltopdf \
    ghcr.io/slashfan/rchtmltopdf - - < page.html > out.pdf
```

### Why `--cap-add=SYS_ADMIN`

**Chromium's sandbox does not work in a default container**, and this image does not turn it
off for you (D33). A plain `docker run` fails, saying so and naming every way to fix it —
because the threat model is untrusted HTML (D10) and an image that quietly rendered it
unsandboxed would be the thing this project exists to improve on.

Three commands work, and each concedes something:

| | the browser's sandbox | Docker's own confinement |
| --- | --- | --- |
| `--cap-add=SYS_ADMIN` | kept | a broad capability granted |
| `--security-opt seccomp=unconfined` | kept | seccomp off |
| `rchtmltopdf --no-sandbox` | **off** | intact |

For HTML you generated yourself, the last one is reasonable and explicit. For HTML somebody
uploaded, it is not.

## You bring the browser

**This program never downloads anything.** Not at conversion time, and not on demand either:
fetching a browser would mean a TLS stack, a checksum and an archive unpacker inside a binary
whose entire networking story is otherwise "the browser does it" (D31). Your package manager
is better at this, and so is `@puppeteer/browsers`.

Any recent Chromium or Chrome will do:

```bash
apt install chromium                 # Debian, Ubuntu
dnf install chromium                 # Fedora
pacman -S chromium                   # Arch
brew install --cask chromium         # macOS
```

Or a pinned build, which is what CI uses and what the conformance suite measures against —
the version is in [`.chromium-version`](.chromium-version):

```bash
npx @puppeteer/browsers install chrome-headless-shell@<version>
```

`chrome-headless-shell` is preferred over a full browser when both are present: it starts
faster and carries no profile or GPU surface.

### Pointing at it

Most of the time you do not have to. It looks, in order, at `--chromium-path`, then
`RCHTMLTOPDF_CHROMIUM`, `CHROME_PATH`, `CHROMIUM_PATH` and `PUPPETEER_EXECUTABLE_PATH`, then
the usual system locations including `/Applications` bundles, then `RCHTMLTOPDF_CACHE_DIR`.

When you do, the path can be **the executable, a directory it was unpacked into, or a macOS
`.app` bundle** — whichever you happen to have:

```bash
rchtmltopdf --chromium-path /usr/bin/chromium               in.html out.pdf
rchtmltopdf --chromium-path ~/.cache/puppeteer/chrome-headless-shell/…  in.html out.pdf
rchtmltopdf --chromium-path "/Applications/Google Chrome.app"  in.html out.pdf
export CHROME_PATH=/opt/chrome
```

To check what it would use, and where it found it:

```console
$ rchtmltopdf --dump-chromium
path:    /usr/bin/chromium
source:  a system location
flavour: full browser, run with --headless
```

If it finds nothing, the error lists every path it tried and repeats the install commands
above.

| Layer | Crate | State |
| --- | --- | --- |
| Grammar, option table, translation | `crates/cli` | Working |
| Units, page sizes, settings model | `crates/core` | Working |
| Finding, launching and driving Chromium | `crates/browser` | Working |
| Several documents and covers, merged into one PDF | `crates/cli`, `crates/pdf` | Working |
| Table of contents | | V3 |
| Metadata, outlines | `crates/pdf` | Metadata working, outlines V2 |

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
