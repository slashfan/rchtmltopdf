# Changelog

All notable changes to this project are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this
project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

A **Compatibility** section carries anything that changes how a wkhtmltopdf command line
behaves. If you are migrating, that is the section to read.

## [Unreleased]

Nothing released yet.

The binary converts one document: a URL, a local file or standard input, to a file or
standard output, with paper size, margins, orientation, zoom, backgrounds, screen or print
stylesheets, whether scripts run, and how long to wait before printing. Options it does not
act on are accepted and warned about rather than breaking a command line that uses them.

Several documents, covers and a table of contents are not built. Neither is most of the
option surface beyond what V0 needed: the outline and everything a table of contents needs
are understood on the command line and not yet acted on.

### Added

**Static musl binaries for x86_64 and aarch64**, built by a tag. Each is statically linked, so
it runs wherever the kernel does — a distroless image, or a machine whose glibc predates the
one it was built on — and each archive carries a `wkhtmltopdf` symlink, which is why they are
tars rather than zips (D13).

Both are built on a runner of their own architecture rather than under emulation, and the
workflow asserts the three things that would otherwise be found out by whoever downloaded one:
that the binary is genuinely static, that the symlink survived being archived, and that it
runs (D32).

No release has been cut yet.

### Changed

**The binary will never download a browser**, and no longer promises a subcommand that would
(D31). Fetching one would mean a TLS stack, a checksum and an archive unpacker inside a
program whose entire networking story is otherwise "the browser does it" — and it would put
the static musl build at risk for a convenience `apt`, `brew` and `@puppeteer/browsers`
already provide.

What replaces it is the part that was actually missing:

- **The README says how to install one**, four ways, with the pinned version for the one CI
  uses.
- **The "could not find Chromium" error repeats those commands**, with the pin, after listing
  every path it tried. It is the one message a first-time user is guaranteed to read.
- **A path may now be a directory or a macOS `.app` bundle**, not only an executable. Both are
  what people actually have to hand — `/Applications/Google Chrome.app`, or whatever
  `@puppeteer/browsers` unpacked — and both used to be refused for not being a file.
- **`--dump-chromium`** reports which browser would be used and where it was found, and says
  so when an explicit `--chromium-path` was not a browser and the search carried on.

### Changed

**The minimum supported Rust version is 1.88**, up from 1.85. `lopdf` requires it and D12
chose lopdf for the PDF layer, so the choice was between carrying an old PDF library to
protect a number nobody had asked for and moving the number (D30).

### Added

**The defaults are proved in a real conversion, not only in the model.** A command line with
no options at all produces A4 portrait at 595.28 by 841.89 points with ten-millimetre margins,
screen stylesheets, backgrounds on, scripts running, and a 200ms settling delay — every one of
them the opposite of Chromium's own (D03). The settings model asserted the same numbers and
could not have caught a default that never reached the print call.

One of those assertions needed a new instrument. `--no-background` does not stop Chromium
emitting a block's rectangle: it emits the same rectangle in the same place and fills it
**white**, so the conformance harness now reads fill colour as well as geometry. The first
version of that test passed whether the option worked or not.

### Added

**The finished file says what it is.** `--title` sets the document's title, the producer names
this program and its version, and the document carries a valid PDF creation date. The print
call takes the title from the document's own `<title>` and offers no override, so the only way
to honour the option is to rewrite the file afterwards. **Without the option the document keeps
the title it had**: the browser derived it from the `<title>` element, and for most documents
that is the only title there will ever be.

That rewrite is the first real code in `crates/pdf`, and it matters more than it looks: it
establishes the read, modify, write path the V2 header overlay and the multi-document merge
both depend on. Object renumbering and cross-reference regeneration are easier to get right on
one document than to discover part-way through a merge. A conformance test round-trips a real
three-page Chromium PDF and checks the page count, the text and every page's paper — a
hand-built fixture cannot find what Chromium's cross-reference streams would.

A title outside ASCII is written as UTF-16 with a byte order mark, because a PDF string
without one is Latin-1 and `Facture n°42` is not.

### Added

**A failure says what wkhtmltopdf would have said.** Chromium's `net::ERR_*` codes and HTTP
statuses are mapped to the Qt names wkhtmltopdf printed — `ContentNotFoundError`,
`HostNotFoundError`, `ConnectionRefusedError`, `TimeoutError`, `ContentAccessDenied`,
`RemoteHostClosedError`, `ProtocolUnknownError`, `UnknownContentError` — because applications
grep for those strings and will go on doing it after the program underneath has changed. An
unmapped code becomes `UnknownContentError` rather than being dropped.

