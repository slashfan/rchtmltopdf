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
                    let Some(path) = read_request_path(&mut stream) else {
                        return;
                    };

                    if path.starts_with(HANG) {
                        // Held open, unanswered. Bounded only so a test run does
                        // not accumulate parked threads for ever; the deadline
                        // under test is far shorter than this.
                        std::thread::sleep(Duration::from_secs(120));
                        return;
                    }

                    let body = if path.starts_with(PAGE) {
                        fixture::document(&format!("<p>{SENTINEL}</p>"))
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

/// Read until the headers end, rather than taking whatever one read returns.
///
/// A request split across segments would otherwise yield a truncated path and a
/// 404, failing a test for a reason that has nothing to do with it.
fn read_request_path(stream: &mut std::net::TcpStream) -> Option<String> {
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
    text.split_whitespace().nth(1).map(str::to_owned)
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
