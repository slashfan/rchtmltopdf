# syntax=docker/dockerfile:1

# The artefact most deployments actually want (D19): the program, a browser it
# can find, and fonts, in an image that converts a document with no arguments
# beyond the ones wkhtmltopdf took.

ARG RUST_VERSION=1.88
ARG DEBIAN=bookworm

FROM rust:${RUST_VERSION}-${DEBIAN} AS build
WORKDIR /src
COPY . .
# Only the binary. The conformance harness is not shipped and wants a browser
# nobody is going to give it here.
RUN cargo build --release --locked --package rchtmltopdf

FROM debian:${DEBIAN}-slim

# Read from `.chromium-version` by whatever invokes the build, so the image, CI
# and the conformance suite cannot drift apart. There is deliberately no default:
# an image built against whatever Chromium happened to be current is not the
# image the conformance suite measured.
ARG CHROMIUM_VERSION
# linux64 or linux-arm64, as Chrome for Testing names them. Both exist for this
# pin, which is why the image can be multi-arch on one browser rather than a
# different one per architecture.
ARG CHROMIUM_PLATFORM=linux64

# Fonts are not decoration. Chromium with none renders boxes, and
# **`fonts-liberation` is load-bearing**: header and footer bands default to
# Arial, which Liberation Sans is metric-compatible with. Without it every
# migrated band changes width.
#
# curl and unzip are here to fetch the browser and gone again in the same layer,
# so the published image cannot download anything (D31 is about the binary; this
# keeps the container honest too).
RUN set -eux; \
    test -n "${CHROMIUM_VERSION}" || { echo "build with --build-arg CHROMIUM_VERSION=\$(cat .chromium-version)" >&2; exit 1; }; \
    apt-get update; \
    apt-get install --yes --no-install-recommends \
        ca-certificates \
        curl \
        unzip \
        fonts-liberation \
        fonts-dejavu-core \
        fonts-noto-core \
        fonts-noto-color-emoji \
        libasound2 \
        libatk-bridge2.0-0 \
        libatk1.0-0 \
        libcairo2 \
        libcups2 \
        libdbus-1-3 \
        libdrm2 \
        libgbm1 \
        libglib2.0-0 \
        libnspr4 \
        libnss3 \
        libpango-1.0-0 \
        libx11-6 \
        libxcb1 \
        libxcomposite1 \
        libxdamage1 \
        libxext6 \
        libxfixes3 \
        libxkbcommon0 \
        libxrandr2 \
    ; \
    curl --fail --silent --show-error --location \
        --output /tmp/shell.zip \
        "https://storage.googleapis.com/chrome-for-testing-public/${CHROMIUM_VERSION}/${CHROMIUM_PLATFORM}/chrome-headless-shell-${CHROMIUM_PLATFORM}.zip"; \
    unzip -q /tmp/shell.zip -d /opt; \
    mv "/opt/chrome-headless-shell-${CHROMIUM_PLATFORM}" /opt/chrome-headless-shell; \
    rm /tmp/shell.zip; \
    apt-get purge --yes curl unzip; \
    apt-get autoremove --yes; \
    rm -rf /var/lib/apt/lists/*; \
    /opt/chrome-headless-shell/chrome-headless-shell --version

COPY --from=build /src/target/release/rchtmltopdf /usr/local/bin/rchtmltopdf
# D13: the point of a drop-in replacement is that it drops in.
RUN ln --symbolic rchtmltopdf /usr/local/bin/wkhtmltopdf

# The second rung of D09's ladder, so the browser resolves with no flags and
# without depending on where a package manager would have put one.
ENV RCHTMLTOPDF_CHROMIUM=/opt/chrome-headless-shell/chrome-headless-shell

# Not root. A program whose whole job is rendering untrusted HTML should not be
# the one thing in the container that can write to it.
RUN useradd --create-home --uid 1000 rchtmltopdf
USER rchtmltopdf
WORKDIR /work

# **`--no-sandbox` is deliberately not baked in** (D33). The threat model is
# untrusted HTML (D10), this is the artefact most deployments use, and an image
# that silently drops the sandbox would be the posture this project exists to
# improve on. A plain `docker run` therefore fails — with a message naming the
# cause and every way to fix it — rather than converting unsafely.
#
#   docker run --rm -i --cap-add=SYS_ADMIN IMAGE - - < page.html > out.pdf
#
# For HTML you generated yourself, `--no-sandbox` as an argument is the explicit
# opt-out, and being explicit is the whole point.
ENTRYPOINT ["rchtmltopdf"]
