//! VNC framebuffer encoding implementations.
//!
//! This module provides encoding strategies for transmitting framebuffer data efficiently
//! over the VNC protocol. Different encodings offer trade-offs between bandwidth usage,
//! CPU overhead, and image quality.
//!
//! # Supported Encodings
//!
//! - **Raw**: Uncompressed pixel data. Simple but bandwidth-intensive.
//! - **RRE**: Rise-and-Run-length Encoding for solid color regions.
//! - **CoRRE**: Compact RRE encoding for smaller rectangles.
//! - **Hextile**: Divides rectangles into 16x16 tiles with intelligent compression.
//! - **Zlib**: Simple zlib compression on raw pixel data.
//! - **Tight**: JPEG-compressed pixel data with palette and zlib modes.
//! - **ZRLE**: Zlib Run-Length Encoding with 64x64 tiles and CPIXEL format.
//!
//! # Architecture
//!
//! The module uses a trait-based design allowing easy addition of new encodings.
//! Each encoding implements the `Encoding` trait which defines how to transform
//! raw RGBA pixel data into the encoded format.

use bytes::BytesMut;
use crate::vnc::protocol::*;

// Module declarations
pub mod common;
pub mod raw;
pub mod rre;
pub mod corre;
pub mod hextile;
pub mod zlib;
pub mod zlibhex;
pub mod tight;
pub mod tightpng;
pub mod zrle;
pub mod zywrle;

// Re-export encoding implementations
pub use raw::RawEncoding;
pub use rre::RreEncoding;
pub use corre::CorRreEncoding;
pub use hextile::HextileEncoding;
pub use tight::TightEncoding;
pub use tightpng::TightPngEncoding;

// Re-export persistent encoding functions (RFC 6143 compliant)
pub use zlib::encode_zlib_persistent;
pub use zlibhex::encode_zlibhex_persistent;
pub use zrle::encode_zrle_persistent;

// Re-export ZYWRLE analysis function
pub use zywrle::zywrle_analyze;

/// A trait defining the interface for VNC encoding implementations.
///
/// This trait defines the interface for different VNC encodings, allowing them to transform
/// raw pixel data into a byte stream suitable for transmission over a VNC connection.
pub trait Encoding {
    /// Encodes raw pixel data into a VNC-compatible byte stream.
    ///
    /// # Arguments
    ///
    /// * `data` - A slice containing the raw pixel data (RGBA format: 4 bytes per pixel).
    /// * `width` - The width of the framebuffer.
    /// * `height` - The height of the framebuffer.
    /// * `quality` - The quality level for lossy encodings (0-100).
    /// * `compression` - The compression level for encodings that support it (0-9).
    ///
    /// # Returns
    ///
    /// A `BytesMut` containing the encoded data.
    fn encode(&self, data: &[u8], width: u16, height: u16, quality: u8, compression: u8) -> BytesMut;
}

/// Creates an encoder instance for the specified encoding type.
///
/// This factory function returns a boxed trait object implementing the `Encoding` trait
/// for the requested encoding type. It allows dynamic selection of encodings at runtime
/// based on client capabilities.
///
/// # Arguments
///
/// * `encoding_type` - The RFB encoding type constant.
///
/// # Returns
///
/// `Some(Box<dyn Encoding>)` if the encoding type is supported, `None` otherwise.
pub fn get_encoder(encoding_type: i32) -> Option<Box<dyn Encoding>> {
    match encoding_type {
        ENCODING_RAW => Some(Box::new(RawEncoding)),
        ENCODING_RRE => Some(Box::new(RreEncoding)),
        ENCODING_CORRE => Some(Box::new(CorRreEncoding)),
        ENCODING_HEXTILE => Some(Box::new(HextileEncoding)),
        ENCODING_TIGHT => Some(Box::new(TightEncoding)),
        ENCODING_TIGHTPNG => Some(Box::new(TightPngEncoding)),
        // ZLIB, ZLIBHEX, and ZRLE use persistent compressors, handled directly in client.rs
        _ => None,
    }
}
