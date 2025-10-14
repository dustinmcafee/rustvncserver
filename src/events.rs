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
