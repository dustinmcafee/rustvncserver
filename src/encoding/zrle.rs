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


//! ZRLE (Zlib Run-Length Encoding) implementation for VNC.
//!
//! ZRLE is a highly efficient encoding that combines tiling, palette-based compression,
//! run-length encoding, and zlib compression. It is effective for a wide range of
//! screen content.
//!
//! # Encoding Process
//!
//! 1. The framebuffer region is divided into 64x64 pixel tiles.
//! 2. Each tile is compressed independently.
//! 3. The compressed data for all tiles is concatenated and then compressed as a whole
//!    using zlib.
//!
//! # Tile Sub-encodings
//!
//! Each tile is analyzed and compressed using one of the following methods:
//! - **Raw:** If not otherwise compressible, sent as uncompressed RGBA data.
//! - **Solid Color:** If the tile contains only one color.
//! - **Packed Palette:** If the tile contains 2-16 unique colors. Pixels are sent as
//!   palette indices, which can be run-length encoded.
//! - **Plain RLE:** If the tile has more than 16 colors but is still compressible with RLE.
//!

use bytes::{BufMut, BytesMut};
use flate2::write::ZlibEncoder;
use flate2::{Compress, Compression, FlushCompress, Status};
use std::io::Write;
use std::collections::HashMap;

use super::Encoding;
use crate::protocol::PixelFormat;

const TILE_SIZE: usize = 64;

/// Analyzes pixel data to count RLE runs, single pixels, and unique colors.
fn analyze_runs_and_palette(pixels: &[u32]) -> (usize, usize, HashMap<u32, usize>) {
    let mut runs = 0;
    let mut single_pixels = 0;
    let mut unique_colors: HashMap<u32, usize> = HashMap::new();

    if pixels.is_empty() {
        return (0, 0, unique_colors);
    }

    let mut i = 0;
    while i < pixels.len() {
        let color = pixels[i];
        *unique_colors.entry(color).or_insert(0) += 1;

        let mut run_len = 1;
        while i + run_len < pixels.len() && pixels[i + run_len] == color {
            run_len += 1;
        }

        if run_len == 1 {
            single_pixels += 1;
        } else {
            runs += 1;
        }
        i += run_len;
    }
    (runs, single_pixels, unique_colors)
}

/// Encodes a rectangle of pixel data using ZRLE with a persistent compressor.
/// This maintains compression state across rectangles as required by RFC 6143.
#[allow(dead_code)]
pub fn encode_zrle_persistent(
    data: &[u8],
    width: u16,
    height: u16,
    _pixel_format: &PixelFormat,
    compressor: &mut Compress,
) -> std::io::Result<Vec<u8>> {
    let width = width as usize;
    let height = height as usize;
    let mut uncompressed_data = BytesMut::new();

    for y in (0..height).step_by(TILE_SIZE) {
        for x in (0..width).step_by(TILE_SIZE) {
            let tile_w = (width - x).min(TILE_SIZE);
            let tile_h = (height - y).min(TILE_SIZE);

            // Extract tile pixel data
            let tile_data = extract_tile(data, width, x, y, tile_w, tile_h);

            // Analyze and encode the tile
            encode_tile(&mut uncompressed_data, &tile_data, tile_w, tile_h);
        }
    }

    // Compress using persistent compressor
    let input = &uncompressed_data[..];
    let mut compressed_output = Vec::new();

    // Conservative buffer size for chunks
    let chunk_size = 65536; // 64KB
    let mut output_buf = vec![0u8; chunk_size];

    let before_in = compressor.total_in();
    let before_out = compressor.total_out();

    let mut input_pos = 0;

    // Loop until all input is consumed
    loop {
        let status = compressor.compress(
            &input[input_pos..],
            &mut output_buf,
            FlushCompress::Sync
        )?;

        // Calculate how much input was consumed and output was produced
        let total_in = compressor.total_in();
        let total_out = compressor.total_out();

        let consumed = (total_in - before_in - input_pos as u64) as usize;
        let produced = (total_out - before_out - compressed_output.len() as u64) as usize;

        // Append produced output
        if produced > 0 {
            compressed_output.extend_from_slice(&output_buf[..produced]);
        }

        input_pos += consumed;

        // Check if we're done
        match status {
            Status::Ok => {
                // More work needed, continue
                if input_pos >= input.len() {
                    // All input consumed but still returning Ok, finish with another call
                    continue;
                }
            },
            Status::BufError => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    "Compression buffer error"
                ));
            },
            Status::StreamEnd => {
                break; // Done
            }
        }

        // Check if all input consumed
        if input_pos >= input.len() {
            break;
        }
    }

    // Verify all input was consumed
    let total_consumed = (compressor.total_in() - before_in) as usize;
    if total_consumed != input.len() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::Other,
            format!("ZRLE: Not all input consumed: {}/{}", total_consumed, input.len())
        ));
    }

    // Build result with length prefix (big-endian) + compressed data
    let mut result = BytesMut::with_capacity(4 + compressed_output.len());
    result.put_u32(compressed_output.len() as u32);
    result.extend_from_slice(&compressed_output);

    log::info!("ZRLE: compressed {}->{}  bytes ({}x{} tiles)",
               uncompressed_data.len(), compressed_output.len(), width, height);

    Ok(result.to_vec())
}

