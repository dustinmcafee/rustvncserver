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

//! VNC encoding implementations.
//!
//! This module provides all supported VNC encodings for efficient framebuffer
//! transmission over the network.

use crate::protocol::{ENCODING_CORRE, ENCODING_HEXTILE, ENCODING_RAW, ENCODING_RRE, ENCODING_TIGHT, ENCODING_TIGHTPNG};
use bytes::BytesMut;

pub mod common;
pub mod corre;
pub mod hextile;
pub mod raw;
pub mod rre;
pub mod tight;
pub mod tightpng;
pub mod zlib;
pub mod zlibhex;
pub mod zrle;
pub mod zywrle;

// Re-export common types
pub use common::*;

// Re-export encoding implementations
pub use corre::CorRreEncoding;
pub use hextile::HextileEncoding;
pub use raw::RawEncoding;
pub use rre::RreEncoding;
pub use tight::TightEncoding;
pub use tightpng::TightPngEncoding;

// Re-export persistent encoding functions
pub use zlib::encode_zlib_persistent;
pub use zlibhex::encode_zlibhex_persistent;
pub use zrle::encode_zrle_persistent;

// Re-export ZYWRLE analysis function
pub use zywrle::zywrle_analyze;

/// Trait defining the interface for VNC encoding implementations.
pub trait Encoding {
    /// Encodes raw pixel data into a VNC-compatible byte stream.
    ///
    /// # Arguments
    ///
    /// * `data` - Raw pixel data (RGBA format: 4 bytes per pixel)
    /// * `width` - Width of the framebuffer
    /// * `height` - Height of the framebuffer
    /// * `quality` - Quality level for lossy encodings (0-100)
    /// * `compression` - Compression level (0-9)
    ///
    /// # Returns
    ///
    /// Encoded data as `BytesMut`
    fn encode(
        &self,
        data: &[u8],
        width: u16,
        height: u16,
        quality: u8,
        compression: u8,
    ) -> BytesMut;
}

/// Creates an encoder instance for the specified encoding type.
#[must_use] pub fn get_encoder(encoding_type: i32) -> Option<Box<dyn Encoding>> {
    match encoding_type {
        ENCODING_RAW => Some(Box::new(RawEncoding)),
        ENCODING_RRE => Some(Box::new(RreEncoding)),
        ENCODING_CORRE => Some(Box::new(CorRreEncoding)),
        ENCODING_HEXTILE => Some(Box::new(HextileEncoding)),
        ENCODING_TIGHT => Some(Box::new(TightEncoding)),
        ENCODING_TIGHTPNG => Some(Box::new(TightPngEncoding)),
        _ => None,
    }
}
