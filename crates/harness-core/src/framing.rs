//! Newline-delimited JSON framing for worker IPC. One envelope per line, at most
//! [`MAX_FRAME_BYTES`] (excluding the newline). Works over any `BufRead`/`Write`, so the
//! subprocess adapter and tests share the exact same code.

use std::io::{self, BufRead, Write};

use crate::protocol::{Envelope, EnvelopeError};

pub const MAX_FRAME_BYTES: usize = 1024 * 1024;

#[derive(Debug)]
pub enum FrameError {
    /// A line exceeded the limit. The stream cannot be resynchronised; drop the peer.
    Oversize {
        limit: usize,
    },
    /// Not valid UTF-8 / JSON / envelope, or an empty line.
    BadFrame(String),
    /// The stream ended before the terminating newline.
    Truncated,
    Io(io::Error),
}

impl std::fmt::Display for FrameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Oversize { limit } => write!(f, "frame exceeds {limit} bytes"),
            Self::BadFrame(e) => write!(f, "bad frame: {e}"),
            Self::Truncated => write!(f, "stream ended mid-frame"),
            Self::Io(e) => write!(f, "io error: {e}"),
        }
    }
}

impl std::error::Error for FrameError {}

impl From<io::Error> for FrameError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<EnvelopeError> for FrameError {
    fn from(e: EnvelopeError) -> Self {
        Self::BadFrame(e.to_string())
    }
}

/// Serialize an envelope as a single line (including the trailing `\n`).
pub fn encode_frame(envelope: &Envelope) -> Result<Vec<u8>, FrameError> {
    envelope.validate()?;
    let mut bytes =
        serde_json::to_vec(envelope).map_err(|e| FrameError::BadFrame(e.to_string()))?;
    if bytes.len() > MAX_FRAME_BYTES {
        return Err(FrameError::Oversize {
            limit: MAX_FRAME_BYTES,
        });
    }
    bytes.push(b'\n');
    Ok(bytes)
}

pub fn write_frame<W: Write>(writer: &mut W, envelope: &Envelope) -> Result<(), FrameError> {
    let bytes = encode_frame(envelope)?;
    writer.write_all(&bytes)?;
    writer.flush()?;
    Ok(())
}

/// Parse one frame body (no newline).
pub fn decode_frame(line: &[u8]) -> Result<Envelope, FrameError> {
    if line.len() > MAX_FRAME_BYTES {
        return Err(FrameError::Oversize {
            limit: MAX_FRAME_BYTES,
        });
    }
    let text = std::str::from_utf8(line).map_err(|e| FrameError::BadFrame(e.to_string()))?;
    if text.trim().is_empty() {
        return Err(FrameError::BadFrame("empty frame".into()));
    }
    Ok(Envelope::from_json(text)?)
}

/// Read the next frame. `Ok(None)` is a clean end of stream.
pub fn read_frame<R: BufRead>(reader: &mut R) -> Result<Option<Envelope>, FrameError> {
    let mut line: Vec<u8> = Vec::new();
    loop {
        let chunk = reader.fill_buf()?;
        if chunk.is_empty() {
            return if line.is_empty() {
                Ok(None)
            } else {
                Err(FrameError::Truncated)
            };
        }
        match chunk.iter().position(|b| *b == b'\n') {
            Some(i) => {
                if line.len() + i > MAX_FRAME_BYTES {
                    return Err(FrameError::Oversize {
                        limit: MAX_FRAME_BYTES,
                    });
                }
                line.extend_from_slice(&chunk[..i]);
                reader.consume(i + 1);
                if line.last() == Some(&b'\r') {
                    line.pop();
                }
                return decode_frame(&line).map(Some);
            }
            None => {
                let n = chunk.len();
                if line.len() + n > MAX_FRAME_BYTES {
                    return Err(FrameError::Oversize {
                        limit: MAX_FRAME_BYTES,
                    });
                }
                line.extend_from_slice(chunk);
                reader.consume(n);
            }
        }
    }
}
