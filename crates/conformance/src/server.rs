//! A throwaway web server, for the shape of the brief that is a URL.
//!
//! Blocking, on its own thread, rather than async. The tests around it run the
//! binary as a subprocess and are otherwise synchronous, and making them async
//! to serve two routes would be the tail wagging the dog.
//!
//! A `data:` URL cannot stand in for this. It is not http(s), so it exercises
//! neither the resolution of a URL input nor a page that has a real origin —
//! and, for [`HANG`], nothing about a `data:` URL can be made slow.

use crate::fixture;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::time::Duration;

/// A document with a sentinel in it, served at a real URL.
pub const PAGE: &str = "/page";

/// Renders back the request header named after `?name=`, so a test can see what
/// the browser actually sent.
pub const ECHO_HEADER: &str = "/echo-header";

/// Renders back the `Cookie` header.
pub const ECHO_COOKIE: &str = "/echo-cookie";

/// A page whose stylesheet only does anything when the *stylesheet's own*
/// request carried `X-Tenant`. That is the one way to see the difference
/// `--custom-header-propagation` makes: without it the header is on the document
/// and on nothing else.
pub const PROPAGATION: &str = "/propagation";

/// The stylesheet [`PROPAGATION`] pulls in.
pub const PROPAGATION_STYLE: &str = "/propagation.css";

/// What a proxied request looks like, and the marker the answer carries.
///
/// A browser sending a request **through** a proxy writes the whole URL on the
/// request line — `GET http://host/path HTTP/1.1` — rather than just the path.
/// So this server is a proxy for free: anything arriving in that shape was
/// proxied, and nothing else can be.
///
/// The destination has to be something other than loopback. Chromium bypasses
/// the proxy for localhost whatever `--proxy-server` says, so a test pointed at
/// `127.0.0.1` proves only that the option was accepted.
pub const PROXIED: &str = "CONFORMANCE-PROXIED-3K9";

/// Answers 401 until the request carries the right Basic credentials.
pub const PROTECTED: &str = "/protected";

/// What [`PROTECTED`] wants, and what it prints once it has it.
pub const USER: &str = "conformance";
pub const PASSWORD: &str = "hunter2";

/// The header the propagation fixtures look for.
pub const TENANT: &str = "x-tenant";

/// A stylesheet, for `--user-style-sheet` written as a URL.
pub const STYLESHEET: &str = "/stylesheet.css";

/// What [`STYLESHEET`] serves: taller than A4's content area, so a document that
/// applies it needs a second page and one that does not needs one.
pub const TALL_CSS: &str = "#pad { height: 400mm; }";

/// A document that pulls in whatever URL follows `?href=`, as a stylesheet.
///
/// For the question only a real origin can ask: what a page fetched over http is
/// allowed to reach on the disk of the machine rendering it (D10). The URL is
/// interpolated as written, so keep `?` and `&` out of it — a temporary
/// directory has neither.
pub const REFERENCING: &str = "/referencing";

/// Accepts the request and never answers. The page under it never loads, which
/// is what a deadline is for (D16).
pub const HANG: &str = "/hang";

/// The sentinel [`PAGE`] carries, chosen so nothing else in a PDF could produce
/// it by accident.
pub const SENTINEL: &str = "CONFORMANCE-SENTINEL-7Q4";

pub struct Server {
    base: String,
}

