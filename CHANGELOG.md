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
option surface beyond what V0 needed: headers and footers, cookies and custom headers,
credentials and proxies, and PDF metadata are all understood on the command line and not
yet acted on.

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
