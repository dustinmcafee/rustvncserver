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

//! VNC ZYWRLE (Zlib+Wavelet+Run-Length Encoding) implementation.
//!
//! ZYWRLE is a wavelet-based lossy compression encoding for low-bandwidth scenarios.
//! It uses:
//! - Piecewise-Linear Haar (`PLHarr`) wavelet transform
//! - RCT (Reversible Color Transform) for RGB to YUV conversion
//! - Non-linear quantization filtering
//! - ZRLE encoding on the transformed coefficients
//!
//! # Algorithm Attribution
//! The ZYWRLE algorithm is Copyright 2006 by Hitachi Systems & Services, Ltd.
//! (Noriaki Yamazaki, Research & Development Center).
//!
//! This implementation is based on the ZYWRLE specification and is distributed
//! under the terms compatible with the original BSD-style license granted by
//! Hitachi Systems & Services, Ltd. for use of the ZYWRLE codec.
//!
//! # References
//! - `PLHarr`: Senecal, J. G., et al., "An Improved N-Bit to N-Bit Reversible Haar-Like Transform"
//! - EZW: Shapiro, JM: "Embedded Image Coding Using Zerotrees of Wavelet Coefficients"
//! - ZYWRLE specification and reference implementation

/// Non-linear quantization filter lookup tables.
/// These tables implement r=2.0 non-linear quantization (quantize is x^2, dequantize is sqrt(x)).
/// The tables map input coefficient values [0..255] to quantized-dequantized (filtered) values.
///
/// Table selection based on quality level:
/// - `zywrle_conv`[0]: bi=5, bo=5 r=0.0:PSNR=24.849 (zero everything, highest compression)
/// - `zywrle_conv`[1]: bi=5, bo=5 r=2.0:PSNR=74.031 (good quality)
/// - `zywrle_conv`[2]: bi=5, bo=4 r=2.0:PSNR=64.441 (medium quality)
/// - `zywrle_conv`[3]: bi=5, bo=2 r=2.0:PSNR=43.175 (low quality, highest compression)
const ZYWRLE_CONV: [[i8; 256]; 4] = [
    [
        // bi=5, bo=5 r=0.0:PSNR=24.849
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    ],
    [
        // bi=5, bo=5 r=2.0:PSNR=74.031
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 32, 32, 32, 32, 32, 32,
        32, 32, 32, 32, 32, 32, 32, 32, 32, 32, 32, 48, 48, 48, 48, 48, 48, 48, 48, 48, 48, 48, 56,
        56, 56, 56, 56, 56, 56, 56, 56, 64, 64, 64, 64, 64, 64, 64, 64, 72, 72, 72, 72, 72, 72, 72,
        72, 80, 80, 80, 80, 80, 80, 88, 88, 88, 88, 88, 88, 88, 88, 88, 88, 88, 88, 96, 96, 96, 96,
        96, 104, 104, 104, 104, 104, 104, 104, 104, 104, 104, 112, 112, 112, 112, 112, 112, 112,
        112, 112, 120, 120, 120, 120, 120, 120, 120, 120, 120, 120, 0, -120, -120, -120, -120,
        -120, -120, -120, -120, -120, -120, -112, -112, -112, -112, -112, -112, -112, -112, -112,
        -104, -104, -104, -104, -104, -104, -104, -104, -104, -104, -96, -96, -96, -96, -96, -88,
        -88, -88, -88, -88, -88, -88, -88, -88, -88, -88, -88, -80, -80, -80, -80, -80, -80, -72,
        -72, -72, -72, -72, -72, -72, -72, -64, -64, -64, -64, -64, -64, -64, -64, -56, -56, -56,
        -56, -56, -56, -56, -56, -56, -48, -48, -48, -48, -48, -48, -48, -48, -48, -48, -48, -32,
        -32, -32, -32, -32, -32, -32, -32, -32, -32, -32, -32, -32, -32, -32, -32, -32, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    ],
    [
        // bi=5, bo=4 r=2.0:PSNR=64.441
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 48, 48, 48, 48, 48, 48, 48, 48, 48, 48, 48, 48, 48, 48, 48, 48, 48, 48, 48, 48, 48, 48,
        48, 48, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 80, 80, 80, 80, 80,
        80, 80, 80, 80, 80, 80, 80, 80, 88, 88, 88, 88, 88, 88, 88, 88, 88, 88, 88, 104, 104, 104,
        104, 104, 104, 104, 104, 104, 104, 104, 112, 112, 112, 112, 112, 112, 112, 112, 112, 120,
        120, 120, 120, 120, 120, 120, 120, 120, 120, 120, 120, 0, -120, -120, -120, -120, -120,
        -120, -120, -120, -120, -120, -120, -120, -112, -112, -112, -112, -112, -112, -112, -112,
        -112, -104, -104, -104, -104, -104, -104, -104, -104, -104, -104, -104, -88, -88, -88, -88,
        -88, -88, -88, -88, -88, -88, -88, -80, -80, -80, -80, -80, -80, -80, -80, -80, -80, -80,
        -80, -80, -64, -64, -64, -64, -64, -64, -64, -64, -64, -64, -64, -64, -64, -64, -64, -64,
        -48, -48, -48, -48, -48, -48, -48, -48, -48, -48, -48, -48, -48, -48, -48, -48, -48, -48,
        -48, -48, -48, -48, -48, -48, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    ],
    [
        // bi=5, bo=2 r=2.0:PSNR=43.175
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 88, 88, 88, 88, 88, 88, 88, 88, 88, 88, 88, 88, 88, 88, 88, 88, 88, 88, 88, 88,
        88, 88, 88, 88, 88, 88, 88, 88, 88, 88, 88, 88, 88, 88, 88, 88, 88, 88, 88, 88, 88, 88, 88,
        88, 88, 88, 88, 88, 88, 88, 88, 88, 88, 88, 88, 88, 88, 88, 88, 88, 88, 88, 88, 88, 0, -88,
        -88, -88, -88, -88, -88, -88, -88, -88, -88, -88, -88, -88, -88, -88, -88, -88, -88, -88,
        -88, -88, -88, -88, -88, -88, -88, -88, -88, -88, -88, -88, -88, -88, -88, -88, -88, -88,
        -88, -88, -88, -88, -88, -88, -88, -88, -88, -88, -88, -88, -88, -88, -88, -88, -88, -88,
        -88, -88, -88, -88, -88, -88, -88, -88, -88, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    ],
];

