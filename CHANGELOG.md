# Changelog

All notable changes to this project are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this
project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

A **Compatibility** section carries anything that changes how a wkhtmltopdf command line
behaves. If you are migrating, that is the section to read.

## [Unreleased]

Nothing released yet.

The command line grammar and the full wkhtmltopdf option table parse. The browser layer
finds and launches Chromium, waits for a page to settle, and prints it to PDF. The
translation from a command line into settings is not written, so the binary does not convert
yet.
