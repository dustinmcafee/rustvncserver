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

//! VNC Zlib encoding implementation.
//!
//! Simple zlib compression on raw pixel data using the client's pixel format.

use bytes::{BufMut, BytesMut};
use flate2::{Compress, FlushCompress};
use std::io;

/// Encodes pixel data using Zlib with a persistent compressor (RFC 6143 compliant).
///
/// This maintains compression state across rectangles as required by RFC 6143.
/// The implementation matches standard VNC protocol's approach: single `deflate()` call per rectangle.
///
/// # Arguments
/// * `data` - RGBA pixel data (4 bytes per pixel)
/// * `compressor` - Persistent zlib compressor maintaining state across rectangles
///
/// # Returns
///
/// 4-byte length header + compressed data
///
/// # Errors
///
/// Returns an error if zlib compression fails
#[allow(clippy::cast_possible_truncation)] // Zlib total_in/total_out limited to buffer size
pub fn encode_zlib_persistent(data: &[u8], compressor: &mut Compress) -> io::Result<Vec<u8>> {
    // Convert RGBA to RGBX (client pixel format for 32bpp)
    // R at byte 0, G at byte 1, B at byte 2, padding at byte 3
    let mut pixel_data = Vec::with_capacity(data.len());
    for chunk in data.chunks_exact(4) {
        pixel_data.push(chunk[0]); // R
        pixel_data.push(chunk[1]); // G
        pixel_data.push(chunk[2]); // B
        pixel_data.push(0); // Padding
    }

    // Calculate maximum compressed size (zlib overhead formula)
    // From zlib.h: compressed size ≤ uncompressed + (uncompressed/1000) + 12
    let max_compressed_size = pixel_data.len() + (pixel_data.len() / 1000) + 12;
    let mut compressed_output = vec![0u8; max_compressed_size];

    // Track total_in and total_out before compression
    let previous_in = compressor.total_in();
    let previous_out = compressor.total_out();

    // Single deflate() call with Z_SYNC_FLUSH (RFC 6143 Section 7.7.2)
    compressor.compress(&pixel_data, &mut compressed_output, FlushCompress::Sync)?;

    // Calculate actual compressed length and consumed input
    let compressed_len = (compressor.total_out() - previous_out) as usize;
    let total_consumed = (compressor.total_in() - previous_in) as usize;

    // Verify all input was consumed
    if total_consumed < pixel_data.len() {
        return Err(io::Error::new(
            io::ErrorKind::Other,
            format!(
                "Zlib: incomplete compression {}/{}",
                total_consumed,
                pixel_data.len()
            ),
        ));
    }

    // Build result: 4-byte big-endian length + compressed data
    let mut result = BytesMut::with_capacity(4 + compressed_len);
    result.put_u32(compressed_len as u32);
    result.extend_from_slice(&compressed_output[..compressed_len]);

    Ok(result.to_vec())
}
