//! Turning a settled page into PDF bytes.

use crate::error::{Error, Result};
use crate::launch::Page;
use crate::plan;
use base64::Engine;
use rchtmltopdf_core::settings::{PageSetup, WebSettings};
use serde_json::{Value, json};

/// How much of the PDF stream to ask for at a time.
const STREAM_CHUNK: u64 = 512 * 1024;

impl Page {
    /// Print the page and return the bytes.
    ///
    /// Sends what [`plan::print`] decided and nothing else, so every choice
    /// about paper, margins, backgrounds and zoom is visible to the guard that
    /// holds the option table honest (D27).
    pub async fn print_to_pdf(&self, page: &PageSetup, web: &WebSettings) -> Result<Vec<u8>> {
        let command = plan::print(page, web);
        let result = self.session().send(command.method, command.params).await?;

        let handle = result
            .get("stream")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::Malformed {
                detail: "Page.printToPDF returned no stream handle".into(),
            })?
            .to_string();

        let bytes = self.drain_stream(&handle).await;
        // Close it whatever happened: the handle is a resource in the browser.
        let _ = self
            .session()
            .send("IO.close", json!({ "handle": handle }))
            .await;
        bytes
    }

    /// Read a browser-side stream to its end.
    async fn drain_stream(&self, handle: &str) -> Result<Vec<u8>> {
        let mut collected = Vec::new();
        let engine = base64::engine::general_purpose::STANDARD;

        loop {
            let chunk = self
                .session()
                .send("IO.read", json!({ "handle": handle, "size": STREAM_CHUNK }))
                .await?;

            let data = chunk
                .get("data")
                .and_then(Value::as_str)
                .unwrap_or_default();

            // PDF is binary, so it always arrives encoded. The flag is honoured
            // rather than assumed, because the protocol allows either.
            if chunk
                .get("base64Encoded")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                let decoded = engine.decode(data).map_err(|error| Error::Malformed {
                    detail: format!("the PDF stream was not valid base64: {error}"),
                })?;
                collected.extend_from_slice(&decoded);
            } else {
                collected.extend_from_slice(data.as_bytes());
            }

            if chunk.get("eof").and_then(Value::as_bool).unwrap_or(false) {
                break;
            }
            if data.is_empty() {
                // No progress and no end marker: stop rather than spin.
                return Err(Error::Malformed {
                    detail: "the PDF stream stopped without ending".into(),
                });
            }
        }

        if !collected.starts_with(b"%PDF-") {
            return Err(Error::Malformed {
                detail: "the browser returned something that is not a PDF".into(),
            });
        }

        Ok(collected)
    }
}
