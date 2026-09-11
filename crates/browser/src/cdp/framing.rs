//! NUL-delimited message framing.
//!
//! Over the debugging pipe, protocol messages are separated by a zero byte, not
//! by a newline. That is the single most common thing to get wrong when moving
//! from the WebSocket transport, where the framing comes for free.
//!
//! JSON never contains a bare zero byte, so splitting on it is unambiguous and
//! needs no escaping.

use crate::error::{Error, Result};
use tokio::io::{AsyncRead, AsyncReadExt};

/// Refuse to buffer a single message larger than this.
///
/// This is a desynchronisation guard, not a real expectation about message size.
/// If framing goes wrong, the reader would otherwise grow a buffer until the
/// process dies. Genuinely large payloads are meant to be fetched as streams,
/// which is why printing to PDF asks for a stream handle rather than inline
/// base64.
pub const MAX_MESSAGE_BYTES: usize = 64 * 1024 * 1024;

/// Splits an incoming byte stream into protocol messages.
pub struct Framed<R> {
    reader: R,
    /// Bytes read but not yet handed out, including any partial message.
    buffer: Vec<u8>,
    /// How far into `buffer` we have already looked for a terminator. Without
    /// this, every extra chunk rescans the whole partial message and framing
    /// becomes quadratic on large payloads.
    scanned: usize,
    limit: usize,
}

impl<R: AsyncRead + Unpin> Framed<R> {
    pub fn new(reader: R) -> Self {
        Self::with_limit(reader, MAX_MESSAGE_BYTES)
    }

    pub fn with_limit(reader: R, limit: usize) -> Self {
        Self {
            reader,
            buffer: Vec::new(),
            scanned: 0,
            limit,
        }
    }

    /// The next complete message, or `None` at end of stream.
    ///
    /// Trailing bytes with no terminator at end of stream are discarded: a
    /// half-written message is not a message. That happens normally when the
    /// browser is killed mid-write.
    pub async fn next_message(&mut self) -> Result<Option<Vec<u8>>> {
        loop {
            if let Some(offset) = self.buffer[self.scanned..].iter().position(|&b| b == 0) {
                let end = self.scanned + offset;
                let message: Vec<u8> = self.buffer.drain(..=end).take(end).collect();
                self.scanned = 0;
                // Empty messages are legal framing noise, not an error.
                if message.is_empty() {
                    continue;
                }
                return Ok(Some(message));
            }

            self.scanned = self.buffer.len();

            if self.buffer.len() > self.limit {
                return Err(Error::MessageTooLarge { limit: self.limit });
            }

            let mut chunk = [0u8; 8192];
            let read = self.reader.read(&mut chunk).await?;
            if read == 0 {
                return Ok(None);
            }
            self.buffer.extend_from_slice(&chunk[..read]);
        }
    }
}

/// Frame one message for sending: the payload, then the terminator.
pub fn frame(payload: &[u8]) -> Vec<u8> {
    let mut framed = Vec::with_capacity(payload.len() + 1);
    framed.extend_from_slice(payload);
    framed.push(0);
    framed
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn collect(input: &[u8]) -> Vec<String> {
        let mut framed = Framed::new(input);
        let mut out = Vec::new();
        while let Some(message) = framed.next_message().await.unwrap() {
            out.push(String::from_utf8(message).unwrap());
        }
        out
    }

    #[tokio::test]
    async fn splits_on_nul_not_newline() {
        // A newline inside a message must not split it.
        let out = collect(b"{\"a\":1}\0{\"b\":\n2}\0").await;
        assert_eq!(out, vec!["{\"a\":1}", "{\"b\":\n2}"]);
    }

    #[tokio::test]
    async fn a_message_split_across_reads_is_reassembled() {
        // tokio::io::duplex delivers in chunks, which is the realistic case.
        let (mut writer, reader) = tokio::io::duplex(8);
        let task = tokio::spawn(async move {
            let mut framed = Framed::new(reader);
            framed.next_message().await.unwrap()
        });
        tokio::io::AsyncWriteExt::write_all(&mut writer, b"{\"method\":\"Page.loadEventFired\"}\0")
            .await
            .unwrap();
        let message = task.await.unwrap().unwrap();
        assert_eq!(
            String::from_utf8(message).unwrap(),
            "{\"method\":\"Page.loadEventFired\"}"
        );
    }

    #[tokio::test]
    async fn end_of_stream_returns_none() {
        assert!(collect(b"").await.is_empty());
    }

    #[tokio::test]
    async fn a_half_written_message_at_end_of_stream_is_dropped() {
        // Exactly what a killed browser leaves behind.
        let out = collect(b"{\"a\":1}\0{\"incomp").await;
        assert_eq!(out, vec!["{\"a\":1}"]);
    }

    #[tokio::test]
    async fn empty_messages_are_skipped() {
        let out = collect(b"\0\0{\"a\":1}\0\0").await;
        assert_eq!(out, vec!["{\"a\":1}"]);
    }

    #[tokio::test]
    async fn an_endless_message_hits_the_limit_instead_of_eating_memory() {
        let flood = vec![b'x'; 4096];
        let mut framed = Framed::with_limit(&flood[..], 1024);
        match framed.next_message().await {
            Err(Error::MessageTooLarge { limit }) => assert_eq!(limit, 1024),
            other => panic!("expected MessageTooLarge, got {other:?}"),
        }
    }

    #[test]
    fn framing_appends_the_terminator() {
        assert_eq!(frame(b"{}"), b"{}\0");
    }

    #[tokio::test]
    async fn framing_round_trips() {
        let mut stream = Vec::new();
        stream.extend_from_slice(&frame(br#"{"id":1}"#));
        stream.extend_from_slice(&frame(br#"{"id":2}"#));
        let out = collect(&stream).await;
        assert_eq!(out, vec![r#"{"id":1}"#, r#"{"id":2}"#]);
    }
}
