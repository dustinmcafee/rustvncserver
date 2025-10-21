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

//! FFI bindings to libjpeg-turbo's `TurboJPEG` API.
//!
//! This module provides a safe Rust wrapper around the `TurboJPEG` C API
//! for high-performance JPEG compression.

use std::ffi::c_void;
use std::os::raw::{c_char, c_int, c_uchar, c_ulong};

// TurboJPEG pixel format constants
/// RGB pixel format (red, green, blue)
pub const TJPF_RGB: c_int = 0;
/// BGR pixel format (blue, green, red)
#[allow(dead_code)]
pub const TJPF_BGR: c_int = 1;
/// RGBX pixel format (red, green, blue, unused)
#[allow(dead_code)]
pub const TJPF_RGBX: c_int = 2;
/// BGRX pixel format (blue, green, red, unused)
#[allow(dead_code)]
pub const TJPF_BGRX: c_int = 3;
/// XBGR pixel format (unused, blue, green, red)
#[allow(dead_code)]
pub const TJPF_XBGR: c_int = 4;
/// XRGB pixel format (unused, red, green, blue)
#[allow(dead_code)]
pub const TJPF_XRGB: c_int = 5;
/// Grayscale pixel format
#[allow(dead_code)]
pub const TJPF_GRAY: c_int = 6;

// TurboJPEG chrominance subsampling constants
/// 4:4:4 chrominance subsampling (no subsampling)
#[allow(dead_code)]
pub const TJSAMP_444: c_int = 0;
/// 4:2:2 chrominance subsampling (2x1 subsampling)
pub const TJSAMP_422: c_int = 1;
/// 4:2:0 chrominance subsampling (2x2 subsampling)
#[allow(dead_code)]
pub const TJSAMP_420: c_int = 2;
/// Grayscale (no chrominance)
#[allow(dead_code)]
pub const TJSAMP_GRAY: c_int = 3;

// Opaque TurboJPEG handle
type TjHandle = *mut c_void;

// External C functions from libjpeg-turbo
#[link(name = "turbojpeg")]
extern "C" {
    fn tjInitCompress() -> TjHandle;
    fn tjDestroy(handle: TjHandle) -> c_int;
    fn tjCompress2(
        handle: TjHandle,
        src_buf: *const c_uchar,
        width: c_int,
        pitch: c_int,
        height: c_int,
        pixel_format: c_int,
        jpeg_buf: *mut *mut c_uchar,
        jpeg_size: *mut c_ulong,
        jpeg_subsamp: c_int,
        jpeg_qual: c_int,
        flags: c_int,
    ) -> c_int;
    fn tjFree(buffer: *mut c_uchar);
    fn tjGetErrorStr2(handle: TjHandle) -> *const c_char;
}

/// Safe Rust wrapper for `TurboJPEG` compression.
pub struct TurboJpegEncoder {
    handle: TjHandle,
}

impl TurboJpegEncoder {
    /// Creates a new `TurboJPEG` encoder.
    pub fn new() -> Result<Self, String> {
        let handle = unsafe { tjInitCompress() };
        if handle.is_null() {
            return Err("Failed to initialize TurboJPEG compressor".to_string());
        }
        Ok(Self { handle })
    }

    /// Compresses RGB image data to JPEG format.
    ///
    /// # Arguments
    /// * `rgb_data` - RGB pixel data (3 bytes per pixel)
    /// * `width` - Image width in pixels
    /// * `height` - Image height in pixels
    /// * `quality` - JPEG quality (1-100, where 100 is best quality)
    ///
    /// # Returns
    /// JPEG-compressed data as a `Vec<u8>`
    pub fn compress_rgb(
        &mut self,
        rgb_data: &[u8],
        width: u16,
        height: u16,
        quality: u8,
    ) -> Result<Vec<u8>, String> {
        let expected_size = (width as usize) * (height as usize) * 3;
        if rgb_data.len() != expected_size {
            return Err(format!(
                "Invalid RGB data size: expected {}, got {}",
                expected_size,
                rgb_data.len()
            ));
        }

        let mut jpeg_buf: *mut c_uchar = std::ptr::null_mut();
        let mut jpeg_size: c_ulong = 0;

        let result = unsafe {
            tjCompress2(
                self.handle,
                rgb_data.as_ptr(),
                c_int::from(width),
                0, // pitch = 0 means width * pixel_size
                c_int::from(height),
                TJPF_RGB,
                &mut jpeg_buf,
                &mut jpeg_size,
                TJSAMP_422, // 4:2:2 subsampling for good quality/size balance
                c_int::from(quality),
                0, // flags
            )
        };

        if result != 0 {
            let error_msg = self.get_error_string();
            return Err(format!("TurboJPEG compression failed: {error_msg}"));
        }

        if jpeg_buf.is_null() {
            return Err("TurboJPEG returned null buffer".to_string());
        }

        // Copy JPEG data to Rust Vec
        let jpeg_data =
            unsafe { std::slice::from_raw_parts(jpeg_buf, jpeg_size as usize).to_vec() };

        // Free TurboJPEG buffer
        unsafe {
            tjFree(jpeg_buf);
        }

        Ok(jpeg_data)
    }

    /// Gets the last error message from `TurboJPEG`.
    fn get_error_string(&self) -> String {
        unsafe {
            let c_str = tjGetErrorStr2(self.handle);
            if c_str.is_null() {
                return "Unknown error".to_string();
            }
            std::ffi::CStr::from_ptr(c_str)
                .to_string_lossy()
                .into_owned()
        }
    }
}

impl Drop for TurboJpegEncoder {
    fn drop(&mut self) {
        unsafe {
            tjDestroy(self.handle);
        }
    }
}

unsafe impl Send for TurboJpegEncoder {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encoder_creation() {
        let encoder = TurboJpegEncoder::new();
        assert!(encoder.is_ok());
    }

    #[test]
    fn test_compress_rgb() {
        let mut encoder = TurboJpegEncoder::new().unwrap();

        // Create a simple 2x2 red image
        let rgb_data = vec![255, 0, 0, 255, 0, 0, 255, 0, 0, 255, 0, 0];

        let result = encoder.compress_rgb(&rgb_data, 2, 2, 90);
        assert!(result.is_ok());

        let jpeg_data = result.unwrap();
        assert!(!jpeg_data.is_empty());
        // JPEG files start with 0xFF 0xD8
        assert_eq!(jpeg_data[0], 0xFF);
        assert_eq!(jpeg_data[1], 0xD8);
    }
}
