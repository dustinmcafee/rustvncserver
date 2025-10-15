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


//! VNC CoRRE (Compact RRE) encoding implementation.
//!
//! CoRRE is like RRE but uses compact subrectangles with u8 coordinates.
//! More efficient for small rectangles.

use bytes::{BufMut, BytesMut};
use super::Encoding;
use super::common::{rgba_to_rgb24_pixels, get_background_color, find_subrects};

/// Implements the VNC "CoRRE" (Compact RRE) encoding.
///
/// CoRRE is like RRE but uses compact subrectangles with u8 coordinates.
/// Format: [nSubrects(u32)][bgColor][subrect1]...[subrectN]
/// Each subrect: [color][x(u8)][y(u8)][w(u8)][h(u8)]
pub struct CorRreEncoding;

impl Encoding for CorRreEncoding {
    fn encode(&self, data: &[u8], width: u16, height: u16, _quality: u8, _compression: u8) -> BytesMut {
        // CoRRE is only suitable for small rectangles (< 256x256)
        if width > 255 || height > 255 {
            // Fall back to raw-style encoding
            let mut buf = BytesMut::with_capacity(4 + 4);
            buf.put_u32(0); // 0 subrects (big-endian)
            let pixels = rgba_to_rgb24_pixels(data);
            let bg_color = get_background_color(&pixels);
            buf.put_u32_le(bg_color); // pixel in client format (little-endian)
            return buf;
        }

        let pixels = rgba_to_rgb24_pixels(data);
        let bg_color = get_background_color(&pixels);
        let subrects = find_subrects(&pixels, width as usize, height as usize, bg_color);

        // Check if CoRRE is worth it
        let encoded_size = 4 + 4 + (subrects.len() * (4 + 4)); // header + bg + compact subrects
        let raw_size = width as usize * height as usize * 4; // 4 bytes per pixel for 32bpp

        if encoded_size >= raw_size {
            let mut buf = BytesMut::with_capacity(4 + 4);
            buf.put_u32(0); // 0 subrects (big-endian)
            buf.put_u32_le(bg_color); // pixel in client format (little-endian)
            return buf;
        }

        let mut buf = BytesMut::with_capacity(encoded_size);

        buf.put_u32(subrects.len() as u32); // count (big-endian)
        buf.put_u32_le(bg_color); // pixel in client format (little-endian)

        for subrect in subrects {
            buf.put_u32_le(subrect.color); // pixel in client format (little-endian)
            buf.put_u8(subrect.x as u8);   // u8 coordinates
            buf.put_u8(subrect.y as u8);
            buf.put_u8(subrect.w as u8);
            buf.put_u8(subrect.h as u8);
        }

        buf
    }
}
