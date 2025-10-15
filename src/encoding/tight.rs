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


//! VNC Tight encoding implementation.
//!
//! Tight encoding with JPEG, palette, mono rect, and zlib support.
//! Highly efficient for various types of screen content.
//!
//! This implementation follows standard VNC protocol's Tight encoding protocol:
//! - Solid fill (1 color)
//! - Mono rect (2 colors, 1-bit bitmap)
//! - Indexed palette (3-16 colors)
//! - JPEG (photographic content)
//!
//! For persistent zlib streams (matching standard VNC protocol), use the `encode_tight_persistent`
//! function which maintains compression dictionaries across multiple rectangles.

use bytes::{BufMut, BytesMut};
use flate2::write::ZlibEncoder;
use flate2::Compression;
use std::io::Write;
use std::collections::HashMap;
use super::Encoding;
use super::common::{rgba_to_rgb24_pixels, check_solid_color, build_palette, put_pixel24};

// Tight encoding protocol constants (from standard VNC protocol rfbproto.h)
const TIGHT_EXPLICIT_FILTER: u8 = 0x04;
const TIGHT_FILL: u8 = 0x08;
#[allow(dead_code)]
const TIGHT_JPEG: u8 = 0x09;

// Filter types
const TIGHT_FILTER_PALETTE: u8 = 0x01;

// Stream IDs for different encoding types (public for use by client.rs)
/// Stream ID for full-color (truecolor) Tight encoding (matches standard VNC protocol stream 0).
pub const STREAM_ID_FULL_COLOR: u8 = 0;
/// Stream ID for mono rect (2-color) Tight encoding (matches standard VNC protocol stream 1).
pub const STREAM_ID_MONO: u8 = 1;
/// Stream ID for indexed palette (3-16 colors) Tight encoding (matches standard VNC protocol stream 2).
pub const STREAM_ID_INDEXED: u8 = 2;

// Minimum data size to apply compression (from standard VNC protocol tight.c line 48)
const TIGHT_MIN_TO_COMPRESS: usize = 12;

/// Trait for managing persistent zlib compression streams.
/// This allows tight.rs to use the client's stream manager without
/// importing client.rs (avoiding circular dependency).
pub trait TightStreamCompressor {
    /// Compress data using the specified stream ID and compression level.
    /// Maintains dictionary state across compressions (Z_SYNC_FLUSH).
    fn compress_tight_stream(&mut self, stream_id: u8, level: u8, input: &[u8]) -> Result<Vec<u8>, String>;
}

/// Implements the VNC "Tight" encoding with JPEG, palette, and zlib support.
pub struct TightEncoding;

impl Encoding for TightEncoding {
    fn encode(&self, data: &[u8], width: u16, height: u16, quality: u8, compression: u8) -> BytesMut {
        // Intelligently choose the best encoding method based on image content
        // Following standard VNC protocol's algorithm (tight.c lines 658-730)

        // Method 1: Check if it's a solid color (1 color)
        let pixels = rgba_to_rgb24_pixels(data);
        if let Some(solid_color) = check_solid_color(&pixels) {
            return encode_tight_solid(solid_color);
        }

        // Method 2: Build palette and route to appropriate encoding
        let palette = build_palette(&pixels);

        // Route based on palette size (matching standard VNC protocol's algorithm)
        match palette.len() {
            // 2 colors: Use mono rect encoding (1-bit bitmap)
            2 => {
                return encode_tight_mono(&pixels, width, height, &palette, compression);
            }
            // 3-16 colors: Use indexed palette encoding
            3..=16 if palette.len() < pixels.len() / 4 => {
                return encode_tight_indexed(&pixels, width, height, &palette, compression);
            }
            // Too many colors or not worth palette encoding
            _ => {}
        }

        // Method 3: Choose between full-color zlib and JPEG based on quality setting
        // Following standard VNC protocol's logic: use JPEG for high quality photographic content,
        // use full-color zlib for lossless compression or lower quality settings
        if quality == 0 || quality >= 10 {
            // Quality 0 = lossless preference, quality >= 10 = disable JPEG
            // Use full-color zlib mode
            encode_tight_full_color(data, width, height, compression)
        } else {
            // Quality 1-9: Use JPEG for photographic content (powered by libjpeg-turbo)
            encode_tight_jpeg(data, width, height, quality)
        }
    }
}