/// Encodes a rectangle of pixel data using the ZRLE encoding.
/// This creates a new compressor for each rectangle (non-RFC compliant, deprecated).
pub fn encode_zrle(
    data: &[u8],
    width: u16,
    height: u16,
    _pixel_format: &PixelFormat, // Assuming RGBA32
    compression: u8,
) -> std::io::Result<Vec<u8>> {
    let compression_level = match compression {
        0 => Compression::fast(),
        1..=3 => Compression::new(compression as u32),
        4..=6 => Compression::default(),
        _ => Compression::best(),
    };
    let mut zlib_encoder = ZlibEncoder::new(Vec::new(), compression_level);
    let mut uncompressed_data = BytesMut::new();

    let width = width as usize;
    let height = height as usize;

    for y in (0..height).step_by(TILE_SIZE) {
        for x in (0..width).step_by(TILE_SIZE) {
            let tile_w = (width - x).min(TILE_SIZE);
            let tile_h = (height - y).min(TILE_SIZE);

            // Extract tile pixel data
            let tile_data = extract_tile(data, width, x, y, tile_w, tile_h);

            // Analyze and encode the tile
            encode_tile(&mut uncompressed_data, &tile_data, tile_w, tile_h);
        }
    }

    zlib_encoder.write_all(&uncompressed_data)?;
    let compressed = zlib_encoder.finish()?;

    // ZRLE requires a 4-byte big-endian length prefix before the zlib data
    let mut result = BytesMut::with_capacity(4 + compressed.len());
    result.put_u32(compressed.len() as u32); // big-endian length
    result.extend_from_slice(&compressed);

    Ok(result.to_vec())
}

/// Encodes a single tile, choosing the best sub-encoding.
/// Encodes a single tile, choosing the best sub-encoding.
fn encode_tile(buf: &mut BytesMut, tile_data: &[u8], width: usize, height: usize) {
    let pixels = rgba_to_rgb24_pixels(tile_data);
    let (runs, single_pixels, unique_colors) = analyze_runs_and_palette(&pixels);

    // Solid tile is a special case
    if unique_colors.len() == 1 {
        encode_solid_color_tile(buf, pixels[0]);
        return;
    }

    const CPIXEL_SIZE: usize = 3; // CPIXEL is 3 bytes for depth=24
    let mut use_rle = false;
    let mut use_palette = false;

    // Start assuming raw encoding size
    let mut estimated_bytes = width * height * CPIXEL_SIZE;

    let plain_rle_bytes = (CPIXEL_SIZE + 1) * (runs + single_pixels);

    if plain_rle_bytes < estimated_bytes {
        use_rle = true;
        estimated_bytes = plain_rle_bytes;
    }

    if unique_colors.len() < 128 {
        let palette: Vec<_> = unique_colors.keys().cloned().collect();
        let palette_size = palette.len();

        // Palette RLE encoding
        let palette_rle_bytes = CPIXEL_SIZE * palette_size + 2 * runs + single_pixels;

        if palette_rle_bytes < estimated_bytes {
            use_rle = true;
            use_palette = true;
            estimated_bytes = palette_rle_bytes;
        }

        // Packed palette encoding (no RLE)
        if palette_size < 17 {
            let bits_per_packed_pixel = match palette_size {
                2 => 1,
                3..=4 => 2,
                _ => 4, // 5-16 colors
            };
            // Round up: (bits + 7) / 8 to match actual encoding
            let packed_bytes = CPIXEL_SIZE * palette_size + (width * height * bits_per_packed_pixel + 7) / 8;

            if packed_bytes < estimated_bytes {
                use_rle = false;
                use_palette = true;
                // No need to update estimated_bytes, this is the last check
            }
        }
    }

    if !use_palette {
        // Raw or Plain RLE
        if use_rle {
            // Plain RLE
            buf.put_u8(128);
            let encoded_rle = encode_rle(&pixels);
            buf.extend_from_slice(&encoded_rle);
        } else {
            // Raw
            encode_raw_tile(buf, tile_data);
        }
    } else {
        // Palette (Packed Palette or Packed Palette RLE)
        let palette: Vec<_> = unique_colors.keys().cloned().collect();
        let color_to_idx: HashMap<_, _> = palette.iter().enumerate().map(|(i, &c)| (c, i as u8)).collect();

        if use_rle {
            // Packed Palette RLE
            encode_packed_palette_rle_tile(buf, &pixels, &palette, &color_to_idx);
        } else {
            // Packed Palette (no RLE)
            encode_packed_palette_tile(buf, &pixels, width, height, &unique_colors);
        }
    }
}

