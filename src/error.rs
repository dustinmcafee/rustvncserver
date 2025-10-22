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
