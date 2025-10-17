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


//! VNC Tight encoding implementation - RFC 6143 compliant with full optimization
//!
//! # Architecture
//!
//! This implementation has TWO layers for optimal compression:
//!
//! ## Layer 1: High-Level Optimization
//! - Rectangle splitting and subdivision
//! - Solid area detection and extraction
//! - Recursive optimization for best encoding
//! - Size limit enforcement (TIGHT_MAX_RECT_SIZE, TIGHT_MAX_RECT_WIDTH)
//!
//! ## Layer 2: Low-Level Encoding
//! - Palette analysis
//! - Encoding mode selection (solid/mono/indexed/full-color/JPEG)
//! - Compression and wire format generation
//!
//! # Protocol Overview
//!
//! Tight encoding supports 5 compression modes:
//!
//! 1. **Solid fill** (1 color) - control byte 0x80
//!    - Wire format: `[0x80][R][G][B]` (4 bytes total)
//!    - Most efficient for solid color rectangles
//!
//! 2. **Mono rect** (2 colors) - control byte 0x50 or 0xA0
//!    - Wire format: `[control][0x01][1][bg RGB24][fg RGB24][length][bitmap]`
//!    - Uses 1-bit bitmap: 0=background, 1=foreground
//!    - MSB first, each row byte-aligned
//!
//! 3. **Indexed palette** (3-16 colors) - control byte 0x60 or 0xA0
//!    - Wire format: `[control][0x01][n-1][colors...][length][indices]`
//!    - Each pixel encoded as palette index (1 byte)
//!
//! 4. **Full-color zlib** - control byte 0x00 or 0xA0
//!    - Wire format: `[control][length][zlib compressed RGB24]`
//!    - Lossless compression for truecolor images
//!
//! 5. **JPEG** - control byte 0x90
//!    - Wire format: `[0x90][length][JPEG data]`
//!    - Lossy compression for photographic content
//!
//! # Configuration Constants
//!
//! ```text
//! TIGHT_MIN_TO_COMPRESS = 12      (data < 12 bytes sent raw)
//! MIN_SPLIT_RECT_SIZE = 4096      (split rectangles >= 4096 pixels)
//! MIN_SOLID_SUBRECT_SIZE = 2048   (solid areas must be >= 2048 pixels)
//! MAX_SPLIT_TILE_SIZE = 16        (tile size for solid detection)
//! TIGHT_MAX_RECT_SIZE = 65536     (max pixels per rectangle)
//! TIGHT_MAX_RECT_WIDTH = 2048     (max rectangle width)
//! ```

use bytes::{BufMut, BytesMut};
use flate2::write::ZlibEncoder;
use flate2::Compression;
use std::io::Write;
use std::collections::HashMap;
use super::Encoding;
use super::common::put_pixel24;
use log::info;

// Tight encoding protocol constants (RFC 6143 section 7.7.4)
const TIGHT_EXPLICIT_FILTER: u8 = 0x04;
const TIGHT_FILL: u8 = 0x08;
#[allow(dead_code)]
const TIGHT_JPEG: u8 = 0x09;
const TIGHT_NO_ZLIB: u8 = 0x0A;

// Filter types
const TIGHT_FILTER_PALETTE: u8 = 0x01;

/// Zlib stream ID for full-color data (RFC 6143 section 7.7.4)
pub const STREAM_ID_FULL_COLOR: u8 = 0;
/// Zlib stream ID for monochrome bitmap data (RFC 6143 section 7.7.4)
pub const STREAM_ID_MONO: u8 = 1;
/// Zlib stream ID for indexed palette data (RFC 6143 section 7.7.4)
pub const STREAM_ID_INDEXED: u8 = 2;

// Compression thresholds for Tight encoding optimization
const TIGHT_MIN_TO_COMPRESS: usize = 12;
const MIN_SPLIT_RECT_SIZE: usize = 4096;
const MIN_SOLID_SUBRECT_SIZE: usize = 2048;
const MAX_SPLIT_TILE_SIZE: u16 = 16;
const TIGHT_MAX_RECT_SIZE: usize = 65536;
const TIGHT_MAX_RECT_WIDTH: u16 = 2048;

