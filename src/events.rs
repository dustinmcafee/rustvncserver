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

//! Server events that can be received by the application.

use std::net::SocketAddr;

/// Events emitted by the VNC server.
#[derive(Debug, Clone)]
pub enum ServerEvent {
    /// A client has connected to the server.
    ClientConnected {
        /// Unique client identifier.
        id: usize,
        /// Client's socket address.
        address: SocketAddr,
    },

    /// A client has disconnected from the server.
    ClientDisconnected {
        /// Unique client identifier.
        id: usize,
    },

    /// Pointer movement or button event from a client.
    PointerEvent {
        /// Client identifier.
        client_id: usize,
        /// X coordinate.
        x: u16,
        /// Y coordinate.
        y: u16,
        /// Button mask (bit 0 = left, bit 1 = middle, bit 2 = right).
        button_mask: u8,
    },

    /// Key press or release event from a client.
    KeyEvent {
        /// Client identifier.
        client_id: usize,
        /// Key symbol (X11 keysym).
        key: u32,
        /// True if pressed, false if released.
        pressed: bool,
    },

    /// Clipboard text received from a client.
    ClipboardReceived {
        /// Client identifier.
        client_id: usize,
        /// Clipboard text content.
        text: String,
    },
}
