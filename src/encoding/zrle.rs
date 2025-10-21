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
use flate2::{Compress, Compression, FlushCompress};
use std::collections::HashMap;
use std::io::Write;

use super::Encoding;
use crate::protocol::PixelFormat;

const TILE_SIZE: usize = 64;

/// Analyzes pixel data to count RLE runs, single pixels, and unique colors.
/// Returns: (runs, `single_pixels`, `palette_vec`)
/// CRITICAL: The palette Vec must preserve insertion order (order colors first appear)
/// as required by RFC 6143 for proper ZRLE palette encoding.
/// Optimized: uses inline array for small palettes to avoid `HashMap` allocation.
fn analyze_runs_and_palette(pixels: &[u32]) -> (usize, usize, Vec<u32>) {
    let mut runs = 0;
    let mut single_pixels = 0;
    let mut palette: Vec<u32> = Vec::with_capacity(16); // Most tiles have <= 16 colors

    if pixels.is_empty() {
        return (0, 0, palette);
    }

    let mut i = 0;
    while i < pixels.len() {
        let color = pixels[i];

        // For small palettes (common case), linear search is faster than HashMap
        if palette.len() < 256 && !palette.contains(&color) {
            palette.push(color);
        }

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
    (runs, single_pixels, palette)
}

/// Encodes a rectangle of pixel data using ZRLE with a persistent compressor.
/// This maintains compression state across rectangles as required by RFC 6143.
///
/// # Errors
///
/// Returns an error if zlib compression fails
#[allow(dead_code)]
#[allow(clippy::cast_possible_truncation)] // ZRLE protocol requires u8/u16/u32 packing of pixel data
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

    // Compress using persistent compressor with Z_SYNC_FLUSH
    // RFC 6143: use persistent zlib stream with dictionary for compression continuity
    let input = &uncompressed_data[..];
    let mut output_buf = vec![0u8; input.len() * 2 + 1024]; // Generous buffer

    let before_out = compressor.total_out();

    // Single compress call with Z_SYNC_FLUSH - this should handle all input
    compressor.compress(input, &mut output_buf, FlushCompress::Sync)?;

    let produced = (compressor.total_out() - before_out) as usize;
    let compressed_output = &output_buf[..produced];

    // Build result with length prefix (big-endian) + compressed data
    let mut result = BytesMut::with_capacity(4 + compressed_output.len());
    result.put_u32(compressed_output.len() as u32);
    result.extend_from_slice(compressed_output);

    log::info!(
        "ZRLE: compressed {}->{}  bytes ({}x{} tiles)",
        uncompressed_data.len(),
        compressed_output.len(),
        width,
        height
    );

    Ok(result.to_vec())
}

