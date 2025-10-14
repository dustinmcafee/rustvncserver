//! VNC encoding implementations.
//!
//! This module provides all supported VNC encodings for efficient framebuffer
//! transmission over the network.

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

// Re-export common types
pub use common::*;
