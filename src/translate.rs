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


//! Pixel format translation between server and client formats.
//!
//! This module provides pixel format conversion to support VNC clients with different
//! color depths and pixel layouts. It implements the translation logic using direct
//! runtime conversion instead of lookup tables.
//!
//! # Supported Formats
//!
//! - **32bpp**: RGBA32, BGRA32, ARGB32, ABGR32 (various shift combinations)
//! - **16bpp**: RGB565, RGB555, BGR565, BGR555
//! - **8bpp**: BGR233 (3-bit red, 3-bit green, 2-bit blue)
//!
//! # Performance
//!
//! This implementation uses direct pixel translation. Modern Rust's optimizer can generate
//! very efficient code for this approach, trading a small amount of CPU for significantly
//! simpler code and lower memory usage compared to lookup table approaches.

use crate::protocol::PixelFormat;
use bytes::BytesMut;

/// Translates pixel data from server format (RGBA32) to client's requested format.
///
/// # Arguments
///
/// * `src` - Source pixel data in RGBA32 format (4 bytes per pixel)
/// * `server_format` - The server's pixel format (should be RGBA32)
/// * `client_format` - The client's requested pixel format
///
/// # Returns
///
/// A `BytesMut` containing the translated pixel data in the client's format.
///
/// # Panics
///
/// Panics if the source data length is not a multiple of 4 (invalid RGBA32 data).
pub fn translate_pixels(
    src: &[u8],
    server_format: &PixelFormat,
    client_format: &PixelFormat,
) -> BytesMut {
    // Fast path: no translation needed
    if pixel_formats_equal(server_format, client_format) {
        return BytesMut::from(src);
    }

    assert_eq!(
        src.len() % 4,
        0,
        "Source data must be RGBA32 (4 bytes per pixel)"
    );

    let pixel_count = src.len() / 4;
    let bytes_per_pixel = (client_format.bits_per_pixel / 8) as usize;
    let mut dst = BytesMut::with_capacity(pixel_count * bytes_per_pixel);

    // Translate pixel by pixel
    for i in 0..pixel_count {
        let offset = i * 4;
        let rgba = &src[offset..offset + 4];

        // Extract RGB components from server format
        let (r, g, b) = extract_rgb(rgba, server_format);

        // Pack into client format
        pack_pixel(&mut dst, r, g, b, client_format);
    }

    dst
}

/// Extracts RGB components from a pixel in the given format.
///
/// # Arguments
///
/// * `pixel` - Pixel data (1-4 bytes depending on format)
/// * `format` - The pixel format describing how to interpret the data
///
/// # Returns
///
/// A tuple `(r, g, b)` with each component as a u8 value (0-255).
fn extract_rgb(pixel: &[u8], format: &PixelFormat) -> (u8, u8, u8) {
    // Read pixel value based on bitsPerPixel
    let pixel_value = match format.bits_per_pixel {
        8 => pixel[0] as u32,
        16 => {
            if format.big_endian_flag != 0 {
                u16::from_be_bytes([pixel[0], pixel[1]]) as u32
            } else {
                u16::from_le_bytes([pixel[0], pixel[1]]) as u32
            }
        }
        32 => {
            if format.big_endian_flag != 0 {
                u32::from_be_bytes([pixel[0], pixel[1], pixel[2], pixel[3]])
            } else {
                u32::from_le_bytes([pixel[0], pixel[1], pixel[2], pixel[3]])
            }
        }
        24 => {
            // 24bpp is stored in 3 bytes, but we need to handle it carefully
            if format.big_endian_flag != 0 {
                (pixel[0] as u32) << 16 | (pixel[1] as u32) << 8 | (pixel[2] as u32)
            } else {
                (pixel[2] as u32) << 16 | (pixel[1] as u32) << 8 | (pixel[0] as u32)
            }
        }
        _ => pixel[0] as u32, // Fallback for unsupported formats
    };

    // Extract color components using shifts and masks
    let r_raw = (pixel_value >> format.red_shift) & format.red_max as u32;
    let g_raw = (pixel_value >> format.green_shift) & format.green_max as u32;
    let b_raw = (pixel_value >> format.blue_shift) & format.blue_max as u32;

    // Scale to 8-bit (0-255) range
    let r = scale_component(r_raw, format.red_max);
    let g = scale_component(g_raw, format.green_max);
    let b = scale_component(b_raw, format.blue_max);

    (r, g, b)
}

/// Packs RGB components into the client's pixel format and writes to the buffer.
///
/// # Arguments
///
/// * `dst` - Destination buffer to write the packed pixel
/// * `r` - Red component (0-255)
/// * `g` - Green component (0-255)
/// * `b` - Blue component (0-255)
/// * `format` - The pixel format for packing
fn pack_pixel(dst: &mut BytesMut, r: u8, g: u8, b: u8, format: &PixelFormat) {
    // Scale components from 8-bit to client's color depth
    let r_scaled = downscale_component(r, format.red_max);
    let g_scaled = downscale_component(g, format.green_max);
    let b_scaled = downscale_component(b, format.blue_max);

    // Combine components with shifts
    let pixel_value =
        ((r_scaled as u32) << format.red_shift)
        | ((g_scaled as u32) << format.green_shift)
        | ((b_scaled as u32) << format.blue_shift);

    // Write pixel value based on bitsPerPixel and endianness
    match format.bits_per_pixel {
        8 => {
            dst.extend_from_slice(&[pixel_value as u8]);
        }
        16 => {
            let bytes = if format.big_endian_flag != 0 {
                (pixel_value as u16).to_be_bytes()
            } else {
                (pixel_value as u16).to_le_bytes()
            };
            dst.extend_from_slice(&bytes);
        }
        24 => {
            // 24bpp: write 3 bytes
            let bytes = if format.big_endian_flag != 0 {
                [(pixel_value >> 16) as u8, (pixel_value >> 8) as u8, pixel_value as u8]
            } else {
                [pixel_value as u8, (pixel_value >> 8) as u8, (pixel_value >> 16) as u8]
            };
            dst.extend_from_slice(&bytes);
        }
        32 => {
            let bytes = if format.big_endian_flag != 0 {
                pixel_value.to_be_bytes()
            } else {
                pixel_value.to_le_bytes()
            };
            dst.extend_from_slice(&bytes);
        }
        _ => {
            // Unsupported format, write as 8-bit
            dst.extend_from_slice(&[pixel_value as u8]);
        }
    }
}