**`--load-error-handling` and `--load-media-error-handling` decide what a failure costs.**
A main document that fails is exit 1 and no PDF; `ignore` prints whatever did load. A failed
subresource under `abort` writes the PDF **and** exits 1, with wkhtmltopdf's exact line —
`Exit with code 1 due to network error: <Name>` — unprefixed, because that line is a
contract. `skip` reports and exits 0; `ignore`, the default for media, says nothing.

A 404 is now a failure. It never was to Chromium, because the bytes came back and an error
page is a page; it was to Qt, which is where `ContentNotFoundError` comes from. A missing
**favicon** is not: Chromium asks for one on every navigation and wkhtmltopdf never did, so
counting it would fail a conversion for a file the document never mentioned.

**Progress lines**, in wkhtmltopdf's shape and on stderr: `Loading page (1/2)`,
`Printing pages (2/2)`, `Done`. Shown at `info`, which is the default, and silenced by `-q` —
which now means a successful run says nothing at all.

`--version` names this program and never wkhtmltopdf (D29). A distinct name exists partly so
version checks are not muddied, and passing one by lying about the renderer would be worse
than failing it.

### Added

**Cookies, custom headers, HTTP credentials and the proxy are honoured.**
`--cookie` values are decoded before they are set, as the option's own help says they arrive;
`--username` and `--password` answer a 401 rather than being sent ahead of one; `--proxy`
becomes a launch flag.

Two asymmetries worth knowing, both wkhtmltopdf's rather than ours:

- **`--custom-header` does not reach subresources** unless `--custom-header-propagation` is
  given. Without it the header is on the document's own request and on nothing else.
- **`--cookie` needs a document fetched over http or https.** A local document has no origin
  to scope a cookie to, so the option is reported and ignored rather than fatal.

`--proxy` is process-wide where the table scopes it per object, which is the same thing while
one document converts. **Chromium bypasses the proxy for localhost** whatever it is told,
which is worth knowing before testing one.

A rejected password now fails the conversion. Cancelling the second challenge stops Chromium
asking for ever, and it also makes the server's own 401 body the response — which renders
perfectly well as a page, and is not the document anybody asked for.