/// Encode as Tight solid fill (1 color).
/// Wire format: [0x80] [R] [G] [B]
/// Following libvncserver tight.c SendSolidRect (lines 768-791)
/// NOTE: Uses Pack24 format (RGB24, 3 bytes) to match libvncserver's tightUsePixelFormat24 behavior
fn encode_tight_solid(color: u32) -> BytesMut {
    let mut buf = BytesMut::with_capacity(4);
    buf.put_u8(TIGHT_FILL << 4); // 0x80: Fill compression (solid color)
    // Pack as RGB24 (3 bytes) - matches libvncserver's Pack24() when depth==24
    put_pixel24(&mut buf, color);
    log::info!("Tight: solid fill, color=0x{:08x}, output {} bytes", color, buf.len());
    buf
}

/// Encode as Tight indexed palette (3-16 colors).
/// Wire format: [control] [filter] [palette_size-1] [colors...] [length...] [compressed indices]
/// Following standard VNC protocol tight.c lines 900-950
fn encode_tight_indexed(pixels: &[u32], _width: u16, _height: u16, palette: &[u32], compression: u8) -> BytesMut {
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
        // Compression failed, fall back to JPEG encoding
        // Convert u32 pixels back to RGBA for JPEG encoding
        return encode_tight_jpeg(
            &pixels.iter().flat_map(|&p| {
                vec![(p & 0xFF) as u8, ((p >> 8) & 0xFF) as u8, ((p >> 16) & 0xFF) as u8, 0xFF]
            }).collect::<Vec<u8>>(),
            _width, _height, 75
        );
    }
    let compressed = match encoder.finish() {
        Ok(data) => data,
        Err(_) => {
            // Compression failed, fall back to JPEG encoding
            // Convert u32 pixels back to RGBA for JPEG encoding
            return encode_tight_jpeg(
                &pixels.iter().flat_map(|&p| {
                    vec![(p & 0xFF) as u8, ((p >> 8) & 0xFF) as u8, ((p >> 16) & 0xFF) as u8, 0xFF]
                }).collect::<Vec<u8>>(),
                _width, _height, 75
            );
        }
    };

    let mut buf = BytesMut::new();

    // 1. Control byte: (stream_id | TIGHT_EXPLICIT_FILTER) << 4
    // Stream 2 for indexed palette, with explicit filter
    buf.put_u8((STREAM_ID_INDEXED | TIGHT_EXPLICIT_FILTER) << 4); // 0x60

    // 2. Filter ID
    buf.put_u8(TIGHT_FILTER_PALETTE); // 0x01

    // 3. Palette size minus 1 (0 = 1 color is invalid, but handled as solid fill)
    buf.put_u8((palette_size - 1) as u8);

    // 4. Palette colors (RGB24, 3 bytes each)
    // Following libvncserver SendIndexedRect with Pack24 for depth==24
    for &color in palette {
        put_pixel24(&mut buf, color);
    }

    // 5. Compressed data with compact length
    write_compact_length(&mut buf, compressed.len());
    buf.put_slice(&compressed);

    buf
}

