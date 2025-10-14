//! JPEG encoding support for Tight encoding.
//!
//! This module provides JPEG compression functionality for VNC Tight encoding.
//! TurboJPEG support is optional and can be enabled with the `turbojpeg` feature.

#[cfg(feature = "turbojpeg")]
pub mod turbojpeg;

#[cfg(feature = "turbojpeg")]
pub use turbojpeg::TurboJpegEncoder;