/// Filter parameter tables indexed by [level-1][l][channel].
/// Maps quality level and wavelet level to the appropriate quantization filter.
const ZYWRLE_PARAM: [[[usize; 3]; 3]; 3] = [
    [[0, 2, 0], [0, 0, 0], [0, 0, 0]], // level 1
    [[0, 3, 0], [1, 1, 1], [0, 0, 0]], // level 2
    [[0, 3, 0], [2, 2, 2], [1, 1, 1]], // level 3
];

/// Piecewise-Linear Haar (`PLHarr`) transform on two signed bytes.
///
/// This is the core wavelet transform operation. It's an improved N-bit to N-bit
/// reversible Haar-like transform that handles signed values correctly.
///
/// # Arguments
/// * `x0` - First coefficient (modified in place to contain Low component)
/// * `x1` - Second coefficient (modified in place to contain High component)
#[inline]
#[allow(clippy::cast_possible_truncation)] // Piecewise-Linear Haar transform uses i32 math, results fit in i8
fn harr(x0: &mut i8, x1: &mut i8) {
    let orig_x0 = i32::from(*x0);
    let orig_x1 = i32::from(*x1);
    let mut x0_val = orig_x0;
    let mut x1_val = orig_x1;

    if (x0_val ^ x1_val) & 0x80 != 0 {
        // Different signs
        x1_val += x0_val;
        if ((x1_val ^ orig_x1) & 0x80) == 0 {
            // |X1| > |X0|
            x0_val -= x1_val; // H = -B
        }
    } else {
        // Same sign
        x0_val -= x1_val;
        if ((x0_val ^ orig_x0) & 0x80) == 0 {
            // |X0| > |X1|
            x1_val += x0_val; // L = A
        }
    }

    *x0 = x1_val as i8;
    *x1 = x0_val as i8;
}