/// Encode as Tight mono rect (2 colors, 1-bit bitmap).
/// Wire format: [control] [filter] [1] [bg color] [fg color] [length...] [compressed bitmap]
/// Following standard VNC protocol tight.c lines 794-873
fn encode_tight_mono(pixels: &[u32], width: u16, height: u16, palette: &[u32], compression: u8) -> BytesMut {
    assert_eq!(palette.len(), 2, "Mono rect requires exactly 2 colors");

    // Determine background (most common) and foreground colors
    let (bg, fg) = determine_bg_fg(pixels, palette);

    // Encode as 1-bit bitmap: 0 = background, 1 = foreground
    // MSB first, each row byte-aligned
    let bitmap = encode_mono_bitmap(pixels, width, height, bg);

    // Compress bitmap
    let compression_level = match compression {
        0 => Compression::fast(),
        1..=3 => Compression::new(compression as u32),
        4..=6 => Compression::default(),
        _ => Compression::best(),
    };

    let compressed = if bitmap.len() >= TIGHT_MIN_TO_COMPRESS {
        let mut encoder = ZlibEncoder::new(Vec::new(), compression_level);
        match encoder.write_all(&bitmap).and_then(|_| encoder.finish()) {
            Ok(data) => Some(data),
            Err(_) => None, // Use uncompressed on error
        }
    } else {
        None // Too small to compress
    };

    let mut buf = BytesMut::new();

    // 1. Control byte: (stream_id | TIGHT_EXPLICIT_FILTER) << 4
    // Stream 1 for mono rect, with explicit filter
    buf.put_u8((STREAM_ID_MONO | TIGHT_EXPLICIT_FILTER) << 4); // 0x50

    // 2. Filter ID
    buf.put_u8(TIGHT_FILTER_PALETTE); // 0x01

    // 3. Palette size minus 1 (1 = 2 colors)
    buf.put_u8(1);

    // 4. Palette: background then foreground (RGB24, 3 bytes each)
    // Following libvncserver SendMonoRect with Pack24 for depth==24
    put_pixel24(&mut buf, bg);
    put_pixel24(&mut buf, fg);

    // 5. Bitmap data (compressed or uncompressed)
    if let Some(comp_data) = compressed {
        write_compact_length(&mut buf, comp_data.len());
        buf.put_slice(&comp_data);
    } else {
        // Send uncompressed (no length header for data < TIGHT_MIN_TO_COMPRESS)
        if bitmap.len() >= TIGHT_MIN_TO_COMPRESS {
            write_compact_length(&mut buf, bitmap.len());
        }
        buf.put_slice(&bitmap);
    }

    buf
}

/// Determine background (most common) and foreground colors from a 2-color palette.
fn determine_bg_fg(pixels: &[u32], palette: &[u32]) -> (u32, u32) {
    let c0 = palette[0];
    let c1 = palette[1];
    let count0 = pixels.iter().filter(|&&p| p == c0).count();
    let count1 = pixels.len() - count0;

    if count0 > count1 {
        (c0, c1) // c0 is background (more common)
    } else {
        (c1, c0) // c1 is background
    }
}

/// Encode pixels as a 1-bit bitmap for mono rect encoding.
/// Returns bitmap where 0 = background, 1 = foreground
/// MSB first, each row byte-aligned
/// Following standard VNC protocol tight.c EncodeMonoRect32 lines 1465-1517
fn encode_mono_bitmap(pixels: &[u32], width: u16, height: u16, bg: u32) -> Vec<u8> {
    let w = width as usize;
    let h = height as usize;
    let bytes_per_row = (w + 7) / 8;
    let mut bitmap = vec![0u8; bytes_per_row * h];

    let mut bitmap_idx = 0;
    for y in 0..h {
        let mut byte_val = 0u8;
        let mut bit_pos = 7i32; // MSB first: 7, 6, 5, 4, 3, 2, 1, 0

        for x in 0..w {
            if pixels[y * w + x] != bg {
                byte_val |= 1 << bit_pos; // Set bit for foreground
            }

            if bit_pos == 0 {
                // Byte complete
                bitmap[bitmap_idx] = byte_val;
                bitmap_idx += 1;
                byte_val = 0;
                bit_pos = 7;
            } else {
                bit_pos -= 1;
            }
        }

        // Write partial byte at end of row (if width not multiple of 8)
        if w % 8 != 0 {
            bitmap[bitmap_idx] = byte_val;
            bitmap_idx += 1;
        }
    }

    bitmap
}

/// Write compact length encoding (1-3 bytes).
/// Format from standard VNC protocol tight.c lines 1055-1071:
/// - 0-127: 1 byte (0xxxxxxx)
/// - 128-16383: 2 bytes (1xxxxxxx 0yyyyyyy)
/// - 16384-4194303: 3 bytes (1xxxxxxx 1yyyyyyy zzzzzzzz)
fn write_compact_length(buf: &mut BytesMut, len: usize) {
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
}