/// Compression configuration for different quality levels
struct TightConf {
    mono_min_rect_size: usize,
    idx_zlib_level: u8,
    mono_zlib_level: u8,
    raw_zlib_level: u8,
}

const TIGHT_CONF: [TightConf; 4] = [
    TightConf { mono_min_rect_size: 6, idx_zlib_level: 0, mono_zlib_level: 0, raw_zlib_level: 0 },  // Level 0
    TightConf { mono_min_rect_size: 32, idx_zlib_level: 1, mono_zlib_level: 1, raw_zlib_level: 1 }, // Level 1
    TightConf { mono_min_rect_size: 32, idx_zlib_level: 3, mono_zlib_level: 3, raw_zlib_level: 2 }, // Level 2
    TightConf { mono_min_rect_size: 32, idx_zlib_level: 7, mono_zlib_level: 7, raw_zlib_level: 5 }, // Level 9
];

/// Rectangle to encode
#[derive(Debug, Clone)]
struct Rect {
    x: u16,
    y: u16,
    w: u16,
    h: u16,
}

/// Result of encoding a rectangle
struct EncodeResult {
    rectangles: Vec<(Rect, BytesMut)>,
}

/// Implements the VNC "Tight" encoding (RFC 6143 section 7.7.4).
pub struct TightEncoding;

impl Encoding for TightEncoding {
    fn encode(&self, data: &[u8], width: u16, height: u16, quality: u8, compression: u8) -> BytesMut {
        // Simple wrapper - for full optimization, use encode_rect_optimized
        let rect = Rect { x: 0, y: 0, w: width, h: height };
        let result = encode_rect_optimized(data, width, &rect, quality, compression);

        // Concatenate all rectangles
        let mut output = BytesMut::new();
        for (_rect, buf) in result.rectangles {
            output.extend_from_slice(&buf);
        }
        output
    }
}