**A main document that fails to load is reported, whatever failed.** `Page.navigate` carries
an error for a host that does not resolve and nothing at all for a proxy that refuses the
connection, so the network events are watched instead. A failed *subresource* still produces
a PDF, which is what `--load-media-error-handling` will make a choice (#28).

### Added

**Header and footer placeholders are substituted**, so
`--footer-center 'Page [page] / [topage]'` — the example in the README, in the brief and in
the grammar tests — prints "Page 1 / 3" on a three page document. `[frompage]`, `[sitepage]`,
`[sitepages]`, `[webpage]`, `[title]`, `[doctitle]`, `[date]`, `[isodate]` and `[time]` go
too, along with any placeholder `--replace` defines. Only the two page counts are left to
Chromium, because nothing else knows how many pages a document has until it has been laid
out.

`[section]`, `[subsection]` and `[subsubsection]` name a position in the document outline,
which V1 does not build. They expand to nothing and say so once on stderr, rather than
printing their own name on every page.

`[date]` is **not** byte-identical to wkhtmltopdf's. Qt renders it through the system locale,
so the same binary prints a different string on two machines and there is no format to match;
this writes the local date as `YYYY-MM-DD`, and `[isodate]` adds the time and the UTC offset.

**Headers and footers are drawn.** `--header-left/center/right`, the same three for the
footer, the font name and size, the rules and `--default-header` all render through
Chromium's print templates. The text is used as written; `[page]` and the other placeholders
are not substituted yet and the option says so.

Three things about where a band lands, measured rather than assumed:

- **A band never moves the content.** It is drawn inside the margin, so adding a header to a
  migrated command line cannot silently repaginate it.
- **The margin has to accommodate the band.** A 12pt band is about 28.5pt tall and a 10mm
  margin is 28.3pt, so the default only just fits; a larger font needs a larger margin.
- **`--header-spacing` moves the content, not the band.** A band is anchored to the paper
  edge, so the print margin is the only thing that can open a gap below it.

### Changed

**The README says what this is.** A weekend project, not a maintained product: no release,
no support, and no undertaking that any of it keeps working. `SECURITY.md` says the same,
so its threat model is not read as a promise that the code meets it.

### Added

**`--encoding`, `--minimum-font-size`, `--viewport-size` and `--no-images` are honoured.**

`--encoding` says how to read a document that declares no charset. There is no protocol
command for that and no launch switch — `--default-encoding` was tried and does nothing — so
a local document is served to the browser with a `Content-Type` that says so, at its own URL,
which leaves every relative link resolving where it did. A document fetched over http or
https is read as its server said and there is nowhere to intervene, so the option says it
does not apply rather than being accepted in silence.

**`--user-style-sheet` and `--run-script` are honoured.** A stylesheet named by path is read
by us and inlined, so the policy that stops a *document* reading the disk has nothing to say
about a file the user named on the command line; one named by a URL is left to the browser to
fetch. Either goes in after the document exists and **before the wait for web fonts**,
because a stylesheet that declares a font face makes that wait meaningless if it arrives
afterwards.

`--run-script` runs each script in order once the page has settled, awaiting a promise if one
is returned. A script that throws fails the conversion and names itself, because a command
line usually carries several. `--load-error-handling` will make that configurable (#28); for
now it is always the default.

`--viewport-size` emulates the window, and **the window is now wkhtmltopdf's 1024 by 768 on
every conversion** rather than whatever Chromium's is, because that is what a migrated
document's scripts were written against (D03).

**`--zoom` is measured rather than asserted.** A block of 50mm comes out at 50mm with no
option, 65mm at `--zoom 1.3` and 25mm at `--zoom 0.5`, and the paper does not move. The four
options that used to do this job in wkhtmltopdf — `--dpi`, `--image-dpi` and both smart
shrinking flags — are accepted, warn, point at the migration guide, and change nothing that
is printed. `docs/migration.md` now carries the recalibration procedure and a checklist.

### Security

**A document can no longer read the disk it is rendered on.** `--enable-local-file-access`,
`--disable-local-file-access` and `--allow` are enforced, which is wkhtmltopdf 0.12.6's rule
and D10's: a local document may not read a file beside it unless the flag is given or
`--allow` names the directory, and a document fetched over http or https may never read a
local file at all, whatever the options say. A refused load is named on stderr rather than
dropped, because a document that renders with a stylesheet missing is the hardest kind of
failure to notice.

Chromium's own default is the permissive one and no launch flag changes it, so this is real
per-request interception. `--allow` resolves symlinks and refuses a path that climbs out of
the directory it names.

### Compatibility

The option table has been reconciled against a real wkhtmltopdf 0.12.6.1. Three things
change how a command line is read, all of them towards what the real program does:

- **`--cookie-jar` is a global option**, not a per-object one. It was already global in
  wkhtmltopdf; only this table had it wrong.
- **`--redirect-delay` is gone.** It does not exist in 0.12.6.1, which answers it with
  `Unknown long argument`, and now so do we. It was accepted and ignored before.
- **Options are refused where wkhtmltopdf refuses them.** A global option must come before
  the first input; a table-of-contents option must follow a `toc` object; page options may
  still go anywhere. Command lines that real wkhtmltopdf rejects used to be accepted here,
  and a misplaced `--toc-header-text` silently attached itself to the preceding page. Our
  own options — `--timeout`, `--no-sandbox`, `--chromium-path`, `--chromium-arg`,
  `--dump-parse` — carry no such rule and may be written anywhere.

**The help now says what works.** Thirty-eight options were advertised as implemented while
doing nothing at all: the command line understood each one, filled a field, and no part of a
conversion ever read it. They are now listed as not implemented yet, and writing one prints
the same warning every other unbuilt option already printed. What those options *do* has not
changed — they did nothing before and they do nothing now — but a command line using them is
noisier, and `-q` or `--log-level none` silences it.

**`--viewport-size` does not decide which media queries match.** It moves what a script
reads from the window, and nothing else: a printed page is laid out at the paper's content
width and `@media (min-width: …)` is evaluated against that, whatever the window is set to.
Measured, not deduced, and `docs/migration.md` has been corrected — it claimed otherwise.

**`--default-header` no longer moves the content down the page.** It pushed the top margin
out to 20mm to make room for a band nothing draws, so a document written with it lost a
centimetre off the top and gained no header. It now fills the header fields and leaves the
page alone until the band is drawn.
