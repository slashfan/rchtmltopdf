# The vendored font

`noto-sans-latin-400-normal.woff2` is the Latin subset of Noto Sans Regular, taken from the
[`@fontsource/noto-sans`](https://www.npmjs.com/package/@fontsource/noto-sans) package.

| | |
| --- | --- |
| Source | `https://cdn.jsdelivr.net/npm/@fontsource/noto-sans/files/noto-sans-latin-400-normal.woff2` |
| Package version | 5.3.0 |
| Size | 13120 bytes |
| SHA-256 | `09aee8065d25508f23a4c3d92cd777ac869c52d93fd868a88f025d888a7937d6` |
| Licence | SIL Open Font License 1.1, in `LICENSE-OFL.txt` |

## Why a font is in the repository at all

**Page count is not font independent.** Line wrapping follows font metrics, and the fonts
installed differ between a GitHub runner image, a developer's Mac and the Docker image. A
fixture that asks for `sans-serif` is measured against a different typeface in each of the
three, so a page count assertion that passes locally can fail in CI for a reason that has
nothing to do with the change under test.

Every fixture therefore declares this one file, by a family name no system font answers to,
with **no fallback**. Page count then depends on the pinned Chromium alone.

The font travels inside the document as a `data:` URI rather than as a relative URL, because
a fixture also has to work when it is read from standard input, where there is no base URL
to resolve against, and from `file://`, where sub-resource access is off by default (D10).

`fixture::tests::font_is_the_vendored_one` guards the file: it asserts the size above and an
FNV-1a digest of the bytes, so replacing the font without reading this page fails a test
rather than quietly moving every page count in the suite. It is not the SHA-256 — that is
here for you to check by hand, and hashing it in a test would mean a digest dependency for
one assertion.

```bash
shasum -a 256 crates/conformance/fixtures/fonts/noto-sans-latin-400-normal.woff2
```

## Regular only, so no headings and no bold in a fixture

The file carries weight 400 and nothing else. Ask for bold — an `<h1>`, a `<b>`, a
`font-weight` — and Chromium synthesises it, and what it then writes into the PDF is a
Type 3 font of glyph outlines rather than the embedded face: the text is drawn, and it is
**not extractable**. `page_text` on such a page gives glyph indices shifted into ASCII
(`COVERTEXT` came back as `&29(57(;7`), so an assertion on a sentinel fails for a reason
that has nothing to do with the option under test. Write sentinels in a `div`.