/// High-level optimization: split rectangles and find solid areas
/// Implements Tight encoding optimization as specified in RFC 6143
fn encode_rect_optimized(
    framebuffer: &[u8],
    fb_width: u16,
    rect: &Rect,
    quality: u8,
    compression: u8,
) -> EncodeResult {
    let mut rectangles = Vec::new();

    // Normalize compression level based on quality settings
    let compression = normalize_compression_level(compression, quality);

    // Check if optimization should be applied
    if (rect.w as usize * rect.h as usize) < MIN_SPLIT_RECT_SIZE {
        // Too small - encode directly
        let buf = encode_subrect(framebuffer, fb_width, rect, quality, compression);
        rectangles.push((rect.clone(), buf));
        return EncodeResult { rectangles };
    }

    // Calculate maximum rows per rectangle
    let n_max_width = rect.w.min(TIGHT_MAX_RECT_WIDTH);
    let n_max_rows = (TIGHT_MAX_RECT_SIZE / n_max_width as usize) as u16;

    // Try to find large solid-color areas for optimization
    let mut current_y = rect.y;
    let mut remaining_h = rect.h;

    while current_y < rect.y + rect.h {
        // Check if rectangle becomes too large
        if (current_y - rect.y) >= n_max_rows {
            let chunk_rect = Rect {
                x: rect.x,
                y: rect.y + (current_y - rect.y - n_max_rows),
                w: rect.w,
                h: n_max_rows,
            };
            let buf = encode_subrect(framebuffer, fb_width, &chunk_rect, quality, compression);
            rectangles.push((chunk_rect, buf));
            remaining_h -= n_max_rows;
        }

        let dy_end = (current_y + MAX_SPLIT_TILE_SIZE).min(rect.y + rect.h);
        let dh = dy_end - current_y;

        let mut current_x = rect.x;
        while current_x < rect.x + rect.w {
            let dx_end = (current_x + MAX_SPLIT_TILE_SIZE).min(rect.x + rect.w);
            let dw = dx_end - current_x;

            // Check if tile is solid
            if let Some(color_value) = check_solid_tile(framebuffer, fb_width, current_x, current_y, dw, dh, None) {
                // Find best solid area
                let (w_best, h_best) = find_best_solid_area(
                    framebuffer,
                    fb_width,
                    current_x,
                    current_y,
                    rect.w - (current_x - rect.x),
                    remaining_h - (current_y - rect.y),
                    color_value,
                );

                // Check if solid area is large enough
                if w_best * h_best != rect.w * remaining_h && (w_best as usize * h_best as usize) < MIN_SOLID_SUBRECT_SIZE {
                    current_x += dw;
                    continue;
                }

                // Extend solid area
                let (x_best, y_best, w_best, h_best) = extend_solid_area(
                    framebuffer,
                    fb_width,
                    rect.x,
                    current_y,
                    rect.w,
                    remaining_h,
                    color_value,
                    current_x,
                    current_y,
                    w_best,
                    h_best,
                );

                // Send rectangles before solid area
                if y_best != current_y {
                    let top_rect = Rect {
                        x: rect.x,
                        y: current_y,
                        w: rect.w,
                        h: y_best - current_y,
                    };
                    let buf = encode_subrect(framebuffer, fb_width, &top_rect, quality, compression);
                    rectangles.push((top_rect, buf));
                }

                if x_best != rect.x {
                    let left_rect = Rect {
                        x: rect.x,
                        y: y_best,
                        w: x_best - rect.x,
                        h: h_best,
                    };
                    let sub_result = encode_rect_optimized(framebuffer, fb_width, &left_rect, quality, compression);
                    rectangles.extend(sub_result.rectangles);
                }

                // Send solid rectangle
                let solid_rect = Rect {
                    x: x_best,
                    y: y_best,
                    w: w_best,
                    h: h_best,
                };
                let buf = encode_solid_rect(color_value);
                rectangles.push((solid_rect, buf));

                // Send remaining rectangles
                if x_best + w_best != rect.x + rect.w {
                    let right_rect = Rect {
                        x: x_best + w_best,
                        y: y_best,
                        w: rect.w - (x_best - rect.x) - w_best,
                        h: h_best,
                    };
                    let sub_result = encode_rect_optimized(framebuffer, fb_width, &right_rect, quality, compression);
                    rectangles.extend(sub_result.rectangles);
                }

                if y_best + h_best != current_y + remaining_h {
                    let bottom_rect = Rect {
                        x: rect.x,
                        y: y_best + h_best,
                        w: rect.w,
                        h: remaining_h - (y_best - current_y) - h_best,
                    };
                    let sub_result = encode_rect_optimized(framebuffer, fb_width, &bottom_rect, quality, compression);
                    rectangles.extend(sub_result.rectangles);
                }

                return EncodeResult { rectangles };
            }

            current_x += dw;
        }

        current_y += dh;
    }

    // No solid areas found - encode normally
    let buf = encode_subrect(framebuffer, fb_width, rect, quality, compression);
    rectangles.push((rect.clone(), buf));
    EncodeResult { rectangles }
}

/// Normalize compression level based on JPEG quality
/// Maps compression level 0-9 to internal configuration indices
fn normalize_compression_level(compression: u8, quality: u8) -> u8 {
    let mut level = compression;

    // Map compression level 0-9 to 0-3 (configuration array indices)
    if level == 9 {
        level = 3;
    } else if level > 1 {
        if quality < 10 {
            // JPEG enabled - allow level 2
            level = level.min(2);
        } else {
            // JPEG disabled - cap at level 1
            level = level.min(1);
        }
    }

    level
}