/// Extracts a tile from the full framebuffer.
fn extract_tile(
    full_frame: &[u8],
    frame_width: usize,
    x: usize,
    y: usize,
    width: usize,
    height: usize,
) -> Vec<u8> {
    let mut tile_data = Vec::with_capacity(width * height * 4);
    for row in 0..height {
        let start = ((y + row) * frame_width + x) * 4;
        let end = start + width * 4;
        tile_data.extend_from_slice(&full_frame[start..end]);
    }
    tile_data
}

/// Converts RGBA to 32-bit RGB pixels (0x00BBGGRR format for VNC).
fn rgba_to_rgb24_pixels(data: &[u8]) -> Vec<u32> {
    data.chunks_exact(4)
        .map(|c| (c[0] as u32) | ((c[1] as u32) << 8) | ((c[2] as u32) << 16))
        .collect()
}

/// Writes a CPIXEL (3 bytes for depth=24) in little-endian format.
/// CPIXEL format: R at byte 0, G at byte 1, B at byte 2
fn put_cpixel(buf: &mut BytesMut, pixel: u32) {
    buf.put_u8((pixel & 0xFF) as u8);          // R at bits 0-7
    buf.put_u8(((pixel >> 8) & 0xFF) as u8);   // G at bits 8-15
    buf.put_u8(((pixel >> 16) & 0xFF) as u8);  // B at bits 16-23
}

/// Sub-encoding for a tile with a single color.
fn encode_solid_color_tile(buf: &mut BytesMut, color: u32) {
    buf.put_u8(1); // Solid color sub-encoding
    put_cpixel(buf, color); // Write 3-byte CPIXEL
}

/// Sub-encoding for raw pixel data.
fn encode_raw_tile(buf: &mut BytesMut, tile_data: &[u8]) {
    buf.put_u8(0); // Raw sub-encoding
    // Convert RGBA (4 bytes) to CPIXEL (3 bytes) for each pixel
    for chunk in tile_data.chunks_exact(4) {
        buf.put_u8(chunk[0]); // R
        buf.put_u8(chunk[1]); // G
        buf.put_u8(chunk[2]); // B (skip alpha channel)
    }
}

