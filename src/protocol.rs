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

//! VNC Remote Framebuffer (RFB) protocol constants and structures.
//!
//! This module provides the fundamental building blocks for VNC protocol communication,
//! including protocol version negotiation, message types, security handshakes, encodings,
//! and pixel format definitions. It implements the RFB protocol as specified in RFC 6143.
//!
//! # Protocol Overview
//!
//! The VNC RFB protocol operates in the following phases:
//! 1. **Protocol Version** - Server and client agree on protocol version
//! 2. **Security Handshake** - Authentication method selection and execution
//! 3. **Initialization** - Exchange of framebuffer parameters and capabilities
//! 4. **Normal Operation** - Ongoing message exchange for input events and screen updates

use bytes::{Buf, BufMut, BytesMut};
use std::io;

/// The RFB protocol version string advertised by the server.
///
/// This server implements RFB protocol version 3.8, which is widely supported
/// by modern VNC clients. The version string must be exactly 12 bytes including
/// the newline character as specified by the RFB protocol.
pub const PROTOCOL_VERSION: &str = "RFB 003.008\n";

// Client-to-Server Message Types

/// Message type: Client requests to change the pixel format.
///
/// This message allows the client to specify its preferred pixel format
/// for receiving framebuffer updates.
pub const CLIENT_MSG_SET_PIXEL_FORMAT: u8 = 0;

/// Message type: Client specifies supported encodings.
///
/// The client sends a list of encoding types it supports, ordered by preference.
/// The server will use the first mutually supported encoding.
pub const CLIENT_MSG_SET_ENCODINGS: u8 = 2;

/// Message type: Client requests a framebuffer update.
///
/// The client can request either an incremental update (changes only) or
/// a full refresh of a specified rectangular region.
pub const CLIENT_MSG_FRAMEBUFFER_UPDATE_REQUEST: u8 = 3;

/// Message type: Client sends a keyboard event.
///
/// Contains information about a key press or release event, including
/// the key symbol and the press/release state.
pub const CLIENT_MSG_KEY_EVENT: u8 = 4;

/// Message type: Client sends a pointer (mouse) event.
///
/// Contains the current pointer position and button state.
pub const CLIENT_MSG_POINTER_EVENT: u8 = 5;

/// Message type: Client sends cut text (clipboard data).
///
/// Allows the client to transfer clipboard contents to the server.
pub const CLIENT_MSG_CLIENT_CUT_TEXT: u8 = 6;

// Server-to-Client Message Types

/// Message type: Server sends a framebuffer update.
///
/// Contains one or more rectangles of pixel data representing screen changes.
/// This is the primary message for transmitting visual updates to the client.
pub const SERVER_MSG_FRAMEBUFFER_UPDATE: u8 = 0;

/// Message type: Server sets colour map entries.
///
/// Used for indexed color modes to define the color palette.
/// Not currently used in this true-color implementation.
#[allow(dead_code)]
pub const SERVER_MSG_SET_COLOUR_MAP_ENTRIES: u8 = 1;

/// Message type: Server sends a bell (beep) notification.
///
/// Signals the client to produce an audible or visual alert.
#[allow(dead_code)]
pub const SERVER_MSG_BELL: u8 = 2;

/// Message type: Server sends cut text (clipboard data).
///
/// Allows the server to transfer clipboard contents to the client.
pub const SERVER_MSG_SERVER_CUT_TEXT: u8 = 3;

// Encoding Types

/// Encoding type: Raw pixel data.
///
/// The simplest encoding that sends uncompressed pixel data directly.
/// High bandwidth but universally supported.
pub const ENCODING_RAW: i32 = 0;

/// Encoding type: Copy Rectangle.
///
/// Instructs the client to copy a rectangular region from one location
/// to another on the screen. Highly efficient for scrolling operations.
pub const ENCODING_COPYRECT: i32 = 1;

/// Encoding type: Rise-and-Run-length Encoding.
///
/// A simple compression scheme for rectangular regions.
pub const ENCODING_RRE: i32 = 2;

/// Encoding type: Compact RRE.
///
/// A more compact version of RRE encoding.
pub const ENCODING_CORRE: i32 = 4;

/// Encoding type: Hextile.
///
/// Divides rectangles into 16x16 tiles for efficient encoding.
pub const ENCODING_HEXTILE: i32 = 5;

/// Encoding type: Zlib compressed.
///
/// Uses zlib compression on raw pixel data.
pub const ENCODING_ZLIB: i32 = 6;

