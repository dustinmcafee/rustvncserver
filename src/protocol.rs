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

use bytes::{BufMut, BytesMut};

// Re-export PixelFormat from rfb-encodings
pub use rfb_encodings::PixelFormat;

// Re-export encoding constants from rfb-encodings
pub use rfb_encodings::{
    // Encoding types
    ENCODING_CORRE,
    ENCODING_HEXTILE,
    ENCODING_RAW,
    ENCODING_RRE,
    ENCODING_TIGHT,
    ENCODING_TIGHTPNG,
    ENCODING_ZLIB,
    ENCODING_ZLIBHEX,
    ENCODING_ZRLE,
    ENCODING_ZYWRLE,
    // Hextile subencoding flags
    HEXTILE_ANY_SUBRECTS,
    HEXTILE_BACKGROUND_SPECIFIED,
    HEXTILE_FOREGROUND_SPECIFIED,
    HEXTILE_RAW,
    HEXTILE_SUBRECTS_COLOURED,
    // Tight subencoding types
    TIGHT_PNG,
};

/// The RFB protocol version string advertised by the server.
///
/// This server implements RFB protocol version 3.8, which is widely supported
/// by modern VNC clients. The version string must be exactly 12 bytes including
/// the newline character as specified by the RFB protocol.
pub const PROTOCOL_VERSION: &str = "RFB 003.008\n";

/// Maximum framebuffer update buffer size in bytes (32KB).
///
/// This limit matches the reference VNC implementation and helps prevent
/// overwhelming clients or network infrastructure with very large updates.
/// When an update would exceed this size, it should be split into multiple
/// `FramebufferUpdate` messages.
pub const UPDATE_BUF_SIZE: usize = 32768;

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
//
// Note: Most encoding type constants are re-exported from rfb-encodings at the top of this file.
// Only server-specific encodings and pseudo-encodings are defined here.

/// Encoding type: Copy Rectangle.
///
/// Instructs the client to copy a rectangular region from one location
/// to another on the screen. Highly efficient for scrolling operations.
/// This is a server-side operation, not a data encoding format.
pub const ENCODING_COPYRECT: i32 = 1;

/// Encoding type: Tile Run-Length Encoding.
///
/// An efficient encoding for palettized and run-length compressed data.
/// Note: Not currently implemented in rfb-encodings.
#[allow(dead_code)]
pub const ENCODING_TRLE: i32 = 15;

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

// Note: Hextile and Tight subencoding constants are re-exported from rfb-encodings
// at the top of this file.

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
        // VNC protocol requires big-endian (network byte order) for all multi-byte integers
        #[cfg_attr(not(feature = "debug-logging"), allow(unused_variables))]
        let start_len = buf.len();
        buf.put_u16(self.x);
        buf.put_u16(self.y);
        buf.put_u16(self.width);
        buf.put_u16(self.height);
        buf.put_i32(self.encoding);

        #[cfg(feature = "debug-logging")]
        {
            let header_bytes = &buf[start_len..];
            log::info!("Rectangle header bytes: x={} y={} w={} h={} enc={} -> [{:02x} {:02x}] [{:02x} {:02x}] [{:02x} {:02x}] [{:02x} {:02x}] [{:02x} {:02x} {:02x} {:02x}]",
                self.x, self.y, self.width, self.height, self.encoding,
                header_bytes[0], header_bytes[1],  // x
                header_bytes[2], header_bytes[3],  // y
                header_bytes[4], header_bytes[5],  // width
                header_bytes[6], header_bytes[7],  // height
                header_bytes[8], header_bytes[9], header_bytes[10], header_bytes[11]  // encoding
            );
        }
    }
}
