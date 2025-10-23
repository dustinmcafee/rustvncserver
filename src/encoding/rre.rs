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

//! VNC RRE (Rise-and-Run-length Encoding) implementation.
//!
//! RRE encodes a rectangle as a background color plus a list of subrectangles
//! with their own colors. Effective for large solid regions.

use super::common::{find_subrects, get_background_color, rgba_to_rgb24_pixels};
use super::Encoding;
use bytes::{BufMut, BytesMut};

/// Implements the VNC "RRE" (Rise-and-Run-length Encoding).
///
/// RRE encodes a rectangle as a background color plus a list of subrectangles
/// with their own colors. Format: \[nSubrects(u32)\]\[bgColor\]\[subrect1\]...\[subrectN\]
/// Each subrect: \[color\]\[x(u16)\]\[y(u16)\]\[w(u16)\]\[h(u16)\]
pub struct RreEncoding;

impl Encoding for RreEncoding {
    #[allow(clippy::cast_possible_truncation)] // Subrectangle count limited to image size per VNC protocol
    fn encode(
        &self,
        data: &[u8],
        width: u16,
        height: u16,
        _quality: u8,
        _compression: u8,
    ) -> BytesMut {
        // Convert RGBA to RGB pixels (u32 format: 0RGB)
        let pixels = rgba_to_rgb24_pixels(data);

        // Find background color (most common pixel)
        let bg_color = get_background_color(&pixels);

        // Find all subrectangles
        let subrects = find_subrects(&pixels, width as usize, height as usize, bg_color);

        // Always encode all pixels to avoid data loss
        // (Even if RRE is inefficient, we must preserve the image correctly)
        let encoded_size = 4 + 4 + (subrects.len() * (4 + 8)); // header + bg + subrects

        let mut buf = BytesMut::with_capacity(encoded_size);

        // Write header
        buf.put_u32(subrects.len() as u32); // number of subrectangles (big-endian)
        buf.put_u32_le(bg_color); // background color in client pixel format (little-endian)

        // Write subrectangles
        for subrect in subrects {
            buf.put_u32_le(subrect.color); // pixel in client format (little-endian)
            buf.put_u16(subrect.x); // protocol coordinates (big-endian)
            buf.put_u16(subrect.y);
            buf.put_u16(subrect.w);
            buf.put_u16(subrect.h);
        }

        buf
    }
}