/// Low-level encoding: analyze and encode a single subrectangle
/// Analyzes palette and selects optimal encoding mode
fn encode_subrect(
    framebuffer: &[u8],
    fb_width: u16,
    rect: &Rect,
    quality: u8,
    compression: u8,
) -> BytesMut {
    // Split if too large
    if rect.w > TIGHT_MAX_RECT_WIDTH || ((rect.w as usize) * (rect.h as usize)) > TIGHT_MAX_RECT_SIZE {
        return encode_large_rect(framebuffer, fb_width, rect, quality, compression);
    }

    // Extract pixel data for this rectangle
    let pixels = extract_rect_rgba(framebuffer, fb_width, rect);

    // Analyze palette
    let palette = analyze_palette(&pixels, rect.w as usize * rect.h as usize, compression);

    // Route to appropriate encoder based on palette
    match palette.num_colors {
        0 => {
            // Truecolor - use JPEG or full-color
            if quality < 10 {
                encode_jpeg_rect(&pixels, rect.w, rect.h, quality)
            } else {
                encode_full_color_rect(&pixels, rect.w, rect.h, compression)
            }
        }
        1 => {
            // Solid color
            encode_solid_rect(palette.colors[0])
        }
        2 => {
            // Mono rect (2 colors)
            encode_mono_rect(&pixels, rect.w, rect.h, palette.colors[0], palette.colors[1], compression)
        }
        _ => {
            // Indexed palette (3-16 colors)
            encode_indexed_rect(&pixels, rect.w, rect.h, &palette.colors[..palette.num_colors], compression)
        }
    }
}

/// Encode large rectangle by splitting it into smaller tiles
/// Ensures rectangles stay within size limits
fn encode_large_rect(
    framebuffer: &[u8],
    fb_width: u16,
    rect: &Rect,
    quality: u8,
    compression: u8,
) -> BytesMut {
    let subrect_max_width = rect.w.min(TIGHT_MAX_RECT_WIDTH);
    let subrect_max_height = (TIGHT_MAX_RECT_SIZE / subrect_max_width as usize) as u16;

    let mut output = BytesMut::new();

    let mut dy = 0;
    while dy < rect.h {
        let mut dx = 0;
        while dx < rect.w {
            let rw = (rect.w - dx).min(TIGHT_MAX_RECT_WIDTH);
            let rh = (rect.h - dy).min(subrect_max_height);

            let sub_rect = Rect {
                x: rect.x + dx,
                y: rect.y + dy,
                w: rw,
                h: rh,
            };

            let buf = encode_subrect(framebuffer, fb_width, &sub_rect, quality, compression);
            output.extend_from_slice(&buf);

            dx += TIGHT_MAX_RECT_WIDTH;
        }
        dy += subrect_max_height;
    }

    output
}

/// Check if a tile is all the same color
/// Used for solid area detection optimization
fn check_solid_tile(
    framebuffer: &[u8],
    fb_width: u16,
    x: u16,
    y: u16,
    w: u16,
    h: u16,
    need_same_color: Option<u32>,
) -> Option<u32> {
    let _fb_stride = fb_width as usize * 4; // RGBA32
    let offset = (y as usize * fb_width as usize + x as usize) * 4;

    // Get first pixel color (RGB24)
    let first_color = rgba_to_rgb24(
        framebuffer[offset],
        framebuffer[offset + 1],
        framebuffer[offset + 2],
    );

    // Check if we need a specific color
    if let Some(required) = need_same_color {
        if first_color != required {
            return None;
        }
    }

    // Check all pixels
    for dy in 0..h {
        let row_offset = ((y + dy) as usize * fb_width as usize + x as usize) * 4;
        for dx in 0..w {
            let pix_offset = row_offset + dx as usize * 4;
            let color = rgba_to_rgb24(
                framebuffer[pix_offset],
                framebuffer[pix_offset + 1],
                framebuffer[pix_offset + 2],
            );
            if color != first_color {
                return None;
            }
        }
    }

    Some(first_color)
}

