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
//! TightPng encoding is like Tight but uses PNG compression instead of JPEG.
//! This provides lossless compression with good compression ratios.

use bytes::{BufMut, BytesMut};
use flate2::write::ZlibEncoder;
use flate2::Compression;
use std::io::Write;
use std::collections::HashMap;
use super::Encoding;
use super::common::{rgba_to_rgb24_pixels, check_solid_color, build_palette, put_pixel32};
use crate::protocol::TIGHT_PNG;

/// Implements the VNC "TightPng" encoding with PNG, palette, and solid fill support.
pub struct TightPngEncoding;

impl Encoding for TightPngEncoding {
    fn encode(&self, data: &[u8], width: u16, height: u16, _quality: u8, compression: u8) -> BytesMut {
        // Intelligently choose the best encoding method based on image content

        // Method 1: Check if it's a solid color
        let pixels = rgba_to_rgb24_pixels(data);
        if let Some(solid_color) = check_solid_color(&pixels) {
            return encode_tightpng_solid(solid_color);
        }

        // Method 2: Check if palette encoding would be good
        // Tight indexed color only supports 2-16 colors (RFC 6143 Section 7.7.5)
        let palette = build_palette(&pixels);
        if palette.len() >= 2 && palette.len() <= 16 && palette.len() < pixels.len() / 4 {
            return encode_tightpng_palette(&pixels, width, height, &palette, compression);
        }

        // Method 3: Use PNG for all other content (lossless compression)
        encode_tightpng_png(data, width, height, compression)
    }
}

/// Encode as TightPng solid fill.
fn encode_tightpng_solid(color: u32) -> BytesMut {
    let mut buf = BytesMut::with_capacity(5);
    buf.put_u8(0x80); // Fill compression (solid color)
    put_pixel32(&mut buf, color); // 4 bytes for 32bpp
    buf
}

/// Encode as TightPng palette.
fn encode_tightpng_palette(pixels: &[u32], _width: u16, _height: u16, palette: &[u32], compression: u8) -> BytesMut {
    let palette_size = palette.len();

    // Build color-to-index map
    let mut color_map: HashMap<u32, u8> = HashMap::new();
    for (idx, &color) in palette.iter().enumerate() {
        color_map.insert(color, idx as u8);
    }

    // Encode pixels as palette indices
    let mut indices = Vec::with_capacity(pixels.len());
    for &pixel in pixels {
        indices.push(*color_map.get(&pixel).unwrap_or(&0));
    }

    // Compress indices
    let compression_level = match compression {
        0 => Compression::fast(),
        1..=3 => Compression::new(compression as u32),
        4..=6 => Compression::default(),
        _ => Compression::best(),
    };

    let mut encoder = ZlibEncoder::new(Vec::new(), compression_level);
    if encoder.write_all(&indices).is_err() {
        // Compression failed, fall back to PNG encoding
        return encode_tightpng_png(
            &pixels.iter().flat_map(|&p| {
                vec![(p & 0xFF) as u8, ((p >> 8) & 0xFF) as u8, ((p >> 16) & 0xFF) as u8, 0xFF]
            }).collect::<Vec<u8>>(),
            _width, _height, compression
        );
    }
    let compressed = match encoder.finish() {
        Ok(data) => data,
        Err(_) => {
            // Compression failed, fall back to PNG encoding
            return encode_tightpng_png(
                &pixels.iter().flat_map(|&p| {
                    vec![(p & 0xFF) as u8, ((p >> 8) & 0xFF) as u8, ((p >> 16) & 0xFF) as u8, 0xFF]
                }).collect::<Vec<u8>>(),
                _width, _height, compression
            );
        }
    };

    let mut buf = BytesMut::new();

    // Compression control byte: palette compression
    buf.put_u8(0x80 | ((palette_size - 1) as u8));

    // Palette (each color is 4 bytes for 32bpp)
    for &color in palette {
        put_pixel32(&mut buf, color);
    }

    // Compact length
    let len = compressed.len();
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

    buf.put_slice(&compressed);
    buf
}

/// Encode as TightPng using PNG compression.
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
