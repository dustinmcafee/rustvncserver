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

//! Common helper functions shared across multiple VNC encodings.

use bytes::{BufMut, BytesMut};
use std::collections::HashMap;

/// Represents a subrectangle in RRE/CoRRE/Hextile encoding.
#[derive(Debug)]
pub struct Subrect {
    /// The color value of this subrectangle in 32-bit RGB format
    pub color: u32,
    /// The X coordinate of the subrectangle's top-left corner
    pub x: u16,
    /// The Y coordinate of the subrectangle's top-left corner
    pub y: u16,
    /// The width of the subrectangle in pixels
    pub w: u16,
    /// The height of the subrectangle in pixels
    pub h: u16,
}

/// Convert RGBA (4 bytes/pixel) to RGB24 pixel values in VNC pixel format.
/// Our pixel format has: `red_shift=0`, `green_shift=8`, `blue_shift=16`, little-endian
/// So pixel = (R << 0) | (G << 8) | (B << 16) = 0x00BBGGRR
#[must_use]
pub fn rgba_to_rgb24_pixels(data: &[u8]) -> Vec<u32> {
    data.chunks_exact(4)
        .map(|chunk| {
            u32::from(chunk[0]) | // R at bits 0-7
            (u32::from(chunk[1]) << 8)  | // G at bits 8-15
            (u32::from(chunk[2]) << 16) // B at bits 16-23
        })
        .collect()
}

/// Find the most common color in the pixel array.
#[must_use]
pub fn get_background_color(pixels: &[u32]) -> u32 {
    if pixels.is_empty() {
        return 0;
    }

    let mut counts: HashMap<u32, usize> = HashMap::new();
    for &pixel in pixels {
        *counts.entry(pixel).or_insert(0) += 1;
    }

    counts
        .into_iter()
        .max_by_key(|(_, count)| *count)
        .map_or(pixels[0], |(color, _)| color)
}

/// Find subrectangles of non-background pixels.
#[must_use]
#[allow(clippy::cast_possible_truncation)] // Subrect coordinates limited to tile dimensions (max 16x16)
pub fn find_subrects(pixels: &[u32], width: usize, height: usize, bg_color: u32) -> Vec<Subrect> {
    let mut subrects = Vec::new();
    let mut marked = vec![false; pixels.len()];

    for y in 0..height {
        for x in 0..width {
            let idx = y * width + x;
            if marked[idx] || pixels[idx] == bg_color {
                continue;
            }

            let color = pixels[idx];

            // Find largest rectangle starting at (x, y)
            let mut max_w = 0;
            for test_x in x..width {
                let test_idx = y * width + test_x;
                if marked[test_idx] || pixels[test_idx] != color {
                    break;
                }
                max_w = test_x - x + 1;
            }

            let mut h = 1;
            'outer: for test_y in (y + 1)..height {
                for test_x in x..(x + max_w) {
                    let test_idx = test_y * width + test_x;
                    if marked[test_idx] || pixels[test_idx] != color {
                        break 'outer;
                    }
                }
                h = test_y - y + 1;
            }

            // Try horizontal vs vertical rectangle
            let mut best_w = max_w;
            let mut best_h = h;

            // Also try vertical
            let mut max_h = 0;
            for test_y in y..height {
                let test_idx = test_y * width + x;
                if marked[test_idx] || pixels[test_idx] != color {
                    break;
                }
                max_h = test_y - y + 1;
            }

            let mut w2 = 1;
            'outer2: for test_x in (x + 1)..width {
                for test_y in y..(y + max_h) {
                    let test_idx = test_y * width + test_x;
                    if marked[test_idx] || pixels[test_idx] != color {
                        break 'outer2;
                    }
                }
                w2 = test_x - x + 1;
            }

            // Choose larger rectangle
            if w2 * max_h > best_w * best_h {
                best_w = w2;
                best_h = max_h;
            }

            // Mark pixels as used
            for dy in 0..best_h {
                for dx in 0..best_w {
                    marked[(y + dy) * width + (x + dx)] = true;
                }
            }

            subrects.push(Subrect {
                color,
                x: x as u16,
                y: y as u16,
                w: best_w as u16,
                h: best_h as u16,
            });
        }
    }

    subrects
}

/// Extract a tile from the pixel array.
#[must_use]
pub fn extract_tile(
    pixels: &[u32],
    width: usize,
    x: usize,
    y: usize,
    tw: usize,
    th: usize,
) -> Vec<u32> {
    let mut tile = Vec::with_capacity(tw * th);
    for dy in 0..th {
        for dx in 0..tw {
            tile.push(pixels[(y + dy) * width + (x + dx)]);
        }
    }
    tile
}

/// Analyze tile colors to determine if solid, monochrome, or multicolor.
/// Returns: (`is_solid`, `is_mono`, `bg_color`, `fg_color`)
#[must_use]
pub fn analyze_tile_colors(pixels: &[u32]) -> (bool, bool, u32, u32) {
    if pixels.is_empty() {
        return (true, true, 0, 0);
    }

    let mut colors: HashMap<u32, usize> = HashMap::new();
    for &pixel in pixels {
        *colors.entry(pixel).or_insert(0) += 1;
    }

    if colors.len() == 1 {
        return (true, true, pixels[0], 0);
    }

    if colors.len() == 2 {
        let mut sorted: Vec<_> = colors.into_iter().collect();
        sorted.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
        return (false, true, sorted[0].0, sorted[1].0);
    }

    let bg = get_background_color(pixels);
    (false, false, bg, 0)
}