/// Find best solid area dimensions
/// Determines optimal size for solid color subrectangle
fn find_best_solid_area(
    framebuffer: &[u8],
    fb_width: u16,
    x: u16,
    y: u16,
    w: u16,
    h: u16,
    color_value: u32,
) -> (u16, u16) {
    let mut w_best = 0;
    let mut h_best = 0;
    let mut w_prev = w;

    let mut dy = 0;
    while dy < h {
        let dh = (h - dy).min(MAX_SPLIT_TILE_SIZE);
        let dw = w_prev.min(MAX_SPLIT_TILE_SIZE);

        if check_solid_tile(framebuffer, fb_width, x, y + dy, dw, dh, Some(color_value)).is_none() {
            break;
        }

        let mut dx = dw;
        while dx < w_prev {
            let dw_check = (w_prev - dx).min(MAX_SPLIT_TILE_SIZE);
            if check_solid_tile(framebuffer, fb_width, x + dx, y + dy, dw_check, dh, Some(color_value)).is_none() {
                break;
            }
            dx += dw_check;
        }

        w_prev = dx;
        if (w_prev as usize * (dy + dh) as usize) > (w_best as usize * h_best as usize) {
            w_best = w_prev;
            h_best = dy + dh;
        }

        dy += dh;
    }

    (w_best, h_best)
}

/// Extend solid area to maximum size
/// Expands solid region in all directions
fn extend_solid_area(
    framebuffer: &[u8],
    fb_width: u16,
    base_x: u16,
    base_y: u16,
    max_w: u16,
    max_h: u16,
    color_value: u32,
    mut x: u16,
    mut y: u16,
    mut w: u16,
    mut h: u16,
) -> (u16, u16, u16, u16) {
    // Extend upwards
    while y > base_y {
        if check_solid_tile(framebuffer, fb_width, x, y - 1, w, 1, Some(color_value)).is_none() {
            break;
        }
        y -= 1;
        h += 1;
    }

    // Extend downwards
    while y + h < base_y + max_h {
        if check_solid_tile(framebuffer, fb_width, x, y + h, w, 1, Some(color_value)).is_none() {
            break;
        }
        h += 1;
    }

    // Extend left
    while x > base_x {
        if check_solid_tile(framebuffer, fb_width, x - 1, y, 1, h, Some(color_value)).is_none() {
            break;
        }
        x -= 1;
        w += 1;
    }

    // Extend right
    while x + w < base_x + max_w {
        if check_solid_tile(framebuffer, fb_width, x + w, y, 1, h, Some(color_value)).is_none() {
            break;
        }
        w += 1;
    }

    (x, y, w, h)
}

/// Palette analysis result
struct Palette {
    num_colors: usize,
    colors: [u32; 256],
    mono_background: u32,
    mono_foreground: u32,
}

/// Analyze palette from pixel data
/// Determines color count and encoding mode selection
fn analyze_palette(pixels: &[u8], pixel_count: usize, compression: u8) -> Palette {
    let conf_idx = match compression {
        0 => 0,
        1 => 1,
        2 | 3 => 2,
        _ => 3,
    };
    let conf = &TIGHT_CONF[conf_idx];

    let mut palette = Palette {
        num_colors: 0,
        colors: [0; 256],
        mono_background: 0,
        mono_foreground: 0,
    };

    if pixel_count == 0 {
        return palette;
    }

    // Get first color
    let c0 = rgba_to_rgb24(pixels[0], pixels[1], pixels[2]);

    // Count how many pixels match first color
    let mut i = 4;
    while i < pixels.len() && rgba_to_rgb24(pixels[i], pixels[i + 1], pixels[i + 2]) == c0 {
        i += 4;
    }

    if i >= pixels.len() {
        // Solid color
        palette.num_colors = 1;
        palette.colors[0] = c0;
        return palette;
    }

    // Check for 2-color (mono) case
    if pixel_count >= conf.mono_min_rect_size {
        let n0 = i / 4;
        let c1 = rgba_to_rgb24(pixels[i], pixels[i + 1], pixels[i + 2]);
        let mut n1 = 0;

        i += 4;
        while i < pixels.len() {
            let color = rgba_to_rgb24(pixels[i], pixels[i + 1], pixels[i + 2]);
            if color == c0 {
                // n0 already counted
            } else if color == c1 {
                n1 += 1;
            } else {
                break;
            }
            i += 4;
        }

        if i >= pixels.len() {
            // Only 2 colors found
            palette.num_colors = 2;
            if n0 > n1 {
                palette.mono_background = c0;
                palette.mono_foreground = c1;
                palette.colors[0] = c0;
                palette.colors[1] = c1;
            } else {
                palette.mono_background = c1;
                palette.mono_foreground = c0;
                palette.colors[0] = c1;
                palette.colors[1] = c0;
            }
            return palette;
        }
    }

    // More than 2 colors - full palette or truecolor
    palette.num_colors = 0;
    palette
}