/// Encode as Tight JPEG using libjpeg-turbo.
fn encode_tight_jpeg(data: &[u8], width: u16, height: u16, quality: u8) -> BytesMut {
    #[cfg(feature = "turbojpeg")]
    {
        use crate::jpeg::TurboJpegEncoder;

        // Convert RGBA to RGB
        let mut rgb_data = Vec::with_capacity((width as usize) * (height as usize) * 3);
        for chunk in data.chunks_exact(4) {
            rgb_data.push(chunk[0]);
            rgb_data.push(chunk[1]);
            rgb_data.push(chunk[2]);
        }

        // Compress with TurboJPEG (libjpeg-turbo)
        let jpeg_data = match TurboJpegEncoder::new() {
            Ok(mut encoder) => {
                match encoder.compress_rgb(&rgb_data, width, height, quality) {
                    Ok(data) => data,
                    Err(e) => {
                        log::error!("TurboJPEG encoding failed: {}, falling back to full-color zlib", e);
                        return encode_tight_full_color(data, width, height, 6);
                    }
                }
            }
            Err(e) => {
                log::error!("Failed to create TurboJPEG encoder: {}, falling back to full-color zlib", e);
                return encode_tight_full_color(data, width, height, 6);
            }
        };

        let mut buf = BytesMut::new();
        buf.put_u8(TIGHT_JPEG << 4); // 0x90: JPEG subencoding

        // Compact length
        write_compact_length(&mut buf, jpeg_data.len());

        buf.put_slice(&jpeg_data);
        buf
    }

    #[cfg(not(feature = "turbojpeg"))]
    {
        // TurboJPEG not available, fall back to full-color zlib
        log::warn!("TurboJPEG not enabled, using full-color zlib instead (quality={})", quality);
        encode_tight_full_color(data, width, height, 6)
    }
}

/// Encode as Tight full-color with zlib compression (lossless).
/// Wire format: [0x00] [length...] [compressed RGB24 data]
/// Following standard VNC protocol tight.c SendFullColorRect (lines 962-992)
fn encode_tight_full_color(data: &[u8], width: u16, height: u16, compression: u8) -> BytesMut {
    // Convert RGBA32 to RGB24 (3 bytes per pixel) for better compression
    let mut rgb_data = Vec::with_capacity((width as usize) * (height as usize) * 3);
    for chunk in data.chunks_exact(4) {
        rgb_data.push(chunk[0]); // R
        rgb_data.push(chunk[1]); // G
        rgb_data.push(chunk[2]); // B
    }

    // Determine zlib compression level based on VNC compression setting
    let compression_level = match compression {
        0 => Compression::fast(),
        1..=3 => Compression::new(compression as u32),
        4..=6 => Compression::default(),
        _ => Compression::best(),
    };

    let mut buf = BytesMut::new();

    // Control byte: stream 0, no filter, basic compression
    buf.put_u8(STREAM_ID_FULL_COLOR << 4); // 0x00

    // Compress data if >= TIGHT_MIN_TO_COMPRESS, otherwise send uncompressed
    if rgb_data.len() >= TIGHT_MIN_TO_COMPRESS {
        let mut encoder = ZlibEncoder::new(Vec::new(), compression_level);
        match encoder.write_all(&rgb_data).and_then(|_| encoder.finish()) {
            Ok(compressed) => {
                // Send compressed data
                write_compact_length(&mut buf, compressed.len());
                buf.put_slice(&compressed);
            }
            Err(e) => {
                // Compression failed, send uncompressed
                log::warn!("Zlib compression failed: {}, sending uncompressed", e);
                write_compact_length(&mut buf, rgb_data.len());
                buf.put_slice(&rgb_data);
            }
        }
    } else {
        // Data too small to compress, send uncompressed without length header
        // (standard VNC protocol doesn't send length for data < TIGHT_MIN_TO_COMPRESS)
        buf.put_slice(&rgb_data);
    }

    buf
}

