//! Cookies, custom headers, credentials and the proxy.
//!
//! All four are about what leaves the browser rather than what it draws, so the
//! fixtures are a server that renders back what it received. Two of them use the
//! page count instead, because what is being measured is whether a *subresource*
//! request carried something, and a subresource has no voice in the document.

use rchtmltopdf_conformance::binary::{Outcome, Run};
use rchtmltopdf_conformance::fixture::{self, Scratch};
use rchtmltopdf_conformance::inspect::Pdf;
use rchtmltopdf_conformance::{require_chromium, server};

fn fetch(url: &str, options: &[&str]) -> (Pdf, Outcome) {
    let outcome = Run::new()
        .args(options.iter().copied())
        .arg(url)
        .arg("-")
        .output();
    outcome.succeeded();
    (Pdf::from_bytes(&outcome.stdout), outcome)
}

fn flat(pdf: &Pdf) -> String {
    pdf.text().replace('\n', "")
}

/// The value arrives url encoded, per the option's own help, so it is decoded
/// before the browser sees it. A value with an escape in it is the case that
/// says whether anybody remembered.
#[test]
fn a_cookie_is_sent_and_its_value_is_decoded() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let server = server::Server::start();

    let (pdf, _) = fetch(
        &server.url(server::ECHO_COOKIE),
        &["--cookie", "session", "abc%20def"],
    );
    let text = flat(&pdf);
    assert!(text.contains("session=abc def"), "{text:?}");
}

/// A cookie on a `file://` document has no origin to be scoped to. Warned about
/// rather than failed, because a wrapper emitting one must not break a
/// conversion that would otherwise work.
#[test]
fn a_cookie_for_a_local_document_is_reported_rather_than_failing() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("network-local-cookie");
    let page = fixture::write(scratch.path(), "p.html", "<p>local</p>");

    let outcome = Run::new()
        .arg("--cookie")
        .arg("session")
        .arg("abc")
        .arg(page.display().to_string())
        .arg("-")
        .output();
    outcome.succeeded();
    assert!(
        outcome.stderr.contains("--cookie"),
        "should say the cookie went nowhere:\n{}",
        outcome.stderr
    );
}

#[test]
fn a_custom_header_reaches_the_document() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let server = server::Server::start();

    let (pdf, _) = fetch(
        &server.url(&format!("{}?name={}", server::ECHO_HEADER, server::TENANT)),
        &["--custom-header", "X-Tenant", "acme"],
    );
    assert!(flat(&pdf).contains("acme"), "{:?}", flat(&pdf));
}

/// **The asymmetry that surprises people.** Without propagation the header is on
/// the document's own request and on nothing else; with it, on every request.
///
/// The signal is the page count, because a subresource cannot render anything
/// itself: the stylesheet is only tall when the stylesheet's *own* request
/// carried the header.
#[test]
fn a_custom_header_reaches_subresources_only_with_propagation() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let server = server::Server::start();
    let url = server.url(server::PROPAGATION);

    let (document_only, _) = fetch(&url, &["--custom-header", "X-Tenant", "acme"]);
    assert_eq!(
        document_only.page_count(),
        1,
        "the stylesheet should not have carried the header: {}",
        document_only.describe()
    );

    let (everywhere, _) = fetch(
        &url,
        &[
            "--custom-header",
            "X-Tenant",
            "acme",
            "--custom-header-propagation",
        ],
    );
    assert_eq!(
        everywhere.page_count(),
        2,
        "the stylesheet should have carried the header: {}",
        everywhere.describe()
    );
}

/// Credentials answer a 401 rather than being sent ahead of one, which is what
/// `Fetch.authRequired` is for.
#[test]
fn credentials_answer_a_challenge() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let server = server::Server::start();

    let (pdf, _) = fetch(
        &server.url(server::PROTECTED),
        &["--username", server::USER, "--password", server::PASSWORD],
    );
    assert!(
        flat(&pdf).contains(server::SENTINEL),
        "the protected page should have loaded: {:?}",
        flat(&pdf)
    );
}

/// A wrong password is retried by the browser for ever unless somebody stops.
/// Offering once and cancelling the second ask stops that — and cancelling makes
/// the server's own 401 body the response, which renders perfectly well as a
/// page, so the rejection is reported and the conversion fails on it. The pair
/// KnpSnappy reads: a non-zero exit and something on stderr (D14).
#[test]
fn a_wrong_password_fails_rather_than_printing_the_401() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let server = server::Server::start();

    let outcome = Run::new()
        .arg("--username")
        .arg(server::USER)
        .arg("--password")
        .arg("not the password")
        .arg(server.url(server::PROTECTED))
        .arg("-")
        .output();
    outcome.failed();
    assert!(
        outcome.stderr.contains("--username"),
        "should name the options that were refused:\n{}",
        outcome.stderr
    );
    assert!(
        outcome.stdout.is_empty(),
        "a failure must leave no document behind"
    );
}

/// `--proxy` is a launch flag, and the proof that it took is that the request
/// arrived *through* the proxy: a browser going through one writes the whole URL
/// on the request line, and the test server answers only that shape with a
/// marker of its own.
///
/// **The destination must not be loopback.** Chromium bypasses the proxy for
/// localhost whatever `--proxy-server` says, so the first version of this test
/// passed while proving nothing — the request went straight out and succeeded.
#[test]
fn a_request_goes_through_the_proxy() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let server = server::Server::start();

    let (pdf, _) = fetch(
        "http://conformance.invalid/anything",
        &["--proxy", &server.url("")],
    );
    assert!(
        flat(&pdf).contains(server::PROXIED),
        "the request should have gone through the proxy: {:?}",
        flat(&pdf)
    );
}

/// And without the proxy the same address goes nowhere, so the test above is
/// measuring the option rather than a server that would have answered anyway.
#[test]
fn the_same_address_fails_without_the_proxy() {
    let Some(_browser) = require_chromium() else {
        return;
    };

    Run::new()
        .arg("http://conformance.invalid/anything")
        .arg("-")
        .output()
        .failed();
}

/// **An image is asked for the way wkhtmltopdf asked for it** (D63).
///
/// Measured against 0.12.6.1: it sends `Accept: */*` for an image, where
/// Chromium offers `image/avif,image/webp,image/apng,image/svg+xml,image/*`.
/// A server that negotiates on `Accept` — serving WebP from a `.jpeg` URL is
/// the ordinary way to do it — therefore answered wkhtmltopdf with a JPEG,
/// which it copied straight into the file, and answered us with a WebP, which
/// no PDF can carry: the picture was decoded and re-embedded losslessly. On a
/// measured document from a reference project (#32) that was 3 MB against
/// 450 kB.
///
/// The server records what the image request carried and a second conversion
/// reads it back, because a subresource has no voice in the document it is
/// part of.
#[test]
fn an_image_is_asked_for_the_way_wkhtmltopdf_asks() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let server = server::Server::start();
    let scratch = Scratch::new("network-image-accept");
    let page = fixture::write(
        scratch.path(),
        "page.html",
        &format!("<img src=\"{}\">", server.url(server::PROBE_IMAGE)),
    );

    let asked = Run::new().arg(page.display().to_string()).arg("-").output();
    asked.succeeded();

    let (report, _) = fetch(&server.url(server::SEEN_ACCEPT), &[]);
    let seen = flat(&report);
    assert!(
        seen.contains("*/*") && !seen.contains("image/webp"),
        "the image request carried {seen:?}"
    );
}
