# Changelog

All notable changes to this project are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this
project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

A **Compatibility** section carries anything that changes how a wkhtmltopdf command line
behaves. If you are migrating, that is the section to read.

## [Unreleased]

### Compatibility

**`--dump-outline` fills `link` and `backLink`** (D56). Both attributes were written empty,
and wkhtmltopdf's own default table-of-contents stylesheet tests for them, so a consumer
feeding the dump to it got a table without links. They now carry wkhtmltopdf's names:
`__WKANCHOR_` and a base-36 counter, two per item in reading order, the documents first and
the tables of contents after them, whose items carry the same name in both attributes, as
wkhtmltopdf writes them.

## [0.2.0] - 2026-09-16

Six compatibility changes, each measured against wkhtmltopdf 0.12.6.1 and recorded as a
decision (D51 to D55), and nothing else: the caveats at the top of the README still apply.

### Compatibility

**A document missing from the disk is judged by `--load-error-handling`** (D55). wkhtmltopdf
fetched a local file through the same stack as a URL, so a path that is not there was a
failed load: `skip` dropped it and wrote the other documents, `ignore` left a blank page in
its place, and both exited 1 with the file written. We refused the path before starting
anything, whatever the handler said, and wrote nothing. `abort`, the default, is unchanged.

**`--dump-outline` writes one `item` per object around that object's headings** (#127,
D52). wkhtmltopdf wraps each document's headings in an item of its own, titled with the
document's `<title>` — a table of contents with its caption — and numbered with the pages
before it, so the first object is `page="0"`. We wrote the headings flat at the top level,
which gave a consumer walking `outline/item` a different tree on every conversion, a single
document included. A cover, or a document `--exclude-from-outline`, keeps its item with
`title=""` and nothing under it, as wkhtmltopdf writes it. The bookmarks in the file are not
affected: they are the flat run of headings in both programs.