impl Server {
    pub fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("a loopback port should be free");
        let base = format!(
            "http://{}",
            listener
                .local_addr()
                .expect("a bound listener has an address")
        );

        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { return };
                std::thread::spawn(move || {
                    let Some((path, headers)) = read_request(&mut stream) else {
                        return;
                    };

                    if path.starts_with(HANG) {
                        // Held open, unanswered. Bounded only so a test run does
                        // not accumulate parked threads for ever; the deadline
                        // under test is far shorter than this.
                        std::thread::sleep(Duration::from_secs(120));
                        return;
                    }

                    // Absolute-form request line: this arrived through us as a
                    // proxy rather than as an ordinary request.
                    if path.starts_with("http://") || path.starts_with("https://") {
                        let body = fixture::document(&format!("<p>{PROXIED}</p>"));
                        let _ = respond(&mut stream, "200 OK", "text/html", &body);
                        return;
                    }

                    if path.starts_with(STYLESHEET) {
                        let _ = respond(&mut stream, "200 OK", "text/css", TALL_CSS);
                        return;
                    }

                    // Only tall when the stylesheet's own request carried the
                    // header, which is exactly what propagation decides.
                    if path.starts_with(PROPAGATION_STYLE) {
                        let body = match header(&headers, TENANT).is_some() {
                            true => TALL_CSS,
                            false => "/* no header on this request */",
                        };
                        let _ = respond(&mut stream, "200 OK", "text/css", body);
                        return;
                    }

                    if path.starts_with(PROTECTED) {
                        let wanted = format!(
                            "Basic {}",
                            crate::fixture::base64(format!("{USER}:{PASSWORD}").as_bytes())
                        );
                        if header(&headers, "authorization").as_deref() != Some(wanted.as_str()) {
                            let _ = challenge(&mut stream);
                            return;
                        }
                        let body = fixture::document(&format!("<p>{SENTINEL}</p>"));
                        let _ = respond(&mut stream, "200 OK", "text/html", &body);
                        return;
                    }

                    let body = if path.starts_with(ECHO_HEADER) {
                        let name = path
                            .split_once("?name=")
                            .map(|(_, name)| name.to_ascii_lowercase())
                            .unwrap_or_default();
                        let seen = header(&headers, &name).unwrap_or_else(|| "(absent)".into());
                        fixture::document(&format!("<p>{seen}</p>"))
                    } else if path.starts_with(ECHO_COOKIE) {
                        let seen = header(&headers, "cookie").unwrap_or_else(|| "(absent)".into());
                        fixture::document(&format!("<p>{seen}</p>"))
                    } else if path.starts_with(PROPAGATION) {
                        fixture::document(&format!(
                            "<link rel=\"stylesheet\" href=\"{PROPAGATION_STYLE}\">\
                             <div id=\"pad\"></div><p>{SENTINEL}</p>"
                        ))
                    } else if path.starts_with(PAGE) {
                        fixture::document(&format!("<p>{SENTINEL}</p>"))
                    } else if let Some(query) = path.strip_prefix(REFERENCING) {
                        let href = query.strip_prefix("?href=").unwrap_or_default();
                        fixture::document(&format!(
                            "<link rel=\"stylesheet\" href=\"{href}\">\
                             <div id=\"pad\"></div><p>{SENTINEL}</p>"
                        ))
                    } else {
                        let _ = respond(&mut stream, "404 Not Found", "text/plain", "not found");
                        return;
                    };
                    let _ = respond(&mut stream, "200 OK", "text/html", &body);
                });
            }
        });

        Self { base }
    }

    pub fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }
}

/// One header, by its lowercased name.
fn header(headers: &[(String, String)], name: &str) -> Option<String> {
    headers
        .iter()
        .find(|(seen, _)| seen == name)
        .map(|(_, value)| value.clone())
}

/// Ask for Basic credentials. Chromium answers this through `Fetch.authRequired`
/// rather than by failing, which is the event `--username` is wired to.
fn challenge(stream: &mut std::net::TcpStream) -> std::io::Result<()> {
    let body = "unauthorised";
    let response = format!(
        "HTTP/1.1 401 Unauthorized\r\nWWW-Authenticate: Basic realm=\"conformance\"\r\n\
         Content-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(response.as_bytes())?;
    stream.flush()
}

/// Read until the headers end, rather than taking whatever one read returns.
///
/// A request split across segments would otherwise yield a truncated path and a
/// 404, failing a test for a reason that has nothing to do with it.
fn read_request(stream: &mut std::net::TcpStream) -> Option<(String, Vec<(String, String)>)> {
    let mut request = Vec::new();
    let mut chunk = [0u8; 1024];
    loop {
        let read = stream.read(&mut chunk).ok()?;
        if read == 0 {
            break;
        }
        request.extend_from_slice(&chunk[..read]);
        if request.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
        if request.len() > 64 * 1024 {
            break;
        }
    }
    let text = String::from_utf8_lossy(&request);
    let mut lines = text.split("\r\n");
    let path = lines.next()?.split_whitespace().nth(1)?.to_owned();

    // Lowercased, because header names are case-insensitive and Chromium does
    // not promise the casing a test typed.
    let headers = lines
        .take_while(|line| !line.is_empty())
        .filter_map(|line| line.split_once(": "))
        .map(|(name, value)| (name.to_ascii_lowercase(), value.to_owned()))
        .collect();

    Some((path, headers))
}

fn respond(
    stream: &mut std::net::TcpStream,
    status: &str,
    content_type: &str,
    body: &str,
) -> std::io::Result<()> {
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\n\
         Connection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(response.as_bytes())?;
    stream.flush()
}

/// An address nothing is listening on.
///
/// Port 1 on loopback: privileged, so nothing binds it, and refused immediately
/// rather than left to time out, which keeps the negative test fast and makes it
/// a connection failure rather than a deadline expiry.
pub fn unreachable_url() -> String {
    "http://127.0.0.1:1/nothing".to_string()
}
