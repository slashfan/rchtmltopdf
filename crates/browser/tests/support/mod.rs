//! Shared scaffolding for browser-backed tests.

#![allow(dead_code)]

use rchtmltopdf_browser::locate::{Executable, SystemEnvironment, locate};
use rchtmltopdf_browser::{Browser, LaunchOptions};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// Find a browser, or say why the test is being skipped.
///
/// Skipping keeps `cargo test` working on a machine with no browser. Setting
/// `RCHTMLTOPDF_REQUIRE_CHROMIUM` turns the skip into a failure, which is what
/// CI does, so a pinned browser quietly vanishing is caught rather than ignored
/// for ever.
pub fn browser_or_skip() -> Option<Executable> {
    match locate(None, &SystemEnvironment) {
        Ok(executable) => Some(executable),
        Err(error) => {
            if std::env::var_os("RCHTMLTOPDF_REQUIRE_CHROMIUM").is_some() {
                panic!("RCHTMLTOPDF_REQUIRE_CHROMIUM is set but no browser was found:\n{error}");
            }
            eprintln!("skipping: no browser on this machine");
            None
        }
    }
}

/// Launch options suited to wherever the tests are running.
pub fn options() -> LaunchOptions {
    LaunchOptions {
        // A GitHub runner and a default container both refuse the sandbox. The
        // product never gives it up on its own (D10); the environment says when
        // it is unavailable.
        no_sandbox: std::env::var_os("RCHTMLTOPDF_TEST_NO_SANDBOX").is_some(),
        handshake_timeout: Some(Duration::from_secs(60)),
        ..LaunchOptions::default()
    }
}

/// Starting a browser is expensive; these take turns so a small runner is not
/// asked to cold-start several at once.
pub async fn one_at_a_time() -> tokio::sync::SemaphorePermit<'static> {
    static TURN: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(1);
    TURN.acquire().await.expect("the semaphore is never closed")
}

pub async fn launch() -> Option<Browser> {
    let executable = browser_or_skip()?;
    Some(Browser::launch(&executable, &options()).await.unwrap())
}

/// A throwaway web server.
///
/// The interesting cases for the wait ladder are all about timing: a resource
/// that is slow, one that is missing, a status that arrives late. A `data:` URL
/// cannot express any of them.
pub struct TestServer {
    pub base: String,
}

impl TestServer {
    pub async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());

        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    return;
                };
                tokio::spawn(async move {
                    let mut buffer = vec![0u8; 2048];
                    let Ok(read) = stream.read(&mut buffer).await else {
                        return;
                    };
                    let request = String::from_utf8_lossy(&buffer[..read]).to_string();
                    let path = request.split_whitespace().nth(1).unwrap_or("/").to_string();
                    let _ = serve(&mut stream, &path).await;
                });
            }
        });

        Self { base }
    }

    pub fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }
}

async fn serve(stream: &mut tokio::net::TcpStream, path: &str) -> std::io::Result<()> {
    let (status, content_type, body) = match path {
        "/" | "/plain" => (
            "200 OK",
            "text/html",
            "<html><body><h1>hello</h1></body></html>".to_string(),
        ),
        "/slow-css" => (
            "200 OK",
            "text/html",
            "<html><head><link rel=stylesheet href=/slow.css></head><body>slow</body></html>"
                .to_string(),
        ),
        "/slow.css" => {
            tokio::time::sleep(Duration::from_millis(700)).await;
            (
                "200 OK",
                "text/css",
                "body { color: rebeccapurple }".to_string(),
            )
        }
        "/missing-image" => (
            "200 OK",
            "text/html",
            "<html><body><img src=/gone.png></body></html>".to_string(),
        ),
        "/late-status" => (
            "200 OK",
            "text/html",
            "<html><body>waiting<script>\
             setTimeout(() => { window.status = 'ready'; }, 400);\
             </script></body></html>"
                .to_string(),
        ),
        "/media" => (
            "200 OK",
            "text/html",
            "<html><head><style>\
             body { --which: none }\
             @media screen { body { --which: screen } }\
             @media print  { body { --which: print } }\
             </style></head><body>media</body></html>"
                .to_string(),
        ),
        "/painted" => (
            "200 OK",
            "text/html",
            "<html><head><style>\
             html, body { margin: 0; height: 100%; }\
             body { background: repeating-linear-gradient(45deg, #333, #333 10px, #ccc 10px, #ccc 20px); }\
             </style></head><body></body></html>"
                .to_string(),
        ),
        "/scripted" => (
            "200 OK",
            "text/html",
            "<html><body><p id=t>before</p><script>\
             document.getElementById('t').textContent = 'after';\
             </script></body></html>"
                .to_string(),
        ),
        _ => ("404 Not Found", "text/plain", "not found".to_string()),
    };

    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(response.as_bytes()).await?;
    stream.flush().await
}