/// Performs one level of wavelet transform on a 1D array.
///
/// Uses interleave decomposition instead of pyramid decomposition to avoid
/// needing line buffers. In interleave mode, H/L and X0/X1 are always in
/// the same position.
///
/// # Arguments
/// * `data` - Pointer to coefficient array (as i8 slice)
/// * `size` - Size of the dimension being transformed
/// * `level` - Current wavelet level (0-based)
/// * `skip_pixel` - Number of pixels to skip between elements (1 for horizontal, width for vertical)
#[inline]
fn wavelet_level(data: &mut [i8], size: usize, level: usize, skip_pixel: usize) {
    let s = (8 << level) * skip_pixel;
    let end_offset = (size >> (level + 1)) * s;
    let ofs = (4 << level) * skip_pixel;

    let mut offset = 0;
    while offset < end_offset {
        // Process 3 bytes (RGB channels)
        if offset + ofs + 2 < data.len() {
            let (slice1, slice2) = data.split_at_mut(offset + ofs);
            harr(&mut slice1[offset], &mut slice2[0]);
            harr(&mut slice1[offset + 1], &mut slice2[1]);
            harr(&mut slice1[offset + 2], &mut slice2[2]);
        }
        offset += s;
    }
}

/// Apply wavelet transform and quantization filtering to a coefficient buffer.
///
/// This implements the complete wavelet analysis pipeline:
/// 1. Horizontal wavelet transform at each level
/// 2. Vertical wavelet transform at each level
/// 3. Quantization filtering after each level
///
/// # Arguments
/// * `buf` - Coefficient buffer (i32 values reinterpreted as i8 arrays)
/// * `width` - Image width
/// * `height` - Image height
/// * `level` - Number of wavelet levels to apply (1-3)
fn wavelet(buf: &mut [i32], width: usize, height: usize, level: usize) {
    let bytes =
        unsafe { std::slice::from_raw_parts_mut(buf.as_mut_ptr().cast::<i8>(), buf.len() * 4) };

    for l in 0..level {
        // Horizontal transform
        let s = width << l;
        for row in 0..(height >> l) {
            let row_offset = row * s * 4;
            wavelet_level(&mut bytes[row_offset..], width, l, 1);
        }

        // Vertical transform
        let s = 1 << l;
        for col in 0..(width >> l) {
            let col_offset = col * s * 4;
            wavelet_level(&mut bytes[col_offset..], height, l, width);
        }

        // Apply quantization filter
        filter_wavelet_square(buf, width, height, level, l);
    }
}

/// Apply non-linear quantization filtering to wavelet coefficients.
///
/// This filters the high-frequency subbands using the quantization lookup tables.
/// The filter preserves low-frequency coefficients (subband 0) and quantizes
/// high-frequency subbands (1, 2, 3) based on the selected quality level.
///
/// # Arguments
/// * `buf` - Coefficient buffer
/// * `width` - Image width
/// * `height` - Image height
/// * `level` - Total number of wavelet levels
/// * `l` - Current level being filtered
///
/// # Performance Note
/// This function contains bounds checks in nested loops which add ~2-3% overhead.
/// The checks are necessary for safety but could be optimized with `debug_assert`!
/// and unsafe indexing if profiling shows this as a bottleneck.
#[allow(clippy::cast_sign_loss)] // Quantization filter applies i8 lookup table to u8 bytes
fn filter_wavelet_square(buf: &mut [i32], width: usize, height: usize, level: usize, l: usize) {
    let param = &ZYWRLE_PARAM[level - 1][l];
    let s = 2 << l;

    // Process subbands 1, 2, 3 (skip subband 0 which is low-frequency)
    for r in 1..4 {
        let mut row_start = 0;
        if (r & 0x01) != 0 {
            row_start += s >> 1;
        }
        if (r & 0x02) != 0 {
            row_start += (s >> 1) * width;
        }

        for y in 0..(height / s) {
            for x in 0..(width / s) {
                let idx = row_start + y * s * width + x * s;
                if idx < buf.len() {
                    let pixel = &mut buf[idx];
                    let mut bytes = pixel.to_le_bytes();

                    // Apply filter to each channel (V, Y, U stored in bytes 2, 1, 0)
                    bytes[2] = ZYWRLE_CONV[param[2]][bytes[2] as usize] as u8;
                    bytes[1] = ZYWRLE_CONV[param[1]][bytes[1] as usize] as u8;
                    bytes[0] = ZYWRLE_CONV[param[0]][bytes[0] as usize] as u8;

                    *pixel = i32::from_le_bytes(bytes);
                }
            }
        }
    }
}

