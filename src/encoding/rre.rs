//! VNC RRE (Rise-and-Run-length Encoding) implementation.
//!
//! RRE encodes a rectangle as a background color plus a list of subrectangles
//! with their own colors. Effective for large solid regions.

use bytes::{BufMut, BytesMut};
use super::Encoding;
use super::common::{rgba_to_rgb24_pixels, get_background_color, find_subrects};

/// Implements the VNC "RRE" (Rise-and-Run-length Encoding).
///
/// RRE encodes a rectangle as a background color plus a list of subrectangles
/// with their own colors. Format: [nSubrects(u32)][bgColor][subrect1]...[subrectN]
/// Each subrect: [color][x(u16)][y(u16)][w(u16)][h(u16)]
pub struct RreEncoding;

impl Encoding for RreEncoding {
    fn encode(&self, data: &[u8], width: u16, height: u16, _quality: u8, _compression: u8) -> BytesMut {
        // Convert RGBA to RGB pixels (u32 format: 0RGB)
        let pixels = rgba_to_rgb24_pixels(data);

        // Find background color (most common pixel)
        let bg_color = get_background_color(&pixels);

        // Find all subrectangles
        let subrects = find_subrects(&pixels, width as usize, height as usize, bg_color);

        // Check if RRE is worth it (otherwise would be larger than raw)
        let encoded_size = 4 + 4 + (subrects.len() * (4 + 8)); // header + bg + subrects
        let raw_size = width as usize * height as usize * 4; // 4 bytes per pixel for 32bpp

        if encoded_size >= raw_size {
            // Fall back to raw encoding within RRE format (0 subrects)
            let mut buf = BytesMut::with_capacity(4 + 4);
            buf.put_u32(0); // 0 subrects (big-endian)
            buf.put_u32_le(bg_color); // background color in client format (little-endian)
            return buf;
        }

        let mut buf = BytesMut::with_capacity(encoded_size);

        // Write header
        buf.put_u32(subrects.len() as u32); // number of subrectangles (big-endian)
        buf.put_u32_le(bg_color); // background color in client pixel format (little-endian)

        // Write subrectangles
        for subrect in subrects {
            buf.put_u32_le(subrect.color); // pixel in client format (little-endian)
            buf.put_u16(subrect.x);  // protocol coordinates (big-endian)
            buf.put_u16(subrect.y);
            buf.put_u16(subrect.w);
            buf.put_u16(subrect.h);
        }

        buf
    }
}
