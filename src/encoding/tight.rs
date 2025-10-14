//! VNC Tight encoding implementation.
//!
//! Tight encoding with JPEG, palette, mono rect, and zlib support.
//! Highly efficient for various types of screen content.
//!
//! This implementation follows libvncserver's Tight encoding protocol:
//! - Solid fill (1 color)
//! - Mono rect (2 colors, 1-bit bitmap)
//! - Indexed palette (3-16 colors)
//! - JPEG (photographic content)

use bytes::{BufMut, BytesMut};
use flate2::write::ZlibEncoder;
use flate2::Compression;
use std::io::Write;
use std::collections::HashMap;
use super::Encoding;
use super::common::{rgba_to_rgb24_pixels, check_solid_color, build_palette, put_pixel32};

// Tight encoding protocol constants (from libvncserver rfbproto.h)
const TIGHT_EXPLICIT_FILTER: u8 = 0x04;
const TIGHT_FILL: u8 = 0x08;
const TIGHT_JPEG: u8 = 0x09;

// Filter types
const TIGHT_FILTER_PALETTE: u8 = 0x01;

// Stream IDs for different encoding types
const STREAM_ID_FULL_COLOR: u8 = 0; // Full-color (truecolor)
const STREAM_ID_MONO: u8 = 1;       // Mono rect (2 colors)
const STREAM_ID_INDEXED: u8 = 2;    // Indexed palette (3-16 colors)

// Minimum data size to apply compression (from libvncserver tight.c line 48)
const TIGHT_MIN_TO_COMPRESS: usize = 12;

/// Implements the VNC "Tight" encoding with JPEG, palette, and zlib support.
pub struct TightEncoding;

impl Encoding for TightEncoding {
    fn encode(&self, data: &[u8], width: u16, height: u16, quality: u8, compression: u8) -> BytesMut {
        // Intelligently choose the best encoding method based on image content
        // Following libvncserver's algorithm (tight.c lines 658-730)

        // Method 1: Check if it's a solid color (1 color)
        let pixels = rgba_to_rgb24_pixels(data);
        if let Some(solid_color) = check_solid_color(&pixels) {
            return encode_tight_solid(solid_color);
        }

        // Method 2: Build palette and route to appropriate encoding
        let palette = build_palette(&pixels);

        // Route based on palette size (matching libvncserver's algorithm)
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
        // Following libvncserver's logic: use JPEG for high quality photographic content,
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
/// Wire format: [0x80] [R] [G] [B] [X]
fn encode_tight_solid(color: u32) -> BytesMut {
    let mut buf = BytesMut::with_capacity(5);
    buf.put_u8(TIGHT_FILL << 4); // 0x80: Fill compression (solid color)
    put_pixel32(&mut buf, color); // 4 bytes for 32bpp
    buf
}

/// Encode as Tight indexed palette (3-16 colors).
/// Wire format: [control] [filter] [palette_size-1] [colors...] [length...] [compressed indices]
/// Following libvncserver tight.c lines 900-950
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

    // 4. Palette colors (each color is 4 bytes for 32bpp)
    for &color in palette {
        put_pixel32(&mut buf, color);
    }

    // 5. Compressed data with compact length
    write_compact_length(&mut buf, compressed.len());
    buf.put_slice(&compressed);

    buf
}

/// Encode as Tight mono rect (2 colors, 1-bit bitmap).
/// Wire format: [control] [filter] [1] [bg color] [fg color] [length...] [compressed bitmap]
/// Following libvncserver tight.c lines 794-873
fn encode_tight_mono(pixels: &[u32], width: u16, height: u16, palette: &[u32], compression: u8) -> BytesMut {
    assert_eq!(palette.len(), 2, "Mono rect requires exactly 2 colors");

    let w = width as usize;
    let h = height as usize;

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

    // 4. Palette: background then foreground (each 4 bytes for 32bpp)
    put_pixel32(&mut buf, bg);
    put_pixel32(&mut buf, fg);

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
/// Following libvncserver tight.c EncodeMonoRect32 lines 1465-1517
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
/// Format from libvncserver tight.c lines 1055-1071:
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
    use crate::turbojpeg::TurboJpegEncoder;

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
                    log::error!("TurboJPEG encoding failed: {}, falling back to basic tight encoding", e);
                    // Basic tight encoding requires client pixel format (4 bytes per pixel for 32bpp)
                    let mut buf = BytesMut::with_capacity(1 + data.len());
                    buf.put_u8(0x00); // Basic tight encoding, no compression
                    // Convert RGBA to client pixel format (RGBX)
                    for chunk in data.chunks_exact(4) {
                        buf.put_u8(chunk[0]); // R
                        buf.put_u8(chunk[1]); // G
                        buf.put_u8(chunk[2]); // B
                        buf.put_u8(0);        // Padding
                    }
                    return buf;
                }
            }
        }
        Err(e) => {
            log::error!("Failed to create TurboJPEG encoder: {}, falling back to basic tight encoding", e);
            // Basic tight encoding requires client pixel format (4 bytes per pixel for 32bpp)
            let mut buf = BytesMut::with_capacity(1 + data.len());
            buf.put_u8(0x00); // Basic tight encoding, no compression
            // Convert RGBA to client pixel format (RGBX)
            for chunk in data.chunks_exact(4) {
                buf.put_u8(chunk[0]); // R
                buf.put_u8(chunk[1]); // G
                buf.put_u8(chunk[2]); // B
                buf.put_u8(0);        // Padding
            }
            return buf;
        }
    };

    let mut buf = BytesMut::new();
    buf.put_u8(TIGHT_JPEG << 4); // 0x90: JPEG subencoding

    // Compact length
    write_compact_length(&mut buf, jpeg_data.len());

    buf.put_slice(&jpeg_data);
    buf
}

/// Encode as Tight full-color with zlib compression (lossless).
/// Wire format: [0x00] [length...] [compressed RGB24 data]
/// Following libvncserver tight.c SendFullColorRect (lines 962-992)
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
        // (libvncserver doesn't send length for data < TIGHT_MIN_TO_COMPRESS)
        buf.put_slice(&rgb_data);
    }

    buf
}