/// Encoding type: Tight.
///
/// A highly efficient encoding using JPEG compression for gradient content
/// and other compression methods for different types of screen content.
pub const ENCODING_TIGHT: i32 = 7;

/// Encoding type: `TightPng`.
///
/// Like Tight encoding but uses PNG compression instead of JPEG.
/// Provides lossless compression for high-quality image transmission.
pub const ENCODING_TIGHTPNG: i32 = -260;

/// Encoding type: `ZlibHex`.
///
/// Zlib-compressed Hextile encoding. Combines Hextile's tile-based encoding
/// with zlib compression for improved bandwidth efficiency.
pub const ENCODING_ZLIBHEX: i32 = 8;

/// Encoding type: Tile Run-Length Encoding.
///
/// An efficient encoding for palettized and run-length compressed data.
#[allow(dead_code)]
pub const ENCODING_TRLE: i32 = 15;

/// Encoding type: Zlib compressed TRLE.
///
/// Combines TRLE with zlib compression.
pub const ENCODING_ZRLE: i32 = 16;

/// Encoding type: ZYWRLE (Zlib+Wavelet+Run-Length Encoding).
///
/// Wavelet-based lossy compression for low-bandwidth scenarios.
/// Uses Piecewise-Linear Haar wavelet transform, RCT (Reversible Color Transform)
/// for RGB to YUV conversion, and non-linear quantization filtering.
/// Shares the ZRLE encoder but applies wavelet preprocessing first.
pub const ENCODING_ZYWRLE: i32 = 17;

/// Encoding type: H.264 video encoding.
///
/// H.264 video compression for very low bandwidth scenarios.
/// Note: This encoding is defined in the RFB protocol but NOT implemented.
/// standard VNC protocol removed H.264 support in v0.9.11 (2016) due to it being
/// broken and unmaintained. This constant exists for protocol compatibility only.
#[allow(dead_code)]
pub const ENCODING_H264: i32 = 0x4832_3634;

/// Pseudo-encoding: Rich Cursor.
///
/// Allows the server to send cursor shape and hotspot information.
#[allow(dead_code)]
pub const ENCODING_CURSOR: i32 = -239;

/// Pseudo-encoding: Desktop Size.
///
/// Notifies the client of framebuffer dimension changes.
#[allow(dead_code)]
pub const ENCODING_DESKTOP_SIZE: i32 = -223;

/// Pseudo-encoding: JPEG Quality Level 0 (lowest quality, highest compression).
///
/// When included in the client's encoding list, this requests the server
/// to use the lowest JPEG quality setting (approximately 10% quality).
pub const ENCODING_QUALITY_LEVEL_0: i32 = -32;

/// Pseudo-encoding: JPEG Quality Level 9 (highest quality, lowest compression).
///
/// When included in the client's encoding list, this requests the server
/// to use the highest JPEG quality setting (approximately 100% quality).
pub const ENCODING_QUALITY_LEVEL_9: i32 = -23;

/// Pseudo-encoding: Compression Level 0 (no compression, fastest).
///
/// Requests the server to use minimal or no compression for encodings
/// that support adjustable compression levels (e.g., Zlib, Tight).
pub const ENCODING_COMPRESS_LEVEL_0: i32 = -256;

/// Pseudo-encoding: Compression Level 9 (maximum compression, slowest).
///
/// Requests the server to use maximum compression, trading CPU time
/// for reduced bandwidth usage.
pub const ENCODING_COMPRESS_LEVEL_9: i32 = -247;

// Hextile subencoding flags

/// Hextile: Raw pixel data for this tile.
pub const HEXTILE_RAW: u8 = 1 << 0;

/// Hextile: Background color is specified.
pub const HEXTILE_BACKGROUND_SPECIFIED: u8 = 1 << 1;

/// Hextile: Foreground color is specified.
pub const HEXTILE_FOREGROUND_SPECIFIED: u8 = 1 << 2;

/// Hextile: Tile contains subrectangles.
pub const HEXTILE_ANY_SUBRECTS: u8 = 1 << 3;

/// Hextile: Subrectangles are colored (not monochrome).
pub const HEXTILE_SUBRECTS_COLOURED: u8 = 1 << 4;

// Tight subencoding types

/// Tight/TightPng: PNG compression subencoding.
pub const TIGHT_PNG: u8 = 0x0A;

// Security Types