/// Encodes a rectangle of pixel data using the ZRLE encoding.
/// This creates a new compressor for each rectangle (non-RFC compliant, deprecated).
///
/// # Errors
///
/// Returns an error if zlib compression fails
#[allow(clippy::cast_possible_truncation)] // ZRLE protocol requires u8/u16/u32 packing of pixel data
pub fn encode_zrle(
    data: &[u8],
    width: u16,
    height: u16,
    _pixel_format: &PixelFormat, // Assuming RGBA32
    compression: u8,
) -> std::io::Result<Vec<u8>> {
    let compression_level = match compression {
        0 => Compression::fast(),
        1..=3 => Compression::new(u32::from(compression)),
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
/// Optimized to minimize allocations by working directly with RGBA data where possible.
fn encode_tile(buf: &mut BytesMut, tile_data: &[u8], width: usize, height: usize) {
    // Quick check for solid color by scanning RGBA data directly (avoid allocation)
    if tile_data.len() >= 4 {
        let first_r = tile_data[0];
        let first_g = tile_data[1];
        let first_b = tile_data[2];
        let mut is_solid = true;

        for chunk in tile_data.chunks_exact(4).skip(1) {
            if chunk[0] != first_r || chunk[1] != first_g || chunk[2] != first_b {
                is_solid = false;
                break;
            }
        }

        if is_solid {
            let color = u32::from(first_r) | (u32::from(first_g) << 8) | (u32::from(first_b) << 16);
            encode_solid_color_tile(buf, color);
            return;
        }
    }

    // Convert RGBA to RGB24 pixels (still needed for analysis)
    let pixels = rgba_to_rgb24_pixels(tile_data);
    let (runs, single_pixels, palette) = analyze_runs_and_palette(&pixels);

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

    if palette.len() < 128 {
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
            let packed_bytes =
                CPIXEL_SIZE * palette_size + (width * height * bits_per_packed_pixel).div_ceil(8);

            if packed_bytes < estimated_bytes {
                use_rle = false;
                use_palette = true;
                // No need to update estimated_bytes, this is the last check
            }
        }
    }

    if use_palette {
        // Palette (Packed Palette or Packed Palette RLE)
        // Build index lookup from palette (preserves insertion order)
        let color_to_idx: HashMap<_, _> = palette
            .iter()
            .enumerate()
            .map(|(i, &c)| (c, i as u8))
            .collect();

        if use_rle {
            // Packed Palette RLE
            encode_packed_palette_rle_tile(buf, &pixels, &palette, &color_to_idx);
        } else {
            // Packed Palette (no RLE)
            encode_packed_palette_tile(buf, &pixels, width, height, &palette, &color_to_idx);
        }
    } else {
        // Raw or Plain RLE
        if use_rle {
            // Plain RLE - encode directly to buffer (avoid intermediate Vec)
            buf.put_u8(128);
            encode_rle_to_buf(buf, &pixels);
        } else {
            // Raw
            encode_raw_tile(buf, tile_data);
        }
    }
}

/// Extracts a tile from the full framebuffer.
/// Optimized to use a single allocation and bulk copy operations.
fn extract_tile(
    full_frame: &[u8],
    frame_width: usize,
    x: usize,
    y: usize,
    width: usize,
    height: usize,
) -> Vec<u8> {
    let tile_size = width * height * 4;
    let mut tile_data = Vec::with_capacity(tile_size);

    // Use unsafe for performance - we know the capacity is correct
    unsafe {
        tile_data.set_len(tile_size);
    }

    let row_bytes = width * 4;
    for row in 0..height {
        let src_start = ((y + row) * frame_width + x) * 4;
        let dst_start = row * row_bytes;
        tile_data[dst_start..dst_start + row_bytes]
            .copy_from_slice(&full_frame[src_start..src_start + row_bytes]);
    }
    tile_data
}

/// Converts RGBA to 32-bit RGB pixels (0x00BBGGRR format for VNC).
fn rgba_to_rgb24_pixels(data: &[u8]) -> Vec<u32> {
    data.chunks_exact(4)
        .map(|c| u32::from(c[0]) | (u32::from(c[1]) << 8) | (u32::from(c[2]) << 16))
        .collect()
}

/// Writes a CPIXEL (3 bytes for depth=24) in little-endian format.
/// CPIXEL format: R at byte 0, G at byte 1, B at byte 2
fn put_cpixel(buf: &mut BytesMut, pixel: u32) {
    buf.put_u8((pixel & 0xFF) as u8); // R at bits 0-7
    buf.put_u8(((pixel >> 8) & 0xFF) as u8); // G at bits 8-15
    buf.put_u8(((pixel >> 16) & 0xFF) as u8); // B at bits 16-23
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
    width: usize,
    height: usize,
    palette: &[u32],
    color_to_idx: &HashMap<u32, u8>,
) {
    let palette_size = palette.len();
    let bits_per_pixel = match palette_size {
        2 => 1,
        3..=4 => 2,
        _ => 4,
    };

    buf.put_u8(palette_size as u8); // Packed palette sub-encoding

    // Write palette as CPIXEL (3 bytes each) - in insertion order
    for &color in palette {
        put_cpixel(buf, color);
    }

    // Write packed pixel data ROW BY ROW per RFC 6143 ZRLE specification
    // Critical: Each row must be byte-aligned
    for row in 0..height {
        let mut packed_byte = 0;
        let mut nbits = 0;
        let row_start = row * width;
        let row_end = row_start + width;

        for &pixel in &pixels[row_start..row_end] {
            let idx = color_to_idx[&pixel];
            // Pack from MSB: byte = (byte << bppp) | index
            packed_byte = (packed_byte << bits_per_pixel) | idx;
            nbits += bits_per_pixel;

            if nbits >= 8 {
                buf.put_u8(packed_byte);
                packed_byte = 0;
                nbits = 0;
            }
        }

        // Pad remaining bits to MSB at end of row per RFC 6143
        if nbits > 0 {
            packed_byte <<= 8 - nbits;
            buf.put_u8(packed_byte);
        }
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

    // Write RLE data using palette indices per RFC 6143 specification
    let mut i = 0;
    while i < pixels.len() {
        let color = pixels[i];
        let index = color_to_idx[&color];

        let mut run_len = 1;
        while i + run_len < pixels.len() && pixels[i + run_len] == color {
            run_len += 1;
        }

        // Short runs (1-2 pixels) are written WITHOUT RLE marker per RFC 6143
        if run_len <= 2 {
            // Write index once for length 1, twice for length 2
            if run_len == 2 {
                buf.put_u8(index);
            }
            buf.put_u8(index);
        } else {
            // RLE encoding for runs >= 3 per RFC 6143
            buf.put_u8(index | 128); // Set bit 7 to indicate RLE follows
                                     // Encode run length - 1 using variable-length encoding
            let mut remaining_len = run_len - 1;
            while remaining_len >= 255 {
                buf.put_u8(255);
                remaining_len -= 255;
            }
            buf.put_u8(remaining_len as u8);
        }
        i += run_len;
    }
}

/// Encodes pixel data using run-length encoding directly to buffer (optimized).
fn encode_rle_to_buf(buf: &mut BytesMut, pixels: &[u32]) {
    let mut i = 0;
    while i < pixels.len() {
        let color = pixels[i];
        let mut run_len = 1;
        while i + run_len < pixels.len() && pixels[i + run_len] == color {
            run_len += 1;
        }
        // Write CPIXEL (3 bytes)
        put_cpixel(buf, color);

        // Encode run length - 1 per RFC 6143 ZRLE specification
        // Length encoding: write 255 for each full 255-length chunk, then remainder
        // NO continuation bits - just plain bytes where 255 means "add 255 to length"
        let mut len_to_encode = run_len - 1;
        while len_to_encode >= 255 {
            buf.put_u8(255);
            len_to_encode -= 255;
        }
        buf.put_u8(len_to_encode as u8);

        i += run_len;
    }
}

/// Implements the VNC "ZRLE" (Zlib Run-Length Encoding).
pub struct ZrleEncoding;

impl Encoding for ZrleEncoding {
    fn encode(
        &self,
        data: &[u8],
        width: u16,
        height: u16,
        _quality: u8,
        compression: u8,
    ) -> BytesMut {
        // ZRLE doesn't use quality, but it does use compression.
        let pixel_format = PixelFormat::rgba32(); // Assuming RGBA32 for now
        if let Ok(encoded_data) = encode_zrle(data, width, height, &pixel_format, compression) { BytesMut::from(&encoded_data[..]) } else {
            // Fallback to Raw encoding if ZRLE fails.
            let mut buf = BytesMut::with_capacity(data.len());
            for chunk in data.chunks_exact(4) {
                buf.put_u8(chunk[0]); // R
                buf.put_u8(chunk[1]); // G
                buf.put_u8(chunk[2]); // B
                buf.put_u8(0); // Padding
            }
            buf
        }
    }
}