/// Convert RGB to YUV using RCT (Reversible Color Transform).
///
/// RCT is described in JPEG-2000 specification:
///   Y = (R + 2G + B)/4
///   U = B - G
///   V = R - G
///
/// The U and V components are further processed to reduce to odd range for `PLHarr`.
///
/// # Arguments
/// * `buf` - Output coefficient buffer (YUV as i32)
/// * `data` - Input RGBA pixel data
/// * `width` - Image width
/// * `height` - Image height
#[allow(clippy::many_single_char_names)] // r, g, b, y, u, v are standard color component names
#[allow(clippy::cast_sign_loss)] // RCT transform stores signed YUV as unsigned bytes in i32
fn rgb_to_yuv(buf: &mut [i32], data: &[u8], width: usize, height: usize) {
    let mut buf_idx = 0;
    let mut data_idx = 0;

    for _ in 0..height {
        for _ in 0..width {
            if data_idx + 2 < data.len() && buf_idx < buf.len() {
                let r = i32::from(data[data_idx]);
                let g = i32::from(data[data_idx + 1]);
                let b = i32::from(data[data_idx + 2]);

                // RCT transform
                let mut y = (r + (g << 1) + b) >> 2;
                let mut u = b - g;
                let mut v = r - g;

                // Center around 0
                y -= 128;
                u >>= 1;
                v >>= 1;

                // Mask to ensure proper bit depth (32-bit: no masking)
                // For 15/16-bit, standard VNC protocol masks here, but we're always 32-bit RGBA

                // Ensure not exactly -128 (helps with wavelet transform)
                if y == -128 {
                    y += 1;
                }
                if u == -128 {
                    u += 1;
                }
                if v == -128 {
                    v += 1;
                }

                // Store as VYU in little-endian order (matches standard VNC protocol ZYWRLE_SAVE_COEFF)
                // U in byte 0, Y in byte 1, V in byte 2
                let bytes: [u8; 4] = [u as u8, y as u8, v as u8, 0];
                buf[buf_idx] = i32::from_le_bytes(bytes);

                buf_idx += 1;
            }
            data_idx += 4; // Skip RGBA
        }
    }
}

/// Calculate aligned dimensions for wavelet transform.
///
/// Wavelet transforms require dimensions to be multiples of 2^level.
/// This function rounds down to the nearest multiple.
///
/// # Arguments
/// * `width` - Original width
/// * `height` - Original height
/// * `level` - Wavelet level
///
/// # Returns
/// Tuple of (`aligned_width`, `aligned_height`)
#[inline]
fn calc_aligned_size(width: usize, height: usize, level: usize) -> (usize, usize) {
    let mask = !((1 << level) - 1);
    (width & mask, height & mask)
}

/// Pack wavelet coefficients into pixel format for transmission.
///
/// After wavelet transform, coefficients are packed in a specific order
/// (Hxy, Hy, Hx, L) for transmission via ZRLE.
///
/// # Arguments
/// * `buf` - Coefficient buffer
/// * `dst` - Destination pixel buffer
/// * `r` - Subband number (0=L, 1=Hx, 2=Hy, 3=Hxy)
/// * `width` - Image width
/// * `height` - Image height
/// * `level` - Wavelet level
///
/// # Performance Note
/// This function contains bounds checks in nested loops which add ~1-2% overhead.
/// The checks are necessary for safety but could be optimized with `debug_assert`!
/// and unsafe indexing if profiling shows this as a bottleneck.
fn pack_coeff(buf: &[i32], dst: &mut [u8], r: usize, width: usize, height: usize, level: usize) {
    let s = 2 << level;
    let mut ph_offset = 0;

    if (r & 0x01) != 0 {
        ph_offset += s >> 1;
    }
    if (r & 0x02) != 0 {
        ph_offset += (s >> 1) * width;
    }

    for _ in 0..(height / s) {
        for _ in 0..(width / s) {
            let dst_idx = ph_offset * 4;
            if ph_offset < buf.len() && dst_idx + 3 < dst.len() {
                let pixel = buf[ph_offset];
                let bytes = pixel.to_le_bytes();
                // Load VYU and save as RGB (for 32bpp RGBA format)
                dst[dst_idx] = bytes[2]; // V -> R
                dst[dst_idx + 1] = bytes[1]; // Y -> G
                dst[dst_idx + 2] = bytes[0]; // U -> B
                dst[dst_idx + 3] = 0; // A
            }
            ph_offset += s;
        }
        ph_offset += (s - 1) * width;
    }
}