/// Extract RGBA rectangle from framebuffer
fn extract_rect_rgba(framebuffer: &[u8], fb_width: u16, rect: &Rect) -> Vec<u8> {
    let mut pixels = Vec::with_capacity(rect.w as usize * rect.h as usize * 4);

    for y in 0..rect.h {
        let row_offset = ((rect.y + y) as usize * fb_width as usize + rect.x as usize) * 4;
        let row_end = row_offset + rect.w as usize * 4;
        pixels.extend_from_slice(&framebuffer[row_offset..row_end]);
    }

    pixels
}

/// Convert RGBA to RGB24
#[inline]
fn rgba_to_rgb24(r: u8, g: u8, b: u8) -> u32 {
    ((r as u32) << 16) | ((g as u32) << 8) | (b as u32)
}

/// Encode solid rectangle
/// Implements solid fill encoding mode (1 color)
fn encode_solid_rect(color: u32) -> BytesMut {
    let mut buf = BytesMut::with_capacity(4);
    buf.put_u8(TIGHT_FILL << 4); // 0x80
    put_pixel24(&mut buf, color);
    info!("Tight solid: 0x{:06x}, {} bytes", color, buf.len());
    buf
}

/// Encode mono rectangle (2 colors)
/// Implements monochrome bitmap encoding with palette
fn encode_mono_rect(
    pixels: &[u8],
    width: u16,
    height: u16,
    bg: u32,
    fg: u32,
    compression: u8,
) -> BytesMut {
    let conf_idx = match compression {
        0 => 0,
        1 => 1,
        2 | 3 => 2,
        _ => 3,
    };
    let zlib_level = TIGHT_CONF[conf_idx].mono_zlib_level;

    // Encode bitmap
    let bitmap = encode_mono_bitmap(pixels, width, height, bg);

    let mut buf = BytesMut::new();

    // Control byte
    if zlib_level == 0 {
        buf.put_u8((TIGHT_NO_ZLIB | TIGHT_EXPLICIT_FILTER) << 4);
    } else {
        buf.put_u8((STREAM_ID_MONO | TIGHT_EXPLICIT_FILTER) << 4);
    }

    // Filter and palette
    buf.put_u8(TIGHT_FILTER_PALETTE);
    buf.put_u8(1); // 2 colors - 1

    // Palette colors
    put_pixel24(&mut buf, bg);
    put_pixel24(&mut buf, fg);

    // Compress data
    compress_data(&mut buf, &bitmap, zlib_level);

    info!("Tight mono: {}x{}, {} bytes", width, height, buf.len());
    buf
}

