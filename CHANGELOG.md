# Changelog

All notable changes to this project are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this
project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

A **Compatibility** section carries anything that changes how a wkhtmltopdf command line
behaves. If you are migrating, that is the section to read.

## [Unreleased]

Nothing released yet.

The binary converts one document: a URL, a local file or standard input, to a file or
standard output, with paper size, margins, orientation, backgrounds, stylesheets, headers
and footers. Options with no Chromium equivalent are accepted and warned about rather than
breaking a command line that uses them.

Several documents, covers and a table of contents are not built. Neither is most of the
option surface beyond what V0 needed.
