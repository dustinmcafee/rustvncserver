//! VNC Raw encoding implementation.
//!
//! The simplest encoding that sends pixel data directly without compression.
//! High bandwidth but universally supported.

use bytes::{BufMut, BytesMut};
use super::Encoding;

/// Implements the VNC "Raw" encoding, which sends pixel data directly without compression.
///
/// This encoding is straightforward but can be very bandwidth-intensive as it transmits
/// the raw framebuffer data in RGB format (without alpha channel).
pub struct RawEncoding;

impl Encoding for RawEncoding {
    fn encode(&self, data: &[u8], _width: u16, _height: u16, _quality: u8, _compression: u8) -> BytesMut {
        // For 32bpp clients: convert RGBA to client pixel format (RGBX where X is padding)
        // Client format: R at bits 0-7, G at bits 8-15, B at bits 16-23, padding at bits 24-31
        let mut buf = BytesMut::with_capacity(data.len()); // Same size: 4 bytes per pixel
        for chunk in data.chunks_exact(4) {
            buf.put_u8(chunk[0]); // R at byte 0
            buf.put_u8(chunk[1]); // G at byte 1
            buf.put_u8(chunk[2]); // B at byte 2
            buf.put_u8(0);        // Padding at byte 3 (not alpha)
        }
        buf
    }
}
