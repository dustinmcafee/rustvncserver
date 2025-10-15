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


//! # rustvncserver
//!
//! A pure Rust implementation of a VNC (Virtual Network Computing) server.
//!
//! This library provides a complete VNC server implementation following the RFB
//! (Remote Framebuffer) protocol specification (RFC 6143). It supports all major
//! VNC encodings and pixel formats, with 100% wire-format compatibility with
//! standard VNC protocol.
//!
//! ## Features
//!
//! - **11 encoding types**: Raw, CopyRect, RRE, CoRRE, Hextile, Zlib, ZlibHex,
//!   Tight, TightPng, ZRLE, ZYWRLE
//! - **All pixel formats**: 8/16/24/32-bit color depths
//! - **Tight encoding**: All 5 production modes (solid fill, mono rect, indexed
//!   palette, full-color zlib, JPEG)
//! - **Async I/O**: Built on Tokio for efficient concurrent client handling
//! - **Memory safe**: Pure Rust with zero unsafe code in core logic
//! - **Optional TurboJPEG**: Hardware-accelerated JPEG compression via feature flag
//!
//! ## Quick Start
//!
//! ```no_run
//! use rustvncserver::{VncServer, ServerEvent};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // Create a VNC server with 1920x1080 framebuffer
//!     let server = VncServer::new(1920, 1080);
//!
//!     // Optional: Set a password
//!     server.set_password(Some("secret".to_string()));
//!
//!     // Start listening on port 5900
//!     let server_handle = tokio::spawn(async move {
//!         server.listen(5900).await
//!     });
//!
//!     // Update the framebuffer
//!     // server.update_framebuffer(&pixels, 0, 0, 1920, 1080);
//!
//!     server_handle.await??;
//!     Ok(())
//! }
//! ```
//!
//! ## Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────┐
//! │           Your Application              │
//! │                                         │
//! │  • Provide framebuffer data             │
//! │  • Receive input events                 │
//! │  • Control server lifecycle             │
//! └──────────────────┬──────────────────────┘
//!                    │
//!                    ▼
//! ┌─────────────────────────────────────────┐
//! │           VncServer (Public)            │
//! │                                         │
//! │  • TCP listener                         │
//! │  • Client management                    │
//! │  • Event distribution                   │
//! └──────────────────┬──────────────────────┘
//!                    │
//!        ┌───────────┼───────────┐
//!        ▼           ▼           ▼
//!   ┌────────┐ ┌────────┐ ┌────────┐
//!   │Client 1│ │Client 2│ │Client N│
//!   └────────┘ └────────┘ └────────┘
//!        │           │           │
//!        └───────────┴───────────┘
//!                    │
//!                    ▼
//! ┌─────────────────────────────────────────┐
//! │      Framebuffer (Thread-safe)          │
//! │                                         │
//! │  • RGBA32 pixel storage                 │
//! │  • Region tracking                      │
//! │  • CopyRect operations                  │
//! └─────────────────────────────────────────┘
//! ```

#![warn(missing_docs)]
#![warn(clippy::all)]

pub mod error;
pub mod events;
pub mod protocol;
pub mod server;
pub mod framebuffer;

// Internal modules
mod client;
mod auth;
mod repeater;
mod translate;
pub mod encoding;
pub mod jpeg;

// Re-exports
pub use error::{VncError, Result};
pub use events::ServerEvent;
pub use server::VncServer;
pub use framebuffer::Framebuffer;
pub use protocol::PixelFormat;
pub use encoding::Encoding;

#[cfg(feature = "turbojpeg")]
pub use jpeg::TurboJpegEncoder;

/// VNC protocol version.
pub const PROTOCOL_VERSION: &str = "RFB 003.008\n";

/// Default VNC port.
pub const DEFAULT_PORT: u16 = 5900;