/// Write a 32-bit pixel value to buffer in little-endian format (4 bytes).
/// Pixel format: R at bits 0-7, G at bits 8-15, B at bits 16-23, unused at bits 24-31
/// For 32bpp client: writes [R, G, B, 0] in that order
pub fn put_pixel32(buf: &mut BytesMut, pixel: u32) {
    buf.put_u32_le(pixel); // Write full 32-bit pixel in little-endian format
}

/// Write a 24-bit pixel value to buffer in RGB24 format (3 bytes).
/// Pixel format: R at bits 0-7, G at bits 8-15, B at bits 16-23
/// Implements 24-bit pixel packing as specified in RFC 6143.
/// Writes [R, G, B] in that order (3 bytes total).
pub fn put_pixel24(buf: &mut BytesMut, pixel: u32) {
    buf.put_u8((pixel & 0xFF) as u8); // R
    buf.put_u8(((pixel >> 8) & 0xFF) as u8); // G
    buf.put_u8(((pixel >> 16) & 0xFF) as u8); // B
}

/// Translate a single RGB pixel to the client's pixel format for TIGHT encoding.
/// This properly handles `red_shift`, `green_shift`, `blue_shift` values for
/// correct pixel format translation.
///
/// IMPORTANT: TIGHT encoding uses 24-bit pixel format (3 bytes) when the client has
/// depth=24 with 8-bit color components, even if `bits_per_pixel=32`. This is a
/// standard optimization for reducing bandwidth.
///
/// Input pixel format: RGB stored in bits 0-23 (R=bits 0-7, G=bits 8-15, B=bits 16-23)
/// Output: Translated bytes in client's pixel format (3 or 4 bytes depending on format)
#[must_use]
#[allow(clippy::cast_possible_truncation)] // Intentionally extracting byte components from pixel values
pub fn translate_pixel_to_client_format(
    pixel: u32,
    client_format: &crate::protocol::PixelFormat,
) -> Vec<u8> {
    use crate::protocol::PixelFormat;
    use crate::translate::translate_pixels;

    // Check if we should use 24-bit format (3 bytes) instead of 32-bit (4 bytes)
    let use_24bit = client_format.depth == 24
        && client_format.red_max == 255
        && client_format.green_max == 255
        && client_format.blue_max == 255;

    if use_24bit {
        // Send only 3 bytes for 24-bit depth clients (TIGHT optimization)
        // Match Pack24 behavior exactly: pack pixel then extract using shifts

        // Pack RGB components into client's pixel format
        let r = (pixel & 0xFF) as u8;
        let g = ((pixel >> 8) & 0xFF) as u8;
        let b = ((pixel >> 16) & 0xFF) as u8;

        // Create pixel value using client's bit layout
        let pixel_value = (u32::from(r) << client_format.red_shift)
            | (u32::from(g) << client_format.green_shift)
            | (u32::from(b) << client_format.blue_shift);

        // Extract 3 bytes in the order they appear in memory (like Pack24)
        // For little-endian with shifts 0/8/16: pixel_value = 0x00BBGGRR
        // So bytes are [RR, GG, BB] in memory order
        if client_format.big_endian_flag != 0 {
            // Big-endian: extract from high to low
            vec![
                (pixel_value >> 16) as u8,
                (pixel_value >> 8) as u8,
                pixel_value as u8,
            ]
        } else {
            // Little-endian: extract from low to high
            vec![
                pixel_value as u8,
                (pixel_value >> 8) as u8,
                (pixel_value >> 16) as u8,
            ]
        }
    } else {
        // Create RGBA32 bytes for this single pixel
        let rgba_bytes = [
            (pixel & 0xFF) as u8,         // R
            ((pixel >> 8) & 0xFF) as u8,  // G
            ((pixel >> 16) & 0xFF) as u8, // B
            0,                            // A
        ];

        // Use existing translation logic to convert to client format
        let server_format = PixelFormat::rgba32();
        let translated = translate_pixels(&rgba_bytes, &server_format, client_format);

        // Return as Vec for easier handling
        translated.to_vec()
    }
}

/// Check if all pixels are the same color.
#[must_use]
pub fn check_solid_color(pixels: &[u32]) -> Option<u32> {
    if pixels.is_empty() {
        return None;
    }

    let first = pixels[0];
    if pixels.iter().all(|&p| p == first) {
        Some(first)
    } else {
        None
    }
}

/// Build a color palette from pixels.
#[must_use]
pub fn build_palette(pixels: &[u32]) -> Vec<u32> {
    let mut colors: HashMap<u32, usize> = HashMap::new();
    for &pixel in pixels {
        *colors.entry(pixel).or_insert(0) += 1;
    }

    let mut palette: Vec<_> = colors.into_iter().collect();
    palette.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
    palette.into_iter().map(|(color, _)| color).collect()
}