/// Scales a color component from its format-specific range to 8-bit (0-255).
///
/// # Arguments
///
/// * `value` - The component value in its native range (0..max)
/// * `max` - The maximum value for this component in the source format
///
/// # Returns
///
/// The scaled value in 0-255 range.
#[inline]
fn scale_component(value: u32, max: u16) -> u8 {
    if max == 0 {
        return 0;
    }
    if max == 255 {
        return value as u8;
    }

    // Scale: value * 255 / max
    // Use 64-bit to avoid overflow
    ((value as u64 * 255) / max as u64) as u8
}

/// Downscales a color component from 8-bit (0-255) to the format-specific range.
///
/// # Arguments
///
/// * `value` - The component value in 0-255 range
/// * `max` - The maximum value for this component in the destination format
///
/// # Returns
///
/// The downscaled value in 0..max range.
#[inline]
fn downscale_component(value: u8, max: u16) -> u16 {
    if max == 0 {
        return 0;
    }
    if max == 255 {
        return value as u16;
    }

    // Downscale: value * max / 255
    // Use 32-bit to avoid overflow
    ((value as u32 * max as u32) / 255) as u16
}

/// Checks if two pixel formats are identical (no translation needed).
///
/// # Arguments
///
/// * `a` - First pixel format
/// * `b` - Second pixel format
///
/// # Returns
///
/// `true` if the formats are identical, `false` otherwise.
fn pixel_formats_equal(a: &PixelFormat, b: &PixelFormat) -> bool {
    a.bits_per_pixel == b.bits_per_pixel
        && a.depth == b.depth
        && (a.big_endian_flag == b.big_endian_flag || a.bits_per_pixel == 8)
        && a.true_colour_flag == b.true_colour_flag
        && (!a.true_colour_flag != 0
            || (a.red_max == b.red_max
                && a.green_max == b.green_max
                && a.blue_max == b.blue_max
                && a.red_shift == b.red_shift
                && a.green_shift == b.green_shift
                && a.blue_shift == b.blue_shift))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_no_translation() {
        let server_format = PixelFormat::rgba32();
        let client_format = PixelFormat::rgba32();

        let src = vec![255u8, 0, 0, 0, 0, 255, 0, 0]; // Red, Green pixels
        let dst = translate_pixels(&src, &server_format, &client_format);

        assert_eq!(&src[..], &dst[..]);
    }

    #[test]
    fn test_rgba32_to_rgb565() {
        let server_format = PixelFormat::rgba32();

        // RGB565: 5-bit red (shift 11), 6-bit green (shift 5), 5-bit blue (shift 0)
        let client_format = PixelFormat {
            bits_per_pixel: 16,
            depth: 16,
            big_endian_flag: 0,
            true_colour_flag: 1,
            red_max: 31,    // 5 bits
            green_max: 63,  // 6 bits
            blue_max: 31,   // 5 bits
            red_shift: 11,
            green_shift: 5,
            blue_shift: 0,
        };

        // Pure red: R=255, G=0, B=0 in RGBA32
        let src = vec![255u8, 0, 0, 0];
        let dst = translate_pixels(&src, &server_format, &client_format);

        // In RGB565: red=(255*31/255)<<11 = 31<<11 = 0xF800
        assert_eq!(dst.len(), 2);
        let value = u16::from_le_bytes([dst[0], dst[1]]);
        assert_eq!(value, 0xF800);
    }

    #[test]
    fn test_extract_rgb_rgba32() {
        let format = PixelFormat::rgba32();
        let pixel = [128u8, 64, 32, 0]; // R=128, G=64, B=32 in RGBA32

        let (r, g, b) = extract_rgb(&pixel, &format);
        assert_eq!(r, 128);
        assert_eq!(g, 64);
        assert_eq!(b, 32);
    }

    #[test]
    fn test_scale_component() {
        // 5-bit (0-31) to 8-bit (0-255)
        assert_eq!(scale_component(0, 31), 0);
        assert_eq!(scale_component(31, 31), 255);
        assert_eq!(scale_component(15, 31), 123); // 15 * 255 / 31 = 123.387... = 123

        // Identity: 8-bit to 8-bit
        assert_eq!(scale_component(128, 255), 128);
    }

    #[test]
    fn test_downscale_component() {
        // 8-bit (0-255) to 5-bit (0-31)
        assert_eq!(downscale_component(0, 31), 0);
        assert_eq!(downscale_component(255, 31), 31);
        assert_eq!(downscale_component(128, 31), 15); // ~half

        // Identity: 8-bit to 8-bit
        assert_eq!(downscale_component(128, 255), 128);
    }
}