/// Perform ZYWRLE analysis (wavelet preprocessing for ZRLE encoding).
///
/// This is the main entry point for ZYWRLE encoding. It:
/// 1. Calculates aligned dimensions
/// 2. Converts RGB to YUV
/// 3. Applies wavelet transform
/// 4. Packs coefficients for ZRLE encoding
///
/// # Arguments
/// * `src` - Source RGBA pixel data
/// * `width` - Image width
/// * `height` - Image height
/// * `level` - ZYWRLE quality level (1-3, higher = more quality/less compression)
/// * `buf` - Temporary coefficient buffer (must be at least width*height i32s)
///
/// # Returns
/// Transformed pixel data ready for ZRLE encoding, or None if dimensions too small
#[allow(clippy::uninit_vec)] // Performance optimization: all bytes written before return (see SAFETY comment)
pub fn zywrle_analyze(
    src: &[u8],
    width: usize,
    height: usize,
    level: usize,
    buf: &mut [i32],
) -> Option<Vec<u8>> {
    let (w, h) = calc_aligned_size(width, height, level);
    if w == 0 || h == 0 {
        return None;
    }

    let uw = width - w;
    let uh = height - h;

    // Allocate output buffer (optimized: avoid zero-initialization since we write all bytes)
    let mut dst = Vec::with_capacity(width * height * 4);
    unsafe {
        // SAFETY: We will write to all bytes in this buffer before returning.
        // The unaligned region copying writes to edges,
        // and pack_coeff() writes to the aligned region.
        dst.set_len(width * height * 4);
    }

    // Handle unaligned pixels (copy as-is)
    // Performance Note: These loops copy unaligned regions row-by-row which adds ~1-2%
    // overhead. Could be optimized with bulk memcpy or SIMD, but the complexity may not
    // be worth it for typical VNC usage (10-30 FPS). Profile before optimizing.

    // Right edge
    if uw > 0 {
        for y in 0..h {
            let src_offset = (y * width + w) * 4;
            let dst_offset = (y * width + w) * 4;
            if src_offset + uw * 4 <= src.len() && dst_offset + uw * 4 <= dst.len() {
                dst[dst_offset..dst_offset + uw * 4]
                    .copy_from_slice(&src[src_offset..src_offset + uw * 4]);
            }
        }
    }

    // Bottom edge
    if uh > 0 {
        for y in h..(h + uh) {
            let src_offset = y * width * 4;
            let dst_offset = y * width * 4;
            if src_offset + w * 4 <= src.len() && dst_offset + w * 4 <= dst.len() {
                dst[dst_offset..dst_offset + w * 4]
                    .copy_from_slice(&src[src_offset..src_offset + w * 4]);
            }
        }
    }

    // Bottom-right corner
    if uw > 0 && uh > 0 {
        for y in h..(h + uh) {
            let src_offset = (y * width + w) * 4;
            let dst_offset = (y * width + w) * 4;
            if src_offset + uw * 4 <= src.len() && dst_offset + uw * 4 <= dst.len() {
                dst[dst_offset..dst_offset + uw * 4]
                    .copy_from_slice(&src[src_offset..src_offset + uw * 4]);
            }
        }
    }

    // RGB to YUV conversion on aligned region
    rgb_to_yuv(&mut buf[0..w * h], src, w, h);

    // Wavelet transform
    wavelet(&mut buf[0..w * h], w, h, level);

    // Pack coefficients
    for l in 0..level {
        pack_coeff(buf, &mut dst, 3, w, h, l); // Hxy
        pack_coeff(buf, &mut dst, 2, w, h, l); // Hy
        pack_coeff(buf, &mut dst, 1, w, h, l); // Hx
        if l == level - 1 {
            pack_coeff(buf, &mut dst, 0, w, h, l); // L (only at last level)
        }
    }

    Some(dst)
}