/// Security type: Invalid/Unknown.
///
/// Indicates an error or unsupported security mechanism.
#[allow(dead_code)]
pub const SECURITY_TYPE_INVALID: u8 = 0;

/// Security type: None (no authentication).
///
/// No authentication is required. The connection proceeds directly
/// to the initialization phase.
pub const SECURITY_TYPE_NONE: u8 = 1;

/// Security type: VNC Authentication.
///
/// Standard VNC authentication using DES-encrypted challenge-response.
/// The server sends a 16-byte challenge, which the client encrypts with
/// the password and returns.
pub const SECURITY_TYPE_VNC_AUTH: u8 = 2;

// Security Results

/// Security result: Authentication successful.
///
/// Sent by the server to indicate that authentication (if any) succeeded.
pub const SECURITY_RESULT_OK: u32 = 0;

/// Security result: Authentication failed.
///
/// Sent by the server to indicate that authentication failed.
pub const SECURITY_RESULT_FAILED: u32 = 1;

/// Represents the pixel format of the VNC framebuffer.
///
/// This struct defines how pixel data is interpreted, including color depth,
/// endianness, and RGB component details.
#[derive(Debug, Clone)]
pub struct PixelFormat {
    /// Number of bits per pixel.
    pub bits_per_pixel: u8,
    /// Depth of the pixel in bits.
    pub depth: u8,
    /// Flag indicating if the pixel data is big-endian (1) or little-endian (0).
    pub big_endian_flag: u8,
    /// Flag indicating if the pixel format is true-colour (1) or colormapped (0).
    pub true_colour_flag: u8,
    /// Maximum red color value.
    pub red_max: u16,
    /// Maximum green color value.
    pub green_max: u16,
    /// Maximum blue color value.
    pub blue_max: u16,
    /// Number of shifts to apply to get the red color component.
    pub red_shift: u8,
    /// Number of shifts to apply to get the green color component.
    pub green_shift: u8,
    /// Number of shifts to apply to get the blue color component.
    pub blue_shift: u8,
}

impl PixelFormat {
    /// Creates a standard 32-bit RGBA pixel format.
    ///
    /// # Returns
    ///
    /// A `PixelFormat` instance configured for 32-bit RGBA.
    #[must_use]
    pub fn rgba32() -> Self {
        Self {
            bits_per_pixel: 32,
            depth: 24,
            big_endian_flag: 0,
            true_colour_flag: 1,
            red_max: 255,
            green_max: 255,
            blue_max: 255,
            red_shift: 0,
            green_shift: 8,
            blue_shift: 16,
        }
    }

    /// Checks if this `PixelFormat` is compatible with the standard 32-bit RGBA format.
    ///
    /// # Returns
    ///
    /// `true` if the pixel format matches 32-bit RGBA, `false` otherwise.
    #[must_use]
    pub fn is_compatible_with_rgba32(&self) -> bool {
        self.bits_per_pixel == 32
            && self.depth == 24
            && self.big_endian_flag == 0
            && self.true_colour_flag == 1
            && self.red_max == 255
            && self.green_max == 255
            && self.blue_max == 255
            && self.red_shift == 0
            && self.green_shift == 8
            && self.blue_shift == 16
    }

    /// Validates that this pixel format is supported by the server.
    ///
    /// Checks that the format uses valid bits-per-pixel values and is either
    /// true-color or a supported color-mapped format.
    ///
    /// # Returns
    ///
    /// `true` if the format is valid and supported, `false` otherwise.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        // Check bits per pixel is valid
        if self.bits_per_pixel != 8
            && self.bits_per_pixel != 16
            && self.bits_per_pixel != 24
            && self.bits_per_pixel != 32
        {
            return false;
        }

        // Check depth is reasonable
        if self.depth == 0 || self.depth > 32 {
            return false;
        }

        // For non-truecolor (color-mapped), only 8bpp is supported
        if self.true_colour_flag == 0 && self.bits_per_pixel != 8 {
            return false;
        }

        // For truecolor, validate color component ranges
        if self.true_colour_flag != 0 {
            // Check that max values fit in the bit depth
            #[allow(clippy::cast_possible_truncation)]
            // leading_zeros() returns max 32, result always fits in u8
            let bits_needed = |max: u16| -> u8 {
                if max == 0 {
                    0
                } else {
                    (16 - max.leading_zeros()) as u8
                }
            };

            let red_bits = bits_needed(self.red_max);
            let green_bits = bits_needed(self.green_max);
            let blue_bits = bits_needed(self.blue_max);

            // Total bits should not exceed depth
            if red_bits + green_bits + blue_bits > self.depth {
                return false;
            }

            // Shifts should not cause overlap or exceed bit depth
            if self.red_shift >= 32 || self.green_shift >= 32 || self.blue_shift >= 32 {
                return false;
            }
        }

