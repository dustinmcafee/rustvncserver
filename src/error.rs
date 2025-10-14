//! Error types for the VNC server library.

use std::io;
use thiserror::Error;

/// Result type for VNC operations.
pub type Result<T> = std::result::Result<T, VncError>;

/// Errors that can occur in VNC server operations.
#[derive(Debug, Error)]
pub enum VncError {
    /// I/O error occurred.
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),

    /// VNC protocol error.
    #[error("Protocol error: {0}")]
    Protocol(String),

    /// Authentication failed.
    #[error("Authentication failed")]
    AuthenticationFailed,

    /// Invalid pixel format.
    #[error("Invalid pixel format")]
    InvalidPixelFormat,

    /// Encoding error.
    #[error("Encoding error: {0}")]
    Encoding(String),

    /// Invalid operation or state.
    #[error("Invalid operation: {0}")]
    InvalidOperation(String),

    /// Connection closed.
    #[error("Connection closed")]
    ConnectionClosed,
}
