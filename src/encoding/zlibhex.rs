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


//! VNC ZlibHex encoding implementation.
//!
//! ZlibHex combines Hextile encoding with zlib compression for improved
//! bandwidth efficiency while maintaining the tile-based structure.

use bytes::{BufMut, BytesMut};
use flate2::{Compress, FlushCompress};
use std::io;
use super::HextileEncoding;
use super::Encoding;

/// Encodes pixel data using ZlibHex with a persistent compressor (RFC 6143 compliant).
///
/// This encoding first applies Hextile encoding to the pixel data, then compresses
/// the result using zlib. The compressor maintains state across rectangles for better
/// compression ratios.
///
/// # Arguments
/// * `data` - RGBA pixel data (4 bytes per pixel)
/// * `width` - Width of the rectangle in pixels
/// * `height` - Height of the rectangle in pixels
/// * `compressor` - Persistent zlib compressor maintaining state across rectangles
///
/// # Returns
/// * `Ok(Vec<u8>)` - 4-byte length header + compressed Hextile data
/// * `Err` - Compression error
pub fn encode_zlibhex_persistent(
    data: &[u8],
    width: u16,
    height: u16,
    compressor: &mut Compress,
) -> io::Result<Vec<u8>> {
    // First, encode using Hextile
    let hextile_encoder = HextileEncoding;
    let hextile_data = hextile_encoder.encode(data, width, height, 0, 0);

    // Calculate maximum compressed size (zlib overhead formula)
    // From zlib.h: compressed size ≤ uncompressed + (uncompressed/1000) + 12
    let max_compressed_size = hextile_data.len() + (hextile_data.len() / 100) + 12;
    let mut compressed_output = vec![0u8; max_compressed_size];

    // Track total_out before compression (standard VNC protocol style)
    let previous_out = compressor.total_out();

    // Single deflate() call with Z_SYNC_FLUSH (RFC 6143 Section 7.7.2)
    compressor.compress(
        &hextile_data,
        &mut compressed_output,
        FlushCompress::Sync
    )?;

    // Calculate actual compressed length
    let compressed_len = (compressor.total_out() - previous_out) as usize;

    // Verify all input was consumed
    let total_consumed = (compressor.total_in() - (previous_out - compressed_len as u64)) as usize;
    if total_consumed < hextile_data.len() {
        return Err(io::Error::new(
            io::ErrorKind::Other,
            format!("ZlibHex: incomplete compression {}/{}", total_consumed, hextile_data.len())
        ));
    }

    // Build result: 4-byte big-endian length + compressed data
    let mut result = BytesMut::with_capacity(4 + compressed_len);
    result.put_u32(compressed_len as u32);
    result.extend_from_slice(&compressed_output[..compressed_len]);

    Ok(result.to_vec())
}
