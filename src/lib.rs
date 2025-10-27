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
//! - **11 encoding types**: Raw, `CopyRect`, RRE, `CoRRE`, Hextile, Zlib, `ZlibHex`,
//!   Tight, `TightPng`, ZRLE, ZYWRLE
//! - **All pixel formats**: 8/16/24/32-bit color depths
//! - **Tight encoding**: All 5 production modes (solid fill, mono rect, indexed
//!   palette, full-color zlib, JPEG)
//! - **Async I/O**: Built on Tokio for efficient concurrent client handling
//! - **Memory safe**: Pure Rust with zero unsafe code in core logic
//! - **Optional `TurboJPEG`**: Hardware-accelerated JPEG compression via feature flag
//!
//! ## Quick Start
//!
//! ```no_run
//! use rustvncserver::VncServer;
//! use rustvncserver::server::ServerEvent;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // Create a VNC server with 1920x1080 framebuffer
//!     let (server, mut event_rx) = VncServer::new(
//!         1920,
//!         1080,
//!         "My Desktop".to_string(),
//!         Some("secret".to_string()), // Optional password
//!     );
//!
//!     // Handle events from clients in a background task
//!     tokio::spawn(async move {
//!         while let Some(event) = event_rx.recv().await {
//!             match event {
//!                 ServerEvent::ClientConnected { client_id } => {
//!                     println!("Client {} connected", client_id);
//!                 }
//!                 ServerEvent::ClientDisconnected { client_id } => {
//!                     println!("Client {} disconnected", client_id);
//!                 }
//!                 _ => {}
//!             }
//!         }
//!     });
//!
//!     // Start listening on port 5900 (blocks until server stops)
//!     server.listen(5900).await?;
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

#![deny(missing_docs)]
#![warn(clippy::all)]
#![warn(clippy::pedantic)]

pub mod error;
pub mod events;
pub mod framebuffer;
pub mod protocol;
pub mod server;

// Internal modules
mod auth;
mod client;
mod repeater;

// Re-export encodings from rfb-encodings crate
pub use rfb_encodings as encoding;

// Re-exports
pub use encoding::Encoding;
pub use error::{Result, VncError};
pub use events::ServerEvent;
pub use framebuffer::Framebuffer;
pub use protocol::PixelFormat;
pub use server::VncServer;

#[cfg(feature = "turbojpeg")]
pub use encoding::jpeg::TurboJpegEncoder;

/// VNC protocol version.
pub const PROTOCOL_VERSION: &str = "RFB 003.008\n";

/// Default VNC port.
pub const DEFAULT_PORT: u16 = 5900;
