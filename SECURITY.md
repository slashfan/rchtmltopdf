# Security policy

**This is a weekend project, not a maintained product** — see the warning at the top of the
README. What follows says what the code tries to do and what counts as a bug in it. It is
not an undertaking to answer within any particular time, and it is not evidence that the
code succeeds at what it tries.

Reports are still welcome, and the threat model below is the part worth reading before
rendering HTML you did not write.

## Supported versions

The latest release, and `main`. A fix lands on `main` and becomes a release when the version
is raised; nothing is backported to an earlier version.

## Reporting a vulnerability

Use GitHub's private vulnerability reporting on this repository: **Security → Report a
vulnerability**. Please do not open a public issue for a suspected vulnerability.

## Threat model

This matters more than usual here, because the tool renders **untrusted HTML** and has
options that deliberately grant it access to the local filesystem.

**In scope. These are vulnerabilities, report them.**

- Reading a local file that the options did not permit. Local file access is off by
  default, and `--enable-local-file-access` and `--allow` are the only ways to grant it. A
  document that escapes those rules is a bug, and it is the exact class of issue that made
  upstream wkhtmltopdf change its own default in 0.12.6.
- Escaping the `--allow` directory whitelist, including through symbolic links or `..`.
- A local `file://` subresource being fetched from a document loaded over HTTP or HTTPS.
  That is blocked unconditionally and no option re-enables it.
- Command injection through option values, or anything that executes code outside the
  browser sandbox.
- A crash reachable from document content that is exploitable rather than merely a crash.

**Out of scope.**

- Rendering differences from wkhtmltopdf. Page size, font metrics, line wrapping and
  pagination are expected to differ. See `docs/migration.md`.
- Running with `--no-sandbox`, or with `--enable-local-file-access` over input you do not
  control. Both are the operator's decision and both are documented as risks.
- Vulnerabilities in Chromium itself. Report those to the Chromium project. Do tell us if
  we are pinning a build with a known issue.

## Operating safely

If the HTML you render comes from your users, leave local file access disabled, keep the
browser sandbox on, and prefer passing content through stdin over granting filesystem
reach.