/// Encode Tight with persistent zlib streams (standard VNC protocol style).
///
/// This function matches standard VNC protocol's behavior by using persistent compression
/// streams that maintain dictionary state across multiple compressions.
///
/// # Arguments
/// * `data` - RGBA pixel data
/// * `width` - Image width
/// * `height` - Image height
/// * `quality` - JPEG quality (0-100)
/// * `compression` - Zlib compression level (0-9)
/// * `compressor` - Persistent stream compressor implementing TightStreamCompressor
///
/// # Returns
/// Encoded data in Tight format
pub fn encode_tight_with_streams<C: TightStreamCompressor>(
    data: &[u8],
    width: u16,
    height: u16,
    quality: u8,
    compression: u8,
    compressor: &mut C,
) -> BytesMut {
    // Intelligently choose the best encoding method based on image content
    // Following standard VNC protocol's algorithm (tight.c lines 658-730)

    // Method 1: Check if it's a solid color (1 color)
    let pixels = rgba_to_rgb24_pixels(data);
    if let Some(solid_color) = check_solid_color(&pixels) {
        return encode_tight_solid(solid_color);
    }

    // Method 2: Build palette and route to appropriate encoding
    let palette = build_palette(&pixels);

    // Route based on palette size (matching standard VNC protocol's algorithm)
    match palette.len() {
        // 2 colors: Use mono rect encoding (1-bit bitmap)
        2 => {
            return encode_tight_mono_persistent(&pixels, width, height, &palette, compression, compressor);
        }
        // 3-16 colors: Use indexed palette encoding
        3..=16 if palette.len() < pixels.len() / 4 => {
            return encode_tight_indexed_persistent(&pixels, width, height, &palette, compression, compressor);
        }
        // Too many colors or not worth palette encoding
        _ => {}
    }

    // Method 3: Choose between JPEG and full-color zlib based on quality level
    // Following libvncserver's logic (tight.c lines 705-715):
    // - if turboQualityLevel == 255 (unset): use JPEG with default quality
    // - if turboQualityLevel != 255: use JPEG with mapped quality
    // Note: Full-color zlib is used when quality level >= 10 (disable JPEG)
    if quality == 255 {
        // Unset quality level: use JPEG with default quality (80)
        log::info!("Tight: quality=255 (unset), using JPEG with quality 80");
        encode_tight_jpeg(data, width, height, 80)
    } else if quality >= 10 {
        // Quality level >= 10: use full-color zlib (lossless), JPEG disabled
        log::info!("Tight: quality={} (>=10), using full-color zlib", quality);
        encode_tight_full_color_persistent(data, width, height, compression, compressor)
    } else {
        // Quality level 0-9: use JPEG with mapped quality
        // Use libvncserver's quality mapping (TigerVNC compatible)
        // Reference: libvncserver/src/libvncserver/rfbserver.c:109
        const TIGHT2TURBO_QUAL: [u8; 10] = [15, 29, 41, 42, 62, 77, 79, 86, 92, 100];
        let jpeg_quality = TIGHT2TURBO_QUAL[quality as usize];
        log::info!("Tight: quality={} (0-9), using JPEG with mapped quality {}", quality, jpeg_quality);
        encode_tight_jpeg(data, width, height, jpeg_quality)
    }
}

/// Encode as Tight indexed palette with persistent stream.
fn encode_tight_indexed_persistent<C: TightStreamCompressor>(
    pixels: &[u32],
    _width: u16,
    _height: u16,
    palette: &[u32],
    compression: u8,
    compressor: &mut C,
) -> BytesMut {
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

    // Compress indices using persistent stream (stream ID 2 for indexed)
    let compressed = match compressor.compress_tight_stream(STREAM_ID_INDEXED, compression, &indices) {
        Ok(data) => data,
        Err(e) => {
            log::error!("Tight indexed persistent compression failed: {}, falling back to JPEG", e);
            // Convert u32 pixels back to RGBA for JPEG encoding
            return encode_tight_jpeg(
                &pixels.iter().flat_map(|&p| {
                    vec![(p & 0xFF) as u8, ((p >> 8) & 0xFF) as u8, ((p >> 16) & 0xFF) as u8, 0xFF]
                }).collect::<Vec<u8>>(),
                _width, _height, 75
            );
        }
    };

    let mut buf = BytesMut::new();

    // 1. Control byte: (stream_id | TIGHT_EXPLICIT_FILTER) << 4
    // Stream 2 for indexed palette, with explicit filter
    buf.put_u8((STREAM_ID_INDEXED | TIGHT_EXPLICIT_FILTER) << 4); // 0x60

    // 2. Filter ID
    buf.put_u8(TIGHT_FILTER_PALETTE); // 0x01

    // 3. Palette size minus 1
    buf.put_u8((palette_size - 1) as u8);

    // 4. Palette colors (RGB24, 3 bytes each)
    // Following libvncserver SendIndexedRect with Pack24 for depth==24
    for &color in palette {
        put_pixel24(&mut buf, color);
    }

    // 5. Compressed data with compact length
    write_compact_length(&mut buf, compressed.len());
    buf.put_slice(&compressed);

    buf
}