        true
    }

    /// Creates a 16-bit RGB565 pixel format.
    ///
    /// RGB565 uses 5 bits for red, 6 bits for green, and 5 bits for blue.
    /// This is a common format for embedded displays and bandwidth-constrained clients.
    ///
    /// # Returns
    ///
    /// A `PixelFormat` instance configured for 16-bit RGB565.
    #[allow(dead_code)]
    #[must_use]
    pub fn rgb565() -> Self {
        Self {
            bits_per_pixel: 16,
            depth: 16,
            big_endian_flag: 0,
            true_colour_flag: 1,
            red_max: 31,   // 5 bits
            green_max: 63, // 6 bits
            blue_max: 31,  // 5 bits
            red_shift: 11,
            green_shift: 5,
            blue_shift: 0,
        }
    }

    /// Creates a 16-bit RGB555 pixel format.
    ///
    /// RGB555 uses 5 bits for each of red, green, and blue, with 1 unused bit.
    ///
    /// # Returns
    ///
    /// A `PixelFormat` instance configured for 16-bit RGB555.
    #[allow(dead_code)]
    #[must_use]
    pub fn rgb555() -> Self {
        Self {
            bits_per_pixel: 16,
            depth: 15,
            big_endian_flag: 0,
            true_colour_flag: 1,
            red_max: 31,   // 5 bits
            green_max: 31, // 5 bits
            blue_max: 31,  // 5 bits
            red_shift: 10,
            green_shift: 5,
            blue_shift: 0,
        }
    }

    /// Creates an 8-bit BGR233 pixel format.
    ///
    /// BGR233 uses 2 bits for blue, 3 bits for green, and 3 bits for red.
    /// This format is used for very low bandwidth connections and legacy clients.
    ///
    /// # Returns
    ///
    /// A `PixelFormat` instance configured for 8-bit BGR233.
    #[allow(dead_code)]
    #[must_use]
    pub fn bgr233() -> Self {
        Self {
            bits_per_pixel: 8,
            depth: 8,
            big_endian_flag: 0,
            true_colour_flag: 1,
            red_max: 7,   // 3 bits
            green_max: 7, // 3 bits
            blue_max: 3,  // 2 bits
            red_shift: 0,
            green_shift: 3,
            blue_shift: 6,
        }
    }

    /// Writes the pixel format data into a `BytesMut` buffer.
    ///
    /// This function serializes the `PixelFormat` into the RFB protocol format.
    ///
    /// # Arguments
    ///
    /// * `buf` - A mutable reference to the `BytesMut` buffer to write into.
    pub fn write_to(&self, buf: &mut BytesMut) {
        buf.put_u8(self.bits_per_pixel);
        buf.put_u8(self.depth);
        buf.put_u8(self.big_endian_flag);
        buf.put_u8(self.true_colour_flag);
        buf.put_u16(self.red_max);
        buf.put_u16(self.green_max);
        buf.put_u16(self.blue_max);
        buf.put_u8(self.red_shift);
        buf.put_u8(self.green_shift);
        buf.put_u8(self.blue_shift);
        buf.put_bytes(0, 3); // padding
    }

    /// Reads and deserializes a `PixelFormat` from a `BytesMut` buffer.
    ///
    /// This function extracts pixel format information from the RFB protocol stream.
    ///
    /// # Arguments
    ///
    /// * `buf` - A mutable reference to the `BytesMut` buffer to read from.
    ///
    /// # Returns
    ///
    /// `Ok(Self)` containing the parsed `PixelFormat`.
    ///
    /// # Errors
    ///
    /// Returns `Err(io::Error)` if there are not enough bytes in the buffer
    /// to read a complete `PixelFormat`.
    pub fn from_bytes(buf: &mut BytesMut) -> io::Result<Self> {
        if buf.len() < 16 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "Not enough bytes for PixelFormat",
            ));
        }

        let pf = Self {
            bits_per_pixel: buf.get_u8(),
            depth: buf.get_u8(),
            big_endian_flag: buf.get_u8(),
            true_colour_flag: buf.get_u8(),
            red_max: buf.get_u16(),
            green_max: buf.get_u16(),
            blue_max: buf.get_u16(),
            red_shift: buf.get_u8(),
            green_shift: buf.get_u8(),
            blue_shift: buf.get_u8(),
        };
        buf.advance(3);
        Ok(pf)
    }
}