/// Sub-encoding for a tile with a small palette.
fn encode_packed_palette_tile(
    buf: &mut BytesMut,
    pixels: &[u32],
    _width: usize,
    _height: usize,
    unique_colors: &HashMap<u32, usize>,
) {
    let palette: Vec<_> = unique_colors.keys().cloned().collect();
    let palette_size = palette.len();
    let bits_per_pixel = match palette_size {
        2 => 1,
        3..=4 => 2,
        _ => 4,
    };

    buf.put_u8(palette_size as u8); // Packed palette sub-encoding

    // Write palette as CPIXEL (3 bytes each)
    for &color in &palette {
        put_cpixel(buf, color);
    }

    // Write packed pixel data (RFC 6143: pack from MSB to LSB)
    let mut packed_byte = 0;
    let mut bit_pos = 0;
    let color_to_idx: HashMap<_, _> = palette.iter().enumerate().map(|(i, &c)| (c, i)).collect();

    for &pixel in pixels {
        let idx = color_to_idx[&pixel] as u8;
        // Pack from MSB: first pixel goes in high bits
        let shift = 8 - bit_pos - bits_per_pixel;
        packed_byte |= idx << shift;
        bit_pos += bits_per_pixel;
        if bit_pos >= 8 {
            buf.put_u8(packed_byte);
            packed_byte = 0;
            bit_pos = 0;
        }
    }

    if bit_pos > 0 {
        buf.put_u8(packed_byte);
    }
}


/// Sub-encoding for a tile with a small palette and RLE.
fn encode_packed_palette_rle_tile(
    buf: &mut BytesMut,
    pixels: &[u32],
    palette: &[u32],
    color_to_idx: &HashMap<u32, u8>,
) {
    let palette_size = palette.len();
    buf.put_u8(128 | (palette_size as u8)); // Packed palette RLE sub-encoding

    // Write palette as CPIXEL (3 bytes each)
    for &color in palette {
        put_cpixel(buf, color);
    }

    // Write RLE data using palette indices
    let mut i = 0;
    while i < pixels.len() {
        let color = pixels[i];
        let index = color_to_idx[&color];

        let mut run_len = 1;
        while i + run_len < pixels.len() && pixels[i + run_len] == color {
            run_len += 1;
        }

        if run_len == 1 {
            buf.put_u8(index);
        } else {
            // RLE encoding for runs > 1
            buf.put_u8(index | 128); // Set bit 7 to indicate RLE follows
            // Encode run length using variable-length encoding (RFC 6143 Section 7.6.6)
            let mut remaining_len = run_len - 1;
            while remaining_len > 127 {
                buf.put_u8(127 | 128); // 127 with continuation bit = 255
                remaining_len -= 127;
            }
            buf.put_u8(remaining_len as u8);
        }
        i += run_len;
    }
}

/// Encodes pixel data using run-length encoding.
fn encode_rle(pixels: &[u32]) -> Vec<u8> {
    let mut encoded = BytesMut::new();
    let mut i = 0;
    while i < pixels.len() {
        let color = pixels[i];
        let mut run_len = 1;
        while i + run_len < pixels.len() && pixels[i + run_len] == color {
            run_len += 1;
        }
        // Write CPIXEL (3 bytes)
        put_cpixel(&mut encoded, color);

        // Encode run length using variable-length encoding (RFC 6143 Section 7.6.6)
        // Each byte holds 0-127 in lower 7 bits, bit 7 set means more bytes follow
        // The total value is the SUM of all bytes (with bit 7 masked)
        let mut len_to_encode = run_len - 1;
        while len_to_encode > 127 {
            encoded.put_u8(127 | 128); // 127 with continuation bit = 255
            len_to_encode -= 127;
        }
        encoded.put_u8(len_to_encode as u8);

        i += run_len;
    }
    encoded.to_vec()
}

/// Implements the VNC "ZRLE" (Zlib Run-Length Encoding).
pub struct ZrleEncoding;

impl Encoding for ZrleEncoding {
    fn encode(&self, data: &[u8], width: u16, height: u16, _quality: u8, compression: u8) -> BytesMut {
        // ZRLE doesn't use quality, but it does use compression.
        let pixel_format = PixelFormat::rgba32(); // Assuming RGBA32 for now
        match encode_zrle(data, width, height, &pixel_format, compression) {
            Ok(encoded_data) => BytesMut::from(&encoded_data[..]),
            Err(_) => {
                // Fallback to Raw encoding if ZRLE fails.
                let mut buf = BytesMut::with_capacity(data.len());
                for chunk in data.chunks_exact(4) {
                    buf.put_u8(chunk[0]); // R
                    buf.put_u8(chunk[1]); // G
                    buf.put_u8(chunk[2]); // B
                    buf.put_u8(0);        // Padding
                }
                buf
            }
        }
    }
}
