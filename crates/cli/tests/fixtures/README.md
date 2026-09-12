# The reference help

`wkhtmltopdf-0.12.6.1-extended-help.txt` is the verbatim standard output of

```
wkhtmltopdf --extended-help
```

from **wkhtmltopdf 0.12.6.1 (with patched qt)**, the last release of the 0.12.6 line, installed
from the official `wkhtmltox_0.12.6.1-3.bookworm_arm64.deb` in `debian:bookworm-slim`.

Not edited, not reflowed, not re-ordered. `reference_help.rs` parses it and holds
`crates/cli/src/table.rs` to it, so the table is checked against the program rather than
against someone's memory of the documentation.

## Why it is a file and not a docker run

The table has to be checkable on a laptop, in CI, and offline, in under a millisecond. A test
that shelled out to a container would be none of those, and would make every unrelated pull
request depend on a third-party release still being downloadable.

The cost is that the fixture can go stale. It cannot go stale *silently*: it is the whole
reference, so any option added to or removed from the table without updating it fails the
test.

## Reproducing it

```bash
docker build -t wkhtmltopdf-ref:0.12.6.1 - <<'DOCKERFILE'
FROM debian:bookworm-slim
ARG DEB=https://github.com/wkhtmltopdf/packaging/releases/download/0.12.6.1-3/wkhtmltox_0.12.6.1-3.bookworm_arm64.deb
RUN apt-get update -qq >/dev/null \
 && apt-get install -y -qq curl ca-certificates >/dev/null \
 && curl -sSL -o /tmp/w.deb "$DEB" \
 && apt-get install -y -qq /tmp/w.deb >/dev/null
DOCKERFILE

docker run --rm wkhtmltopdf-ref:0.12.6.1 wkhtmltopdf --extended-help \
  > crates/cli/tests/fixtures/wkhtmltopdf-0.12.6.1-extended-help.txt
```

Swap `arm64` for `amd64` in the URL on an Intel machine. The help text is identical.