/// Represents the `ServerInit` message sent during VNC initialization.
///
/// This message is sent by the server after security negotiation is complete.
/// It provides the client with framebuffer dimensions, pixel format, and
/// the desktop name.
#[derive(Debug, Clone)]
pub struct ServerInit {
    /// The width of the framebuffer in pixels.
    pub framebuffer_width: u16,
    /// The height of the framebuffer in pixels.
    pub framebuffer_height: u16,
    /// The pixel format used by the framebuffer.
    pub pixel_format: PixelFormat,
    /// The name of the desktop (e.g., "Android VNC Server").
    pub name: String,
}

impl ServerInit {
    /// Serializes the `ServerInit` message into a byte buffer.
    ///
    /// The format follows the RFB protocol specification:
    /// - 2 bytes: framebuffer width
    /// - 2 bytes: framebuffer height
    /// - 16 bytes: pixel format
    /// - 4 bytes: name length
    /// - N bytes: name string (UTF-8)
    ///
    /// # Arguments
    ///
    /// * `buf` - The buffer to write the serialized message into.
    #[allow(clippy::cast_possible_truncation)] // Desktop name length limited to u32 per VNC protocol
    pub fn write_to(&self, buf: &mut BytesMut) {
        buf.put_u16(self.framebuffer_width);
        buf.put_u16(self.framebuffer_height);
        self.pixel_format.write_to(buf);

        let name_bytes = self.name.as_bytes();
        buf.put_u32(name_bytes.len() as u32);
        buf.put_slice(name_bytes);
    }
}

/// Represents all possible message types that can be sent from a VNC client to the server.
///
/// This enum encapsulates the various client messages defined in the RFB protocol,
/// making it easier to handle client input in a type-safe manner.
#[allow(dead_code)]
#[derive(Debug)]
pub enum ClientMessage {
    /// Client requests a specific pixel format for framebuffer updates.
    SetPixelFormat(PixelFormat),

    /// Client specifies the list of encodings it supports.
    SetEncodings(Vec<i32>),

    /// Client requests a framebuffer update for a specific region.
    FramebufferUpdateRequest {
        /// If true, only send changes since the last update; if false, send full refresh.
        incremental: bool,
        /// X coordinate of the requested region.
        x: u16,
        /// Y coordinate of the requested region.
        y: u16,
        /// Width of the requested region.
        width: u16,
        /// Height of the requested region.
        height: u16,
    },

    /// Client sends a keyboard key event.
    KeyEvent {
        /// True if the key is pressed, false if released.
        down: bool,
        /// The X Window System keysym value of the key.
        key: u32,
    },

    /// Client sends a pointer (mouse) event.
    PointerEvent {
        /// Bitmask of currently pressed mouse buttons.
        button_mask: u8,
        /// X coordinate of the pointer.
        x: u16,
        /// Y coordinate of the pointer.
        y: u16,
    },

    /// Client sends clipboard (cut text) data.
    ClientCutText(String),
}

/// Represents a rectangle header in a framebuffer update message.
///
/// Each framebuffer update can contain multiple rectangles, each with its own
/// encoding type. The rectangle header specifies the position, dimensions,
/// and encoding of the pixel data that follows.
#[derive(Debug)]
pub struct Rectangle {
    /// X coordinate of the top-left corner.
    pub x: u16,
    /// Y coordinate of the top-left corner.
    pub y: u16,
    /// Width of the rectangle in pixels.
    pub width: u16,
    /// Height of the rectangle in pixels.
    pub height: u16,
    /// The encoding type used for this rectangle's pixel data.
    pub encoding: i32,
}

impl Rectangle {
    /// Writes the rectangle header to a byte buffer.
    ///
    /// The header format is:
    /// - 2 bytes: x position
    /// - 2 bytes: y position
    /// - 2 bytes: width
    /// - 2 bytes: height
    /// - 4 bytes: encoding type (signed 32-bit integer)
    ///
    /// # Arguments
    ///
    /// * `buf` - The buffer to write the header into.
    pub fn write_header(&self, buf: &mut BytesMut) {
        buf.put_u16(self.x);
        buf.put_u16(self.y);
        buf.put_u16(self.width);
        buf.put_u16(self.height);
        buf.put_i32(self.encoding);
    }
}
