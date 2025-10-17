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
use log::info;

/// Implements the VNC "CoRRE" (Compact RRE) encoding.
///
/// CoRRE is like RRE but uses compact subrectangles with u8 coordinates.
/// Format: \[bgColor\]\[nSubrects(u8)\]\[subrect1\]...\[subrectN\]
/// Each subrect: \[color\]\[x(u8)\]\[y(u8)\]\[w(u8)\]\[h(u8)\]
pub struct CorRreEncoding;

impl Encoding for CorRreEncoding {
    fn encode(&self, data: &[u8], width: u16, height: u16, _quality: u8, _compression: u8) -> BytesMut {
        // CoRRE format per RFC 6143:
        // Protocol layer writes: FramebufferUpdateRectHeader + nSubrects count
        // Encoder writes: bgColor + subrects
        // Each subrect: color(4) + x(1) + y(1) + w(1) + h(1)
        let pixels = rgba_to_rgb24_pixels(data);
        let bg_color = get_background_color(&pixels);

        // Find subrectangles
        let subrects = find_subrects(&pixels, width as usize, height as usize, bg_color);

        // Encoder output: background color + subrectangle data
        // Protocol layer will write nSubrects separately
        let mut buf = BytesMut::with_capacity(4 + subrects.len() * 8);
        buf.put_u32_le(bg_color); // background pixel value (little-endian)

        // Write subrectangles
        for subrect in &subrects {
            buf.put_u32_le(subrect.color); // pixel color (little-endian)
            buf.put_u8(subrect.x as u8);   // x coordinate (u8)
            buf.put_u8(subrect.y as u8);   // y coordinate (u8)
            buf.put_u8(subrect.w as u8);   // width (u8)
            buf.put_u8(subrect.h as u8);   // height (u8)
        }

        // HEX DUMP: Log the exact bytes being encoded
        let hex_str: String = buf.iter()
            .take(32) // Only show first 32 bytes
            .map(|b| format!("{:02x}", b))
            .collect::<Vec<String>>()
            .join(" ");
        info!("CoRRE encoded {}x{}: {} bytes ({}subrects) = [{}...]",
              width, height, buf.len(), subrects.len(), hex_str);

        buf
    }
}