/// Encode as Tight mono rect with persistent stream.
fn encode_tight_mono_persistent<C: TightStreamCompressor>(
    pixels: &[u32],
    width: u16,
    height: u16,
    palette: &[u32],
    compression: u8,
    compressor: &mut C,
) -> BytesMut {
    assert_eq!(palette.len(), 2, "Mono rect requires exactly 2 colors");

    // Determine background (most common) and foreground colors
    let (bg, fg) = determine_bg_fg(pixels, palette);

    // Encode as 1-bit bitmap: 0 = background, 1 = foreground
    let bitmap = encode_mono_bitmap(pixels, width, height, bg);

    // Compress bitmap using persistent stream (stream ID 1 for mono)
    let compressed = if bitmap.len() >= TIGHT_MIN_TO_COMPRESS {
        match compressor.compress_tight_stream(STREAM_ID_MONO, compression, &bitmap) {
            Ok(data) => Some(data),
            Err(_) => None, // Use uncompressed on error
        }
    } else {
        None // Too small to compress
    };

    let mut buf = BytesMut::new();

    // 1. Control byte: (stream_id | TIGHT_EXPLICIT_FILTER) << 4
    // Stream 1 for mono rect, with explicit filter
    buf.put_u8((STREAM_ID_MONO | TIGHT_EXPLICIT_FILTER) << 4); // 0x50

    // 2. Filter ID
    buf.put_u8(TIGHT_FILTER_PALETTE); // 0x01

    // 3. Palette size minus 1 (1 = 2 colors)
    buf.put_u8(1);

    // 4. Palette: background then foreground (RGB24, 3 bytes each)
    // Following libvncserver SendMonoRect with Pack24 for depth==24
    put_pixel24(&mut buf, bg);
    put_pixel24(&mut buf, fg);

    // 5. Bitmap data (compressed or uncompressed)
    if let Some(comp_data) = compressed {
        write_compact_length(&mut buf, comp_data.len());
        buf.put_slice(&comp_data);
    } else {
        // Send uncompressed (no length header for data < TIGHT_MIN_TO_COMPRESS)
        if bitmap.len() >= TIGHT_MIN_TO_COMPRESS {
            write_compact_length(&mut buf, bitmap.len());
        }
        buf.put_slice(&bitmap);
    }

    buf
}

/// Encode as Tight full-color with persistent stream (lossless).
fn encode_tight_full_color_persistent<C: TightStreamCompressor>(
    data: &[u8],
    width: u16,
    height: u16,
    compression: u8,
    compressor: &mut C,
) -> BytesMut {
    // Convert RGBA32 to RGB24 (3 bytes per pixel) for better compression
    let mut rgb_data = Vec::with_capacity((width as usize) * (height as usize) * 3);
    for chunk in data.chunks_exact(4) {
        rgb_data.push(chunk[0]); // R
        rgb_data.push(chunk[1]); // G
        rgb_data.push(chunk[2]); // B
    }

    let mut buf = BytesMut::new();

    // Control byte: stream 0, no filter, basic compression
    buf.put_u8(STREAM_ID_FULL_COLOR << 4); // 0x00

    // Compress data if >= TIGHT_MIN_TO_COMPRESS, otherwise send uncompressed
    if rgb_data.len() >= TIGHT_MIN_TO_COMPRESS {
        match compressor.compress_tight_stream(STREAM_ID_FULL_COLOR, compression, &rgb_data) {
            Ok(compressed) => {
                // Send compressed data
                write_compact_length(&mut buf, compressed.len());
                buf.put_slice(&compressed);
            }
            Err(e) => {
                // Compression failed, send uncompressed
                log::warn!("Tight full-color persistent compression failed: {}, sending uncompressed", e);
                write_compact_length(&mut buf, rgb_data.len());
                buf.put_slice(&rgb_data);
            }
        }
    } else {
        // Data too small to compress, send uncompressed without length header
        buf.put_slice(&rgb_data);
    }

    buf
}