/// Encode indexed palette rectangle (3-16 colors)
/// Implements palette-based encoding with color indices
fn encode_indexed_rect(
    pixels: &[u8],
    width: u16,
    height: u16,
    palette: &[u32],
    compression: u8,
) -> BytesMut {
    let conf_idx = match compression {
        0 => 0,
        1 => 1,
        2 | 3 => 2,
        _ => 3,
    };
    let zlib_level = TIGHT_CONF[conf_idx].idx_zlib_level;

    // Build color-to-index map
    let mut color_map = HashMap::new();
    for (idx, &color) in palette.iter().enumerate() {
        color_map.insert(color, idx as u8);
    }

    // Encode indices
    let mut indices = Vec::with_capacity(width as usize * height as usize);
    for chunk in pixels.chunks_exact(4) {
        let color = rgba_to_rgb24(chunk[0], chunk[1], chunk[2]);
        indices.push(*color_map.get(&color).unwrap_or(&0));
    }

    let mut buf = BytesMut::new();

    // Control byte
    if zlib_level == 0 {
        buf.put_u8((TIGHT_NO_ZLIB | TIGHT_EXPLICIT_FILTER) << 4);
    } else {
        buf.put_u8((STREAM_ID_INDEXED | TIGHT_EXPLICIT_FILTER) << 4);
    }

    // Filter and palette size
    buf.put_u8(TIGHT_FILTER_PALETTE);
    buf.put_u8((palette.len() - 1) as u8);

    // Palette colors
    for &color in palette {
        put_pixel24(&mut buf, color);
    }

    // Compress data
    compress_data(&mut buf, &indices, zlib_level);

    info!("Tight indexed: {} colors, {}x{}, {} bytes", palette.len(), width, height, buf.len());
    buf
}

/// Encode full-color rectangle
/// Implements full-color zlib encoding for truecolor images
fn encode_full_color_rect(
    pixels: &[u8],
    width: u16,
    height: u16,
    compression: u8,
) -> BytesMut {
    let conf_idx = match compression {
        0 => 0,
        1 => 1,
        2 | 3 => 2,
        _ => 3,
    };
    let zlib_level = TIGHT_CONF[conf_idx].raw_zlib_level;

    // Convert RGBA to RGB24
    let mut rgb_data = Vec::with_capacity(width as usize * height as usize * 3);
    for chunk in pixels.chunks_exact(4) {
        rgb_data.push(chunk[0]);
        rgb_data.push(chunk[1]);
        rgb_data.push(chunk[2]);
    }

    let mut buf = BytesMut::new();

    // Control byte
    if zlib_level == 0 {
        buf.put_u8(TIGHT_NO_ZLIB << 4);
    } else {
        buf.put_u8(STREAM_ID_FULL_COLOR << 4);
    }

    // Compress data
    compress_data(&mut buf, &rgb_data, zlib_level);

    info!("Tight full-color: {}x{}, {} bytes", width, height, buf.len());
    buf
}

/// Encode JPEG rectangle
/// Implements lossy JPEG compression for photographic content
fn encode_jpeg_rect(
    pixels: &[u8],
    width: u16,
    height: u16,
    quality: u8,
) -> BytesMut {
    #[cfg(feature = "turbojpeg")]
    {
        use crate::jpeg::TurboJpegEncoder;

        // Convert RGBA to RGB
        let mut rgb_data = Vec::with_capacity(width as usize * height as usize * 3);
        for chunk in pixels.chunks_exact(4) {
            rgb_data.push(chunk[0]);
            rgb_data.push(chunk[1]);
            rgb_data.push(chunk[2]);
        }

        // Compress with TurboJPEG
        let jpeg_data = match TurboJpegEncoder::new() {
            Ok(mut encoder) => {
                match encoder.compress_rgb(&rgb_data, width, height, quality) {
                    Ok(data) => data,
                    Err(e) => {
                        info!("TurboJPEG failed: {}, using full-color", e);
                        return encode_full_color_rect(pixels, width, height, 6);
                    }
                }
            }
            Err(e) => {
                info!("TurboJPEG init failed: {}, using full-color", e);
                return encode_full_color_rect(pixels, width, height, 6);
            }
        };

        let mut buf = BytesMut::new();
        buf.put_u8(TIGHT_JPEG << 4); // 0x90
        write_compact_length(&mut buf, jpeg_data.len());
        buf.put_slice(&jpeg_data);

        info!("Tight JPEG: {}x{}, quality {}, {} bytes", width, height, quality, jpeg_data.len());
        buf
    }

    #[cfg(not(feature = "turbojpeg"))]
    {
        info!("TurboJPEG not enabled, using full-color (quality={})", quality);
        encode_full_color_rect(pixels, width, height, 6)
    }
}