**`--outline-depth` bounds the bookmarks and nothing else** (#115, D53). It used to cut
the `--dump-outline` XML, the headings a table of contents lists and the headings
`[subsection]` and `[subsubsection]` name to the same depth. wkhtmltopdf checks the depth
only where it writes the bookmarks: with `--outline-depth 1`, its dump, its table of
contents and its bands still carry every level, and so do ours now.

**A document with no `<title>` element has no title** (D52). Chromium writes the URL into the
Info dictionary of such a document — the file name, or the host and path — and that is what
`[title]` printed, what `[doctitle]` fell back to and what the file carried. wkhtmltopdf
prints nothing for either and leaves the file's title empty, and so does this now: the title
is read from the page itself once it has loaded, and the file's title is written whether or
not `--title` was given.

**`--page-offset` is one number for the whole output** (#44, D51). It used to shift the
document it was written on, so `wkhtmltopdf a.html b.html --page-offset 100` numbered the
first document `1 2 3` and the second `104 105`. wkhtmltopdf keeps the offset in its global
settings, though its help lists it among the page options: written anywhere it shifts every
page, and the last one written wins. The same command line now numbers all five pages
`101` to `105`, and the `--dump-outline` attribute follows, which is what wkhtmltopdf has
always done with it.

**`[frompage]` is the first page of the output**, not of the document the page came from
(#44, D51). Across two documents every page now prints `[frompage]` = 1, where the pages of
the second document used to print the page it began on. Under an offset it is that offset
plus one. `[sitepage]` and `[sitepages]` are unchanged: they are the frame that counts
within a document, and a cover and a table of contents each have one of their own.

## [0.1.0] - 2026-09-15

The first published version. Static Linux binaries for x86_64 and aarch64, macOS binaries
for Apple silicon and Intel, and a Docker image with a pinned browser, all published from a
merge that raises the version (D50). A first version rather than a finished one: the warning
at the top of the README still applies in full.

The binary converts one document or several: each a URL, a local file or standard input,
to one file or standard output, with paper size, margins, orientation, zoom, backgrounds,
screen or print stylesheets, whether scripts run, and how long to wait before printing.
Options it does not act on are accepted and warned about rather than breaking a command
line that uses them.

A table of contents is built, from the same headings as the outline. Most of the option
surface beyond that is understood on the command line and not yet acted on.

### Compatibility

**Three stderr lines wkhtmltopdf writes and this program did not** (#114, D48). A document
that fails to load is now named on its own line — `Failed to load <url>, with network status
code 3 and http status code 0 - HostNotFoundError` — under `abort`, `skip` and `ignore`
alike, which is the line that carries the address a script is watching for. A subresource
lost under `--load-media-error-handling ignore`, the default, is named too instead of
passing in silence: a PDF that rendered without its stylesheet now says why. And
`Exit with code 1 due to network error: <Name>` ends every run that exits 1 because
something would not load, including under `abort` and including a document that was missing
from the disk rather than from the network. Found by running a Symfony application through
knp-snappy against both binaries (#32).

**`[section]`, `[subsection]` and `[subsubsection]` name the first heading on the page**
(#109, D47). Where two headings of one level fall on the same page, the band now names the
first, as wkhtmltopdf does; it named the last. A page that begins no heading of a level
keeps what the page before it named, which is that same first heading — and keeps it across
a document boundary, so a second document without an `h2` of its own still carries the
previous one's. Found by running a Symfony application through knp-snappy against both
binaries (#32).

**`[title]` is the document's own `<title>` again** (#110, D46). It printed `--title`, which
is `[doctitle]`'s answer, so the two were the same string on every page. Each document's
band now names that document — with several documents, each page says its own — and
`[doctitle]` names the file: `--title`, or the first document's title when it was not
given, which is the title the file itself carries (#107). Found by running a Symfony
application through knp-snappy against both binaries (#32).

**A cover counts in `[page]` and `[topage]`** (#108, D45). Behind a one-page cover the
first page of the document now prints 2, not 1, and the total counts the cover — which is
what wkhtmltopdf prints, and what `--dump-outline` and the table of contents already said.
A cover still gets no header or footer of its own, so the number it carries is printed
nowhere. Found by running a Symfony application through knp-snappy against both binaries
(#32).

**`--load-error-handling skip` and `ignore` survive a document that never loaded**
(#113, D44). A host that does not resolve ended the conversion whatever the option said —
exit 1 and no file — because the failure was raised before the handler saw it. Now the
handler decides, as wkhtmltopdf's does: `skip` writes the file from the documents that did
load, `ignore` leaves a blank page where the missing one would have been so the page behind
it keeps its number, and `abort`, the default, still writes nothing. All three exit 1, with
`Exit with code 1 due to network error: HostNotFoundError` — D14 said `skip` and `ignore`
exit 0, which holds for a failing subresource and not for a document. Found by running a
Symfony application through knp-snappy against both binaries (#32).

**`--replace` no longer shadows a built-in placeholder** (#111). A pair named after one
of wkhtmltopdf's own placeholders defined nothing in a text band and hid the real answer:
`--footer-center 'Page [page]' --replace page draft` printed "Page draft" on every page.
The built-in wins now, which is the order wkhtmltopdf fills its hash in and what
`--header-html`'s query string already did. A `--replace` pair still defines a placeholder
of your own. Found by running a Symfony application through knp-snappy against both
binaries (#32).

**A `toc` no longer names the file after itself** (#107). The merged file is named after
the first document, as wkhtmltopdf named it, and the contents page this program writes is
passed over the way wkhtmltopdf passed over its own: `toc doc.html` carries `doc.html`'s
`<title>`, not "Table of Contents". Found by running a Symfony application through
knp-snappy against both binaries (#32).

### Added

**The table of contents** (#42, D41). `toc` on the command line generates one from the
headings of the documents around it, listing each with the page of the finished file it
landed on — the page `--dump-outline` gives it, so a cover counts and `--page-offset` moves
it. It goes where it was written, and it lists itself, which is the line wkhtmltopdf prints
too.

**Its length settles rather than being guessed.** A table lists the pages behind it and its
own pages push them down, so it is built, measured and built again until the two agree. A
table of contents running to three pages numbers the first heading behind it 4, as
wkhtmltopdf does; a single pass would have said 2.

**Every entry links to the heading it names** (#43, D42), landing on the heading itself
rather than on the top of its page, so two headings sharing a page are two different links.
Nothing is planted in the document to make that work: the link is the destination Chromium
wrote for its own outline. `--disable-toc-links` writes the entries without links at all.
A table's links are its own — `--disable-internal-links` and `--disable-external-links` are
about the documents being converted and leave them alone, as they do in wkhtmltopdf.

**`--enable-toc-back-links` is not built yet** (D42). A back link is an annotation over the
heading, so it needs the heading's box, and Chromium gives only its position. wkhtmltopdf
got the box by wrapping each heading in an `<a>` before printing, which lets the page's own
`a { }` rules repaint its headings; that is not done here. The option stays accepted and
warned about, and it is off by default in wkhtmltopdf too.

`--toc-header-text`, `--toc-level-indentation`, `--toc-text-size-shrink` and
`--disable-dotted-lines` land where wkhtmltopdf's default stylesheet carried them: the
heading above the list, a `padding-left` per level, a `font-size` per level, and the rule
under each entry.

**`--bypass-proxy-for` and `--cache-dir`** (#46) reach the browser: hosts written on the
first go direct instead of through `--proxy`, and the second names where the web cache
lives instead of letting it go with the throwaway profile. Both were measured before being
claimed — Chromium ignores a flag it does not know, so a flag that is merely accepted
proves nothing.

**`--enable-toc-back-links` and `--disable-toc-back-links` have no equivalent** (D42). A
back link is an annotation over the heading, so it needs the heading's box, and Chromium
reports only its position. The ways to get the box either guess it or wrap the heading in
markup of ours, which would let the page's own `a { }` rules repaint its headings.

**Seventeen options stopped claiming a milestone that had shipped** (#46). They said
"planned for V2" on the day V2 opened and went on saying it after V2 closed, which is the
same kind of claim the audit found in `Implemented` (#19): one that names a date rather
than a fact and goes stale in silence. They now say what is so — not built, and not
scheduled — and a test fails if any option is ever left promising a milestone that has
already shipped.

**`--xsl-style-sheet` and `--dump-default-toc-xsl` have no equivalent** (D41). The table is
generated rather than transformed, so there is no stylesheet to supply or to dump. Chromium
does still perform the transform — the prototype worked on the pinned 153 — but Chrome
removes XSLT on stable in 158, on 17 November 2026, and D09 prefers whatever browser the
machine already has. Both options are accepted with the usual warning.

### Changed

**Headers and footers are drawn after printing, as an overlay** (D38, #38). Until now they
were Chromium's print templates, which cannot count past the document being printed: `[page]`
restarted at one for every document of a conversion, `[topage]` was each document's own
total, and `--page-offset` could not be honoured. Now the documents are printed without
bands and merged, the page counts are read off the result, one sheet per page is written
with every placeholder expanded, the same browser prints that document of sheets, and each
sheet is drawn onto its page. Typography stays Chromium's, and the band's text is
extractable like any other.

Where a band lands has not changed in the ways that matter and has changed in one that
does. A band still never moves the content, `--header-spacing` still opens a gap by moving
the content and not the band, and a band taller than the margin still runs into the content.
What changed: the rule of a `--header-line` now sits exactly on the margin line, the top
margin down from the paper edge, where Chromium's template put it about a point lower.

### Added

**`--header-html` and `--footer-html`** (#38, D39). The document is framed on every page
with the placeholders passed as a query string, the way wkhtmltopdf passed them, so the
`subst()` script from its manual works unchanged, and `[page]` counts across documents.
The geometry follows wkhtmltopdf's rule for these bands: with no `--margin-top` written, the
document's height becomes the margin, plus `--header-spacing`; with one written, the document
is fitted into it. The document reads the disk under the same rule as the input.
`--run-script`, `--user-style-sheet`, custom headers and credentials do not reach it.

**Page numbers across documents** (#39). `[page]` and `[topage]` count across the whole
output, `[sitepage]` and `[sitepages]` within the document the page came from, and
`[frompage]` is where that document began. A cover's pages count like any other (D45); what
a cover has not got is a band to print them in.

**`--page-offset`** (#37) shifts `[page]`, `[topage]` and `[frompage]` on the document it
was written on, and on every document when written before the first input. It shifts the
`page` attribute of `--dump-outline` too, by the offset of the document the entry came from
— where wkhtmltopdf applies the last offset written on the command line to every entry
(D40).

**`[section]`, `[subsection]` and `[subsubsection]`** name the heading in force on the page:
the last `h1`, `h2` or `h3` at or before it, within the same document, read from the
outline (D36). A band that uses them has the outline generated for its sake even under
`--no-outline`. They used to expand to nothing with a warning.

**Links** (#41). Chromium writes a link annotation for every `<a href>` it prints, and the
conversion now finishes the job wkhtmltopdf did around them:

- **An anchor survives a merge.** `<a href="#x">` is a named destination resolved through a
  table in the document's catalog, and the merge drops that catalog. Every name is resolved
  to the page it meant before the table is left behind, so the link still lands after the
  pages have moved.
- **A link to another document of the same conversion is a link into the file**, to the
  anchor it names or to that document's first page — what wkhtmltopdf called a local link.
- `--disable-internal-links` drops the links to anchors, in this document or another of the
  conversion; `--disable-external-links` drops the rest. Both per document.
- `--keep-relative-links` writes a link that sits below the document's own directory back
  relative to it, undoing the browser's resolution; the default, `--resolve-relative-links`,
  leaves it absolute. A link above the directory stays absolute either way.

**`--enable-forms` and `--disable-forms` have no equivalent** (D37). Chromium's print path
draws a form control as it looks and writes no field behind it — no AcroForm, no widget — so
there is nothing to turn on or off. Both are accepted with the usual warning.

**The outline** (#40): the bookmarks a reader lists in its sidebar, derived from the
headings, nested by level, one destination each. On by default as in wkhtmltopdf;
`--no-outline` leaves it out; `--outline-depth` cuts it, four levels deep by default;
`--exclude-from-outline` keeps one document's headings out of it and a cover is out of it
by construction. Across several documents the outlines are joined end to end, in order,
each entry pointing at the page it landed on after the merge.

Chromium builds the outline during the print call, all or nothing per document (D36). The
depth is cut afterwards in the `pdf` crate, which unhooks the entries below the boundary
and prunes them from the file rather than leaving titles in the bytes that no reader shows.

`--dump-outline <file>` writes it in the XML wkhtmltopdf's scripts read: an `outline` root in
the `http://wkhtmltopdf.org/outline` namespace, `item` elements nested as the headings were,
each with `title`, `page`, `link` and `backLink`. `page` is the 1-based page in the file
shifted by `--page-offset` (#37, D40), and a cover is one of those pages — as it is in
`[page]` since D45.
`link` and `backLink` are written empty: they named anchors wkhtmltopdf planted so a table of
contents could point at a section and back, and nothing plants those yet (#43). The dump is
written even with `--no-outline`, and describes the outline after `--outline-depth`.

**The cover object** (#37). `cover <input>` is printed like a page, with two differences
wkhtmltopdf documents: it has no headers and footers — the bands given as defaults reach
every page and not the cover, while a band written after `cover` is the cover's own — and
it does not count, so the first page after it is page one and `[topage]` leaves it out.
It is left out of the outline (#40), and it is a page of the file like any other to
`--dump-outline`, which counts it where `[page]` does not (D40).

### Added

**Several documents on one command line**, combined into one PDF in the order given (#36).
Each is loaded and printed on its own page of one browser, with its own options — a footer
written after the second input belongs to the second document alone — and the printed
documents are merged afterwards: every object renumbered, one page tree, whatever a page
inherited from its old tree copied onto it, and the first document's title kept, which is
the title wkhtmltopdf's output carried.

`--load-error-handling skip` now means what it says. The document that failed is dropped
with `failed loading page <url> (skipped)` on stderr, the others are converted, and the exit
code is 0 (D14). When every document was skipped the conversion fails and says so, rather
than writing an empty file. `abort`, the default, still means exit 1 and no PDF.

One browser serves the whole conversion (D35). It is restarted only for a document whose
options are decided on the browser's own command line rather than over the protocol —
`--proxy`, `--minimum-font-size`, `--no-images` — and the result is the same either way.

**Fonts are not deduplicated across documents (D34).** Chromium subsets a font per
document, so ten documents in one face embed ten subsets of it. Deliberate, and measured in
the decision.

Standard input can be given as one document only. `- -` is refused by name rather than
read as one document and one blank page, and every input is checked before a browser starts,
so a missing file at the end of a long command line fails as fast as one at the front.

### Added

**A Docker image**, multi-architecture, carrying a pinned `chrome-headless-shell`, fonts and
the `wkhtmltopdf` symlink. Both architectures use the same pinned browser — Chrome for Testing
publishes `linux-arm64` for it — so a page count does not depend on which one you are on.

**It does not disable Chromium's sandbox for you** (D33). A plain `docker run` fails, saying
what happened and naming every way to fix it; the README recommends `--cap-add=SYS_ADMIN`,
which keeps the sandbox, and documents `--no-sandbox` as the explicit opt-out for HTML you
generated yourself. CI checks both halves — that the recommended command converts, **and that
the plain one refuses** rather than quietly rendering untrusted HTML unsandboxed.

`fonts-liberation` is in the image because it is load-bearing rather than decoration: header
and footer bands default to Arial, and Liberation Sans is what is metric-compatible with it.

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
