// Copyright 2025 Dustin McAfee
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.


//! VNC TightPng encoding implementation.
//!
//! TightPng encoding uses PNG compression exclusively for all rectangles.
//! Unlike standard Tight encoding which supports multiple compression modes
//! (solid fill, palette, zlib, JPEG), TightPng ONLY uses PNG mode.
//!
//! This design is optimized for browser-based VNC clients like noVNC,
//! which can decode PNG data natively in hardware without needing to
//! handle zlib decompression or palette operations.

use bytes::{BufMut, BytesMut};
use super::Encoding;
use crate::protocol::TIGHT_PNG;

/// Implements the VNC "TightPng" encoding (encoding -260).
///
/// TightPng sends all pixel data as PNG-compressed images, regardless of
/// content. This differs from standard Tight encoding which uses multiple
/// compression strategies.
pub struct TightPngEncoding;

impl Encoding for TightPngEncoding {
    fn encode(&self, data: &[u8], width: u16, height: u16, _quality: u8, compression: u8) -> BytesMut {
        // TightPng ONLY uses PNG mode - no solid fill, no palette modes
        // This is the key difference from standard Tight encoding
        // Browser-based clients like noVNC expect only PNG data
        encode_tightpng_png(data, width, height, compression)
    }
}

/// Encode as TightPng using PNG compression.
///
/// This is the only compression mode used by TightPng encoding.
fn encode_tightpng_png(data: &[u8], width: u16, height: u16, compression: u8) -> BytesMut {
    use png::{Encoder, ColorType, BitDepth};

    // Convert RGBA to RGB (PNG encoder will handle this)
    let mut rgb_data = Vec::with_capacity((width as usize) * (height as usize) * 3);
    for chunk in data.chunks_exact(4) {
        rgb_data.push(chunk[0]);
        rgb_data.push(chunk[1]);
        rgb_data.push(chunk[2]);
    }

    // Create PNG encoder
    let mut png_data = Vec::new();
    {
        let mut encoder = Encoder::new(&mut png_data, width as u32, height as u32);
        encoder.set_color(ColorType::Rgb);
        encoder.set_depth(BitDepth::Eight);

        // Map TightVNC compression level (0-9) to PNG compression (0-9 maps to Fast/Default/Best)
        let png_compression = match compression {
            0..=2 => png::Compression::Fast,
            3..=6 => png::Compression::Default,
            _ => png::Compression::Best,
        };
        encoder.set_compression(png_compression);

        let mut writer = match encoder.write_header() {
            Ok(w) => w,
            Err(e) => {
                log::error!("PNG header write failed: {}, falling back to basic encoding", e);
                // Fall back to basic tight encoding
                let mut buf = BytesMut::with_capacity(1 + data.len());
                buf.put_u8(0x00); // Basic tight encoding, no compression
                for chunk in data.chunks_exact(4) {
                    buf.put_u8(chunk[0]); // R
                    buf.put_u8(chunk[1]); // G
                    buf.put_u8(chunk[2]); // B
                    buf.put_u8(0);        // Padding
                }
                return buf;
            }
        };

        if let Err(e) = writer.write_image_data(&rgb_data) {
            log::error!("PNG data write failed: {}, falling back to basic encoding", e);
            // Fall back to basic tight encoding
            let mut buf = BytesMut::with_capacity(1 + data.len());
            buf.put_u8(0x00); // Basic tight encoding, no compression
            for chunk in data.chunks_exact(4) {
                buf.put_u8(chunk[0]); // R
                buf.put_u8(chunk[1]); // G
                buf.put_u8(chunk[2]); // B
                buf.put_u8(0);        // Padding
            }
            return buf;
        }
    }

    let mut buf = BytesMut::new();
    buf.put_u8(TIGHT_PNG << 4); // PNG subencoding

    // Compact length
    let len = png_data.len();
    if len < 128 {
        buf.put_u8(len as u8);
    } else if len < 16384 {
        buf.put_u8(((len & 0x7F) | 0x80) as u8);
        buf.put_u8((len >> 7) as u8);
    } else {
        buf.put_u8(((len & 0x7F) | 0x80) as u8);
        buf.put_u8((((len >> 7) & 0x7F) | 0x80) as u8);
        buf.put_u8((len >> 14) as u8);
    }

    buf.put_slice(&png_data);
    buf
}
