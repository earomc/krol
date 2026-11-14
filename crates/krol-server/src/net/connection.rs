//! A client connection with vanilla's pipeline: decryption → framing → decompression
//! (and the reverse for sending).

use std::time::Duration;

use krol_protocol::cipher::Cfb8;
use krol_protocol::compression;
use krol_protocol::frame::{FrameDecoder, encode_frame};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadHalf, WriteHalf};
use tokio::time::Instant;

use super::{ConnectionError, READ_TIMEOUT};

/// Size of each socket read (Netty's initial adaptive receive buffer).
pub(super) const READ_CHUNK: usize = 1024;

/// A framed, optionally encrypted and compressed connection.
pub(super) struct Connection<S> {
    stream: S,
    decoder: FrameDecoder,
    encryptor: Option<Cfb8>,
    decryptor: Option<Cfb8>,
    threshold: Option<usize>,
}

impl<S: AsyncRead + AsyncWrite + Unpin> Connection<S> {
    /// Wraps `stream`; `initial` holds bytes already read from it.
    pub(super) fn new(stream: S, initial: &[u8]) -> Self {
        let mut decoder = FrameDecoder::new();
        decoder.extend(initial);
        Self {
            stream,
            decoder,
            encryptor: None,
            decryptor: None,
            threshold: None,
        }
    }

    /// The next packet body (decompressed), `None` on a clean close by the client.
    ///
    /// Fails with [`ConnectionError::Timeout`] after [`READ_TIMEOUT`] without data and with
    /// [`ConnectionError::Deadline`] once `deadline` passes.
    pub(super) async fn next_packet(
        &mut self,
        deadline: Option<Instant>,
    ) -> Result<Option<Vec<u8>>, ConnectionError> {
        let mut buf = [0u8; READ_CHUNK];
        loop {
            if let Some(frame) = self.decoder.next_frame()? {
                let packet = match self.threshold {
                    Some(t) => compression::decompress(&frame, t)?,
                    None => frame,
                };
                return Ok(Some(packet));
            }
            let read_deadline = Instant::now() + READ_TIMEOUT;
            let (limit, is_deadline) = match deadline {
                Some(d) if d < read_deadline => (d, true),
                _ => (read_deadline, false),
            };
            let n = match tokio::time::timeout_at(limit, self.stream.read(&mut buf)).await {
                Ok(result) => result?,
                Err(_) if is_deadline => return Err(ConnectionError::Deadline),
                Err(_) => return Err(ConnectionError::Timeout),
            };
            if n == 0 {
                return Ok(None);
            }
            let data = &mut buf[..n];
            if let Some(dec) = &mut self.decryptor {
                dec.apply(data);
            }
            self.decoder.extend(data);
        }
    }

    /// Sends a packet body through compression, framing and encryption.
    pub(super) async fn send(&mut self, body: &[u8]) -> Result<(), ConnectionError> {
        let payload = match self.threshold {
            Some(t) => compression::compress(body, t),
            None => body.to_vec(),
        };
        let mut bytes = encode_frame(&payload)?;
        if let Some(enc) = &mut self.encryptor {
            enc.apply(&mut bytes);
        }
        self.stream.write_all(&bytes).await?;
        self.stream.flush().await?;
        Ok(())
    }

    /// Turns on AES/CFB-8 in both directions. Bytes already buffered but not yet framed
    /// were sent encrypted, so they are decrypted now.
    pub(super) fn enable_encryption(&mut self, secret: &[u8; 16]) {
        let mut decryptor = Cfb8::decryptor(secret);
        let mut pending = self.decoder.buffered().to_vec();
        decryptor.apply(&mut pending);
        self.decoder = FrameDecoder::new();
        self.decoder.extend(&pending);
        self.decryptor = Some(decryptor);
        self.encryptor = Some(Cfb8::encryptor(secret));
    }

    /// Turns on compression for subsequent packets in both directions.
    pub(super) fn enable_compression(&mut self, threshold: usize) {
        self.threshold = Some(threshold);
    }

    /// Flushes and closes the write side.
    pub(super) async fn close(&mut self) {
        let _ = tokio::time::timeout(Duration::from_secs(5), self.stream.shutdown()).await;
    }

    /// Splits into independently usable read and write halves, keeping cipher and
    /// compression state.
    pub(super) fn into_split(self) -> (ConnReader<ReadHalf<S>>, ConnWriter<WriteHalf<S>>) {
        let (read, write) = tokio::io::split(self.stream);
        (
            ConnReader {
                inner: Connection {
                    stream: read,
                    decoder: self.decoder,
                    encryptor: None,
                    decryptor: self.decryptor,
                    threshold: self.threshold,
                },
            },
            ConnWriter {
                stream: write,
                encryptor: self.encryptor,
                threshold: self.threshold,
            },
        )
    }
}

/// The read half of a split connection.
pub(super) struct ConnReader<R> {
    inner: Connection<R>,
}

impl<R: AsyncRead + Unpin> ConnReader<R> {
    /// Reads and discards whatever the client still sends until it closes the
    /// connection or `limit` passes. Used after the server closed its side: dropping a
    /// socket with unread data resets it (notably on Windows), which can destroy the
    /// disconnect packet before the client reads it.
    pub(super) async fn drain(&mut self, limit: Duration) {
        let mut buf = [0u8; READ_CHUNK];
        let deadline = Instant::now() + limit;
        while let Ok(Ok(n)) =
            tokio::time::timeout_at(deadline, self.inner.stream.read(&mut buf)).await
        {
            if n == 0 {
                break;
            }
        }
    }

    /// See [`Connection::next_packet`].
    pub(super) async fn next_packet(&mut self) -> Result<Option<Vec<u8>>, ConnectionError> {
        let mut buf = [0u8; READ_CHUNK];
        let c = &mut self.inner;
        loop {
            if let Some(frame) = c.decoder.next_frame()? {
                return Ok(Some(match c.threshold {
                    Some(t) => compression::decompress(&frame, t)?,
                    None => frame,
                }));
            }
            let n = match tokio::time::timeout(READ_TIMEOUT, c.stream.read(&mut buf)).await {
                Ok(result) => result?,
                Err(_) => return Err(ConnectionError::Timeout),
            };
            if n == 0 {
                return Ok(None);
            }
            let data = &mut buf[..n];
            if let Some(dec) = &mut c.decryptor {
                dec.apply(data);
            }
            c.decoder.extend(data);
        }
    }
}

/// The write half of a split connection.
pub(super) struct ConnWriter<W> {
    stream: W,
    encryptor: Option<Cfb8>,
    threshold: Option<usize>,
}

impl<W: AsyncWrite + Unpin> ConnWriter<W> {
    /// Sends a packet body through compression, framing and encryption.
    pub(super) async fn send(&mut self, body: &[u8]) -> Result<(), ConnectionError> {
        let payload = match self.threshold {
            Some(t) => compression::compress(body, t),
            None => body.to_vec(),
        };
        let mut bytes = encode_frame(&payload)?;
        if let Some(enc) = &mut self.encryptor {
            enc.apply(&mut bytes);
        }
        self.stream.write_all(&bytes).await?;
        self.stream.flush().await?;
        Ok(())
    }

    /// Flushes and closes the write side.
    pub(super) async fn close(&mut self) {
        let _ = tokio::time::timeout(Duration::from_secs(5), self.stream.shutdown()).await;
    }
}
