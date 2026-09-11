//! Turning a settled page into PDF bytes.

use crate::error::{Error, Result};
use crate::launch::Page;
use base64::Engine;
use rchtmltopdf_core::settings::{MediaType, PageSetup, WebSettings};
use serde_json::{Value, json};

/// How much of the PDF stream to ask for at a time.
const STREAM_CHUNK: u64 = 512 * 1024;

impl Page {
    /// Choose which stylesheets apply.
    ///
    /// **This is the easiest thing in the whole project to get wrong.** Chromium
    /// prints with `print` stylesheets unless told otherwise; wkhtmltopdf renders
    /// with `screen` ones. A migrated document that has `@media print` rules it
    /// never used before will silently change layout if this is skipped, and so
    /// will every fixture calibrated against it (D03).
    ///
    /// Set before the document loads rather than just before printing, because
    /// media queries decide which resources are fetched at all.
    pub async fn emulate_media(&self, web: &WebSettings) -> Result<()> {
        let media = match web.media_type {
            MediaType::Screen => "screen",
            MediaType::Print => "print",
        };
        self.session()
            .send("Emulation.setEmulatedMedia", json!({ "media": media }))
            .await?;
        Ok(())
    }

    /// Print the page and return the bytes.
    ///
    /// Orientation is resolved before the call, so the paper handed over is
    /// already the right way round and this is the only place it is decided.
    /// Letting the protocol rotate it as well would apply the swap twice.
    /// Paper geometry is global in wkhtmltopdf; backgrounds and zoom belong to
    /// the object. Both are needed, and they come from different places.
    pub async fn print_to_pdf(&self, page: &PageSetup, web: &WebSettings) -> Result<Vec<u8>> {
        let result = self
            .session()
            .send(
                "Page.printToPDF",
                json!({
                    "paperWidth": page.width_inches(),
                    "paperHeight": page.height_inches(),
                    "marginTop": page.margins.top.to_inches(),
                    "marginBottom": page.margins.bottom.to_inches(),
                    "marginLeft": page.margins.left.to_inches(),
                    "marginRight": page.margins.right.to_inches(),
                    "printBackground": web.background,
                    "scale": web.zoom,
                    // Orientation is already in the paper dimensions above.
                    "landscape": false,
                    // Leave this off. Turning it on lets a document's own @page
                    // rule override --page-size, and wkhtmltopdf does not do
                    // that, so neither do we.
                    "preferCSSPageSize": false,
                    // Not the default. The default returns the whole document
                    // base64-encoded inside one protocol message, and a document
                    // of any size exceeds the message limit.
                    "transferMode": "ReturnAsStream",
                }),
            )
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