/// Compress data with zlib or send uncompressed
/// Handles compression based on data size and level settings
fn compress_data(buf: &mut BytesMut, data: &[u8], zlib_level: u8) {
    // Data < 12 bytes sent raw WITHOUT length
    if data.len() < TIGHT_MIN_TO_COMPRESS {
        buf.put_slice(data);
        return;
    }

    // zlibLevel == 0 means uncompressed WITH length
    if zlib_level == 0 {
        write_compact_length(buf, data.len());
        buf.put_slice(data);
        return;
    }

    // Compress with zlib
    let comp_level = Compression::new(zlib_level as u32);
    let mut encoder = ZlibEncoder::new(Vec::new(), comp_level);

    match encoder.write_all(data).and_then(|_| encoder.finish()) {
        Ok(compressed) => {
            write_compact_length(buf, compressed.len());
            buf.put_slice(&compressed);
        }
        Err(_) => {
            // Compression failed - send uncompressed
            write_compact_length(buf, data.len());
            buf.put_slice(data);
        }
    }
}

/// Encode mono bitmap (1 bit per pixel)
/// Converts 2-color image to packed bitmap format
fn encode_mono_bitmap(pixels: &[u8], width: u16, height: u16, bg: u32) -> Vec<u8> {
    let w = width as usize;
    let h = height as usize;
    let bytes_per_row = (w + 7) / 8;
    let mut bitmap = vec![0u8; bytes_per_row * h];

    let mut bitmap_idx = 0;
    for y in 0..h {
        let mut byte_val = 0u8;
        let mut bit_pos = 7i32; // MSB first

        for x in 0..w {
            let pix_offset = (y * w + x) * 4;
            let color = rgba_to_rgb24(pixels[pix_offset], pixels[pix_offset + 1], pixels[pix_offset + 2]);

            if color != bg {
                byte_val |= 1 << bit_pos;
            }

            if bit_pos == 0 {
                bitmap[bitmap_idx] = byte_val;
                bitmap_idx += 1;
                byte_val = 0;
                bit_pos = 7;
            } else {
                bit_pos -= 1;
            }
        }

        // Write partial byte at end of row
        if w % 8 != 0 {
            bitmap[bitmap_idx] = byte_val;
            bitmap_idx += 1;
        }
    }

    bitmap
}

/// Write compact length encoding
/// Implements variable-length integer encoding for Tight protocol
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

/// Trait for managing persistent zlib compression streams
///
/// Implementations of this trait maintain separate compression streams for different
/// data types (full-color, mono, indexed) to improve compression ratios across
/// multiple rectangle updates.
pub trait TightStreamCompressor {
    /// Compresses data using a persistent zlib stream
    ///
    /// # Arguments
    /// * `stream_id` - Stream identifier (STREAM_ID_FULL_COLOR, STREAM_ID_MONO, or STREAM_ID_INDEXED)
    /// * `level` - Compression level (0-9)
    /// * `input` - Data to compress
    ///
    /// # Returns
    /// Compressed data or error message
    fn compress_tight_stream(&mut self, stream_id: u8, level: u8, input: &[u8]) -> Result<Vec<u8>, String>;
}

/// Encode Tight with persistent zlib streams (for use with VNC client streams)
pub fn encode_tight_with_streams<C: TightStreamCompressor>(
    data: &[u8],
    width: u16,
    height: u16,
    quality: u8,
    compression: u8,
    _compressor: &mut C,
) -> BytesMut {
    // For now, use standard encoding (persistent streams require more plumbing)
    // TODO: Integrate persistent compression streams into encode_rect_optimized
    let encoding = TightEncoding;
    encoding.encode(data, width, height, quality, compression)
}
