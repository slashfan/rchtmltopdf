//! Turning a settled page into PDF bytes.

use crate::error::{Error, Result};
use crate::launch::Page;
use crate::plan::{self, Command};
use base64::Engine;
use serde_json::{Value, json};

/// How much of the PDF stream to ask for at a time.
const STREAM_CHUNK: u64 = 512 * 1024;

impl Page {
    /// Print the page and return the bytes.
    ///
    /// Takes the command the plan decided rather than the settings it was
    /// decided from, which is the strongest form of D27's contract: there is
    /// nothing left here to decide, and a choice that never reached the plan
    /// cannot be made on the way out.
    pub async fn print_to_pdf(&self, command: &Command) -> Result<Vec<u8>> {
        self.hold_the_margins(command).await;

        let result = self
            .session()
            .send(command.method, command.params.clone())
            .await?;

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

    /// Write the print call's own margins into the document, so its `@page`
    /// rule cannot take them (D60).
    ///
    /// Injected as a `<style>` on the end of `documentElement`, which is the
    /// last node of the document: a later rule of equal specificity wins, and
    /// no protocol command offers a stronger origin. **The end of the head is
    /// not late enough** — a document is free to put its `@page` rule in the
    /// body, and one that does would beat a rule injected above it. A document
    /// whose `@page` margins carry `!important` still wins, which is the same
    /// limitation the user stylesheet already carries.
    ///
    /// Failures are swallowed. The page is about to be printed with these very
    /// margins in the print call; a document that could not be reached to be
    /// told about them is no reason to refuse to print it.
    async fn hold_the_margins(&self, command: &Command) {
        let css = plan::page_box(command);
        let expression = format!(
            "(() => {{ const sheet = document.createElement('style'); \
             sheet.textContent = {}; \
             document.documentElement.appendChild(sheet); }})()",
            serde_json::to_string(&css).unwrap_or_else(|_| "\"\"".to_string())
        );
        let _ = self
            .session()
            .send(
                "Runtime.evaluate",
                json!({ "expression": expression, "returnByValue": true }),
            )
            .await;
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
