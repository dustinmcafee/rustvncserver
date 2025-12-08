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

//! VNC client connection handling and protocol implementation.
//!
//! This module manages individual VNC client sessions, handling:
//! - RFB protocol handshake and negotiation
//! - Client message processing (input events, encoding requests, etc.)
//! - Framebuffer update transmission with batching and rate limiting
//! - Client-specific state management (pixel format, encodings, dirty regions)
//!
//! # Protocol Flow
//!
//! 1. **Handshake**: Protocol version exchange and security negotiation
//! 2. **Initialization**: Send framebuffer dimensions and pixel format
//! 3. **Message Loop**: Handle incoming client messages and send framebuffer updates
//!
//! # Performance Features
//!
//! - **Update Deferral**: Batches small changes to reduce message overhead
//! - **Region Merging**: Combines overlapping dirty regions for efficiency
//! - **Encoding Selection**: Chooses optimal encoding based on client capabilities
//! - **Rate Limiting**: Prevents overwhelming clients with excessive update frequency

use bytes::{Buf, BufMut, BytesMut};
use flate2::Compress;
use flate2::Compression;
use flate2::FlushCompress;
use log::error;
#[cfg(feature = "debug-logging")]
use log::info;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::mpsc;
use tokio::sync::RwLock;

use crate::auth::VncAuth;
use crate::encoding;
use crate::encoding::tight::TightStreamCompressor;
use crate::framebuffer::{DirtyRegion, Framebuffer};
use crate::protocol::{
    PixelFormat, Rectangle, ServerInit, CLIENT_MSG_CLIENT_CUT_TEXT,
    CLIENT_MSG_FRAMEBUFFER_UPDATE_REQUEST, CLIENT_MSG_KEY_EVENT, CLIENT_MSG_POINTER_EVENT,
    CLIENT_MSG_SET_ENCODINGS, CLIENT_MSG_SET_PIXEL_FORMAT, ENCODING_COMPRESS_LEVEL_0,
    ENCODING_COMPRESS_LEVEL_9, ENCODING_COPYRECT, ENCODING_CORRE, ENCODING_HEXTILE,
    ENCODING_QUALITY_LEVEL_0, ENCODING_QUALITY_LEVEL_9, ENCODING_RAW, ENCODING_RRE, ENCODING_TIGHT,
    ENCODING_TIGHTPNG, ENCODING_ZLIB, ENCODING_ZLIBHEX, ENCODING_ZRLE, ENCODING_ZYWRLE,
    PROTOCOL_VERSION, SECURITY_RESULT_FAILED, SECURITY_RESULT_OK, SECURITY_TYPE_NONE,
    SECURITY_TYPE_VNC_AUTH, SERVER_MSG_FRAMEBUFFER_UPDATE, SERVER_MSG_SERVER_CUT_TEXT,
};
use rfb_encodings::translate;

/// Represents various events that a VNC client can send to the server.
/// These events typically correspond to user interactions like keyboard input,
/// pointer movements, or clipboard updates.
pub enum ClientEvent {
    /// A key press or release event.
    /// - `down`: `true` if the key is pressed, `false` if released.
    /// - `key`: The X Window System keysym of the key.
    KeyPress { down: bool, key: u32 },
    /// A pointer (mouse) movement or button event.
    /// - `x`: The X-coordinate of the pointer.
    /// - `y`: The Y-coordinate of the pointer.
    /// - `button_mask`: A bitmask indicating which mouse buttons are pressed.
    PointerMove { x: u16, y: u16, button_mask: u8 },
    /// A client-side clipboard (cut text) update.
    /// - `text`: The textual content from the client's clipboard.
    CutText { text: String },
    /// Notification that the client has disconnected.
    Disconnected,
}

/// Manages persistent zlib compression streams for Tight encoding.
///
/// Per RFC 6143 Tight encoding specification, uses 4 separate zlib streams
/// to maintain compression dictionaries:
/// - Stream 0: Full-color (truecolor) data
/// - Stream 1: Mono rect (2-color bitmap) data
/// - Stream 2: Indexed palette (3-16 colors) data
/// - Stream 3: Unused (reserved)
///
/// Each stream maintains its own dictionary and compression level, allowing
/// dynamic compression parameter changes without reinitializing the stream.
pub struct TightZlibStreams {
    /// Array of 4 zlib compression streams
    streams: [Option<Compress>; 4],
    /// Active flag for each stream
    active: [bool; 4],
    /// Compression level for each stream
    levels: [u8; 4],
}

impl TightZlibStreams {
    /// Creates a new `TightZlibStreams` with all streams uninitialized.
    pub fn new() -> Self {
        Self {
            streams: [None, None, None, None],
            active: [false; 4],
            levels: [0; 4],
        }
    }

    /// Gets or initializes a stream for the given stream ID and compression level.
    ///
    /// Implements lazy initialization and dynamic level changes:
    /// - On first use: Initialize stream with zlib
    /// - On level change: Update compression level dynamically
    /// - Otherwise: Use existing stream with preserved dictionary
    ///
    /// # Arguments
    /// * `stream_id` - The stream ID (0-3)
    /// * `level` - Desired compression level (0-9)
    ///
    /// # Returns
    /// Mutable reference to the initialized Compress stream
    fn get_or_init_stream(&mut self, stream_id: usize, level: u8) -> &mut Compress {
        assert!(stream_id < 4, "stream_id must be 0-3");

        if !self.active[stream_id] {
            // Initialize stream on first use
            self.streams[stream_id] = Some(Compress::new(Compression::new(u32::from(level)), true));
            self.active[stream_id] = true;
            self.levels[stream_id] = level;
        } else if self.levels[stream_id] != level {
            // Compression level changed - Don't recreate the stream!
            // Changing compression level mid-session with persistent streams is problematic:
            // - Recreating the stream resets the dictionary, causing client decompression errors
            // - Using set_level() can corrupt the stream state
            //
            // The safest approach: Keep using the ORIGINAL compression level for this stream.
            // The client's compression level preference mainly affects NEW streams.
            // This matches behavior of other VNC servers (e.g., TigerVNC).
            //
            // Do nothing - keep using self.levels[stream_id]
        }

        self.streams[stream_id].as_mut().unwrap()
    }

    /// Compresses data using the specified stream with `Z_SYNC_FLUSH`.
    ///
    /// Uses `Z_SYNC_FLUSH` to maintain the dictionary state for subsequent compressions
    /// per RFC 6143 Tight encoding specification.
    ///
    /// CRITICAL: This function does NOT reset the stream between calls! The stream maintains
    /// its dictionary state across multiple compressions, which allows the client to decompress
    /// the data using the same persistent stream state. This is essential for TIGHT encoding.
    ///
    /// # Arguments
    /// * `stream_id` - The stream ID (0-3)
    /// * `level` - Desired compression level (0-9)
    /// * `input` - Data to compress
    ///
    /// # Returns
    /// Compressed data, or error if compression fails
    #[allow(clippy::cast_possible_truncation)] // Zlib total_out limited to buffer size, safe to truncate
    fn compress(&mut self, stream_id: usize, level: u8, input: &[u8]) -> Result<Vec<u8>, String> {
        let stream = self.get_or_init_stream(stream_id, level);

        // Prepare output buffer (worst case: input size + overhead)
        let mut output = vec![0u8; input.len() + 64];

        // Compress with Z_SYNC_FLUSH to preserve dictionary for next compression
        // IMPORTANT: Do NOT reset() the stream! We need to maintain the dictionary state.
        let before_out = stream.total_out();

        match stream.compress(input, &mut output, FlushCompress::Sync) {
            Ok(flate2::Status::Ok | flate2::Status::StreamEnd) => {
                let total_out = (stream.total_out() - before_out) as usize;
                output.truncate(total_out);
                Ok(output)
            }
            Ok(flate2::Status::BufError) => Err("Compression buffer error".to_string()),
            Err(e) => Err(format!("Compression failed: {e}")),
        }
    }
}

/// Implement `TightStreamCompressor` trait for `TightZlibStreams`.
/// This allows the tight encoding module to use our stream manager.
impl TightStreamCompressor for TightZlibStreams {
    fn compress_tight_stream(
        &mut self,
        stream_id: u8,
        level: u8,
        input: &[u8],
    ) -> Result<Vec<u8>, String> {
        self.compress(stream_id as usize, level, input)
    }
}

/// Manages a single VNC client connection, handling communication, framebuffer updates,
/// and client input events.
///
/// This struct encapsulates the state and logic for interacting with a connected VNC viewer.
/// It is responsible for sending framebuffer updates to the client based on dirty regions,
/// processing incoming client messages (e.g., key events, pointer events, pixel format requests),
/// and managing client-specific settings like preferred encodings and JPEG quality.
pub struct VncClient {
    /// The read half of the stream for receiving client messages.
    read_stream: Box<dyn AsyncRead + Unpin + Send + Sync>,
    /// The write half of the stream for sending updates to the client.
    write_stream: Arc<tokio::sync::Mutex<Box<dyn AsyncWrite + Unpin + Send + Sync>>>,
    /// A reference to the framebuffer, used to retrieve pixel data for updates.
    framebuffer: Framebuffer,
    /// The pixel format requested by the client, protected by a `RwLock` for concurrent access.
    /// It is written by the message handler and read by the encoder.
    pixel_format: RwLock<PixelFormat>, // Protected - written by message handler, read by encoder
    /// The list of preferred encodings supported by the client, protected by a `RwLock`.
    /// It is written by the message handler and read by the encoder.
    encodings: RwLock<Vec<i32>>, // Protected - written by message handler, read by encoder
    /// Sender for client events (e.g., key presses, pointer movements) to be processed by other parts of the server.
    event_tx: mpsc::UnboundedSender<ClientEvent>,
    /// The `Instant` when the last framebuffer update was sent to this client, protected by a `RwLock`.
    /// Used for rate limiting and deferral logic.
    last_update_sent: RwLock<Instant>, // Protected - written by update sender, read by rate limiter
    /// The JPEG quality level for encodings, stored as an `AtomicU8` for atomic access from multiple contexts.
    jpeg_quality: AtomicU8, // Atomic - simple u8 value accessed from multiple contexts
    /// The compression level for encodings (e.g., Zlib), stored as an `AtomicU8` for atomic access.
    compression_level: AtomicU8, // Atomic - simple u8 value accessed from multiple contexts
    /// The VNC quality level (0-9, or 255 for unset = use JPEG).
    /// Stored as an `AtomicU8` for atomic access from multiple contexts.
    quality_level: AtomicU8, // Atomic - VNC quality level (0-9, 255=unset)
    /// A flag indicating whether the client has requested continuous framebuffer updates, stored as an `AtomicBool`.
    continuous_updates: AtomicBool, // Atomic - simple bool flag
    /// A shared, locked vector of `DirtyRegion`s specific to this client.
    /// These regions represent areas of the framebuffer that have been modified and need to be sent to the client.
    modified_regions: Arc<RwLock<Vec<DirtyRegion>>>, // Per-client dirty regions (standard VNC protocol style - receives pushes from framebuffer)
    /// The region specifically requested by the client for an update, protected by a `RwLock`.
    /// It is written by the message handler and read by the encoder.
    requested_region: RwLock<Option<DirtyRegion>>, // Protected - written by message handler, read by encoder
    /// `CopyRect` tracking (standard VNC protocol style): destination regions to be copied
    copy_region: Arc<RwLock<Vec<DirtyRegion>>>, // Destination regions for CopyRect
    /// Translation vector for `CopyRect`: (dx, dy) where src = dest + (dx, dy)
    copy_offset: RwLock<Option<(i16, i16)>>, // (dx, dy) translation for copy operations
    /// The duration to defer sending updates, matching `standard VNC protocol`'s default.
    defer_update_time: Duration, // Constant - set once at init
    /// The timestamp (in nanoseconds since creation) when deferring of updates began (0 if not deferring).
    /// Stored as an `AtomicU64` for atomic access.
    start_deferring_nanos: AtomicU64, // Atomic - nanos since creation (0 = not deferring)
    /// The `Instant` when this `VncClient` instance was created, used for calculating elapsed time.
    creation_time: Instant, // Constant - for calculating elapsed time
    /// The maximum number of rectangles to send in a single framebuffer update message, matching `standard VNC protocol`'s default.
    max_rects_per_update: usize, // Constant - set once at init
    /// A mutex used to ensure exclusive access to the client's `TcpStream` for sending data,
    /// preventing interleaved writes from concurrent tasks.
    send_mutex: Arc<tokio::sync::Mutex<()>>,
    /// Persistent zlib compressor for Zlib encoding (RFC 6143: one stream per connection).
    /// Protected by `RwLock` since encoding happens during `send_batched_update`.
    zlib_compressor: RwLock<Option<Compress>>,
    /// Persistent zlib compressor for `ZlibHex` encoding (RFC 6143: one stream per connection).
    /// Protected by `RwLock` since encoding happens during `send_batched_update`.
    zlibhex_compressor: RwLock<Option<Compress>>,
    /// Persistent zlib compressor for ZRLE encoding (RFC 6143: one stream per connection).
    /// Protected by `RwLock` since encoding happens during `send_batched_update`.
    #[allow(dead_code)]
    zrle_compressor: RwLock<Option<Compress>>,
    /// ZYWRLE quality level (0 = disabled, 1-3 = quality levels, higher = better quality).
    /// Stored as `AtomicU8` for atomic access. Updated based on client's quality setting.
    zywrle_level: AtomicU8, // Atomic - updated when ZYWRLE encoding is detected
    /// Persistent zlib compression streams for Tight encoding (4 streams with dictionaries).
    /// Protected by `RwLock` since encoding happens during `send_batched_update`.
    tight_zlib_streams: RwLock<TightZlibStreams>,
    /// Remote host address (IP:port) of the connected client (None for generic streams)
    remote_host: Option<String>,
    /// Destination port for repeater connections (None for direct connections)
    destination_port: Option<u16>,
    /// Repeater ID for repeater connections (None for direct connections)
    repeater_id: Option<String>,
    /// Unique client ID assigned by the server
    client_id: usize,
}

impl VncClient {
    /// Creates a new `VncClient` instance, performing the VNC handshake with the connected client.
    ///
    /// This function handles the initial protocol version exchange, security type negotiation,
    /// and sends the `ServerInit` message to the client, providing framebuffer information.
    ///
    /// # Arguments
    ///
    /// * `client_id` - The unique client ID assigned by the server.
    /// * `stream` - A stream implementing `AsyncRead + AsyncWrite + Unpin + Send` representing the connection to the VNC client.
    /// * `framebuffer` - The `Framebuffer` instance that this client will receive updates from.
    /// * `desktop_name` - The name of the desktop to be sent to the client during `ServerInit`.
    /// * `password` - An optional password for VNC authentication. If `Some`, VNC authentication
    ///   will be offered. (Note: Current implementation uses a placeholder for authentication).
    /// * `event_tx` - An `mpsc::UnboundedSender` for sending `ClientEvent`s generated by the client
    ///   (e.g., key presses, pointer movements) to other parts of the server.
    ///
    /// # Returns
    ///
    /// A `Result` which is `Ok(VncClient)` on successful handshake and initialization, or
    /// `Err(std::io::Error)` if an I/O error occurs during communication or handshake.
    pub async fn new<S>(
        client_id: usize,
        mut stream: S,
        framebuffer: Framebuffer,
        desktop_name: String,
        password: Option<String>,
        event_tx: mpsc::UnboundedSender<ClientEvent>,
    ) -> Result<Self, std::io::Error>
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + Sync + 'static,
    {
        // Capture remote host address before handshake
        let remote_host = None; // Generic streams may not have peer_addr

        // Disable Nagle's algorithm for immediate frame delivery
        //stream.set_nodelay(true)?;

        // Send protocol version
        stream.write_all(PROTOCOL_VERSION.as_bytes()).await?;

        // Read client protocol version
        let mut version_buf = vec![0u8; 12];
        stream.read_exact(&mut version_buf).await?;
        #[cfg(feature = "debug-logging")]
        info!("Client version: {}", String::from_utf8_lossy(&version_buf));

        // Send security types
        if password.is_some() {
            stream.write_all(&[1, SECURITY_TYPE_VNC_AUTH]).await?;
        } else {
            stream.write_all(&[1, SECURITY_TYPE_NONE]).await?;
        }

        // Read client's security type choice
        let mut sec_type = [0u8; 1];
        stream.read_exact(&mut sec_type).await?;

        // Handle authentication
        if sec_type[0] == SECURITY_TYPE_VNC_AUTH {
            let auth = VncAuth::new(password.clone());
            let challenge = auth.generate_challenge();
            stream.write_all(&challenge).await?;

            let mut response = vec![0u8; 16];
            stream.read_exact(&mut response).await?;

            if auth.verify_response(&response, &challenge) {
                let mut buf = BytesMut::with_capacity(4);
                buf.put_u32(SECURITY_RESULT_OK);
                stream.write_all(&buf).await?;
            } else {
                let mut buf = BytesMut::with_capacity(4);
                buf.put_u32(SECURITY_RESULT_FAILED);
                stream.write_all(&buf).await?;
                return Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "VNC authentication failed",
                ));
            }
        } else if sec_type[0] == SECURITY_TYPE_NONE {
            let mut buf = BytesMut::with_capacity(4);
            buf.put_u32(SECURITY_RESULT_OK);
            stream.write_all(&buf).await?;
        }

        // Read ClientInit
        let mut shared = [0u8; 1];
        stream.read_exact(&mut shared).await?;

        // Send ServerInit
        let server_init = ServerInit {
            framebuffer_width: framebuffer.width(),
            framebuffer_height: framebuffer.height(),
            pixel_format: PixelFormat::rgba32(),
            name: desktop_name,
        };

        let mut init_buf = BytesMut::new();
        server_init.write_to(&mut init_buf);
        stream.write_all(&init_buf).await?;

        log::info!("VNC client handshake completed");

        // Split stream into read/write halves for lock-free shutdown
        let (read_stream, write_stream) = tokio::io::split(stream);

        let creation_time = Instant::now();

        Ok(Self {
            read_stream: Box::new(read_stream),
            write_stream: Arc::new(tokio::sync::Mutex::new(Box::new(write_stream))),
            framebuffer,
            pixel_format: RwLock::new(PixelFormat::rgba32()),
            encodings: RwLock::new(vec![ENCODING_RAW]),
            event_tx,
            last_update_sent: RwLock::new(creation_time),
            jpeg_quality: AtomicU8::new(80),     // Default quality
            compression_level: AtomicU8::new(6), // Default zlib compression (balanced)
            quality_level: AtomicU8::new(255),   // 255 = unset (use JPEG by default)
            continuous_updates: AtomicBool::new(false),
            modified_regions: Arc::new(RwLock::new(Vec::new())),
            requested_region: RwLock::new(None),
            copy_region: Arc::new(RwLock::new(Vec::new())), // Initialize empty copy region
            copy_offset: RwLock::new(None),                 // No copy offset initially
            defer_update_time: Duration::from_millis(5),    // Match standard VNC protocol default
            start_deferring_nanos: AtomicU64::new(0),       // 0 = not deferring
            creation_time,
            max_rects_per_update: 50, // Match standard VNC protocol default
            send_mutex: Arc::new(tokio::sync::Mutex::new(())),
            zlib_compressor: RwLock::new(None), // Initialized lazily when first used
            zlibhex_compressor: RwLock::new(None), // Initialized lazily when first used
            zrle_compressor: RwLock::new(None), // Initialized lazily when first used
            zywrle_level: AtomicU8::new(0), // Disabled by default, updated when ZYWRLE is requested
            tight_zlib_streams: RwLock::new(TightZlibStreams::new()), // 4 persistent streams for Tight encoding
            remote_host,
            destination_port: None, // None for direct inbound connections
            repeater_id: None,      // None for direct inbound connections
            client_id,
        })
    }

    /// Returns a clone of the `Arc` containing the client's `modified_regions`.
    ///
    /// This handle is used to register the client with the `Framebuffer` to receive
    /// dirty region notifications.
    ///
    /// # Returns
    ///
    /// An `Arc<RwLock<Vec<DirtyRegion>>>` that can be used as a handle for the client's dirty regions.
    pub fn get_receiver_handle(&self) -> Arc<RwLock<Vec<DirtyRegion>>> {
        self.modified_regions.clone()
    }

    /// Returns a clone of the `Arc` containing the client's `copy_region`.
    ///
    /// This handle can be used to schedule copy operations for this client.
    ///
    /// # Returns
    ///
    /// An `Arc<RwLock<Vec<DirtyRegion>>>` that can be used as a handle for the client's copy regions.
    #[allow(dead_code)]
    pub fn get_copy_region_handle(&self) -> Arc<RwLock<Vec<DirtyRegion>>> {
        self.copy_region.clone()
    }

    /// Schedules a copy operation for this client (standard VNC protocol style).
    ///
    /// This method adds a region to be sent using `CopyRect` encoding with the specified offset.
    /// According to standard VNC protocol's algorithm, if a copy operation with a different offset
    /// already exists, the old copy region is treated as modified.
    ///
    /// # Arguments
    ///
    /// * `region` - The destination region to be copied.
    /// * `dx` - The X offset from destination to source (`src_x` = `dest_x` + dx).
    /// * `dy` - The Y offset from destination to source (`src_y` = `dest_y` + dy).
    pub async fn schedule_copy_region(&self, region: DirtyRegion, dx: i16, dy: i16) {
        let mut copy_regions = self.copy_region.write().await;
        let mut copy_offset = self.copy_offset.write().await;
        let mut modified_regions = self.modified_regions.write().await;

        // Check if we have an existing copy with a different offset
        if let Some((existing_dx, existing_dy)) = *copy_offset {
            if existing_dx != dx || existing_dy != dy {
                // Different offset - treat existing copy region as modified
                // This matches standard VNC protocol's behavior in rfbScheduleCopyRegion
                modified_regions.extend(copy_regions.drain(..));
                copy_regions.clear();
            }
        }

        // Add the new region to copy_region
        copy_regions.push(region);
        *copy_offset = Some((dx, dy));
    }

    /// Enters the main message loop for the `VncClient`, handling incoming data from the client
    /// and periodically sending framebuffer updates.
    ///
    /// This function continuously reads from the client's stream and processes VNC messages
    /// such as `SetPixelFormat`, `SetEncodings`, `FramebufferUpdateRequest`, `KeyEvent`,
    /// `PointerEvent`, and `ClientCutText`. It also uses a `tokio::time::interval` to
    /// periodically check if batched framebuffer updates should be sent to the client,
    /// based on dirty regions and deferral logic.
    ///
    /// # Returns
    ///
    /// `Ok(())` if the client disconnects gracefully.
    /// Returns `Err(std::io::Error)` if an I/O error occurs or an invalid message is received.
    #[allow(clippy::too_many_lines)] // VNC protocol message handler requires complete state machine
    #[allow(clippy::cast_possible_truncation)] // VNC protocol message fields use u8/u16/u32 as specified in RFC 6143
    #[allow(clippy::cast_sign_loss)] // VNC pseudo-encoding values are negative i32, converted to positive u8/u16 offsets
    pub async fn handle_messages(&mut self) -> Result<(), std::io::Error> {
        // Use standard VNC quality mapping (TigerVNC compatible)
        const TIGHT2TURBO_QUAL: [u8; 10] = [15, 29, 41, 42, 62, 77, 79, 86, 92, 100];
        // Limit clipboard size to prevent memory exhaustion attacks
        const MAX_CUT_TEXT: usize = 10 * 1024 * 1024; // 10MB limit

        let mut buf = BytesMut::with_capacity(4096);
        let mut check_interval = tokio::time::interval(tokio::time::Duration::from_millis(16)); // Check for updates ~60 times/sec

        loop {
            tokio::select! {
                // Handle incoming client messages
                result = self.read_stream.read_buf(&mut buf) => {
                    if result? == 0 {
                        let _ = self.event_tx.send(ClientEvent::Disconnected);
                        return Ok(());
                    }

                    // Process all available messages in the buffer
                    while !buf.is_empty() {

                        let msg_type = buf[0];

                        match msg_type {
                            CLIENT_MSG_SET_PIXEL_FORMAT => {
                                if buf.len() < 20 { // 1 + 3 padding + 16 pixel format
                                    break; // Need more data
                                }
                                buf.advance(1); // message type
                                buf.advance(3); // padding
                                let requested_format = PixelFormat::from_bytes(&mut buf)?;

                                // Validate that the requested format is valid and supported
                                if !requested_format.is_valid() {
                                    error!(
                                        "Client requested invalid pixel format (bpp={}, depth={}, truecolor={}, shifts=R{},G{},B{}). Disconnecting.",
                                        requested_format.bits_per_pixel,
                                        requested_format.depth,
                                        requested_format.true_colour_flag,
                                        requested_format.red_shift,
                                        requested_format.green_shift,
                                        requested_format.blue_shift
                                    );
                                    let _ = self.event_tx.send(ClientEvent::Disconnected);
                                    return Err(std::io::Error::new(
                                        std::io::ErrorKind::InvalidData,
                                        "Invalid pixel format requested"
                                    ));
                                }

                                // Accept the format and store it for translation during encoding
                                *self.pixel_format.write().await = requested_format.clone();

                                #[cfg(feature = "debug-logging")]
                                {
                                    info!(
                                        "Client set pixel format: {}bpp, depth={}, bigEndian={}, R_shift={} R_max={}, G_shift={} G_max={}, B_shift={} B_max={} - compatible_with_rgba32={}",
                                        requested_format.bits_per_pixel,
                                        requested_format.depth,
                                        requested_format.big_endian_flag,
                                        requested_format.red_shift, requested_format.red_max,
                                        requested_format.green_shift, requested_format.green_max,
                                        requested_format.blue_shift, requested_format.blue_max,
                                        requested_format.is_compatible_with_rgba32()
                                    );
                                }
                            }
                            CLIENT_MSG_SET_ENCODINGS => {
                                if buf.len() < 4 { // 1 + 1 padding + 2 count
                                    break;
                                }
                                buf.advance(1); // message type
                                buf.advance(1); // padding
                                let count = buf.get_u16() as usize;
                                if buf.len() < count * 4 {
                                    break; // Need more data
                                }
                                let mut encodings_list = Vec::with_capacity(count);
                                for _ in 0..count {
                                    let encoding = buf.get_i32();
                                    encodings_list.push(encoding);

                                    // Check for quality level pseudo-encodings (-32 to -23)
                                    if (ENCODING_QUALITY_LEVEL_0..=ENCODING_QUALITY_LEVEL_9).contains(&encoding) {
                                        // -32 = level 0 (lowest), -23 = level 9 (highest)
                                        let quality_level = (encoding - ENCODING_QUALITY_LEVEL_0) as u8;
                                        let quality = TIGHT2TURBO_QUAL[quality_level as usize];
                                        self.jpeg_quality.store(quality, Ordering::Relaxed);
                                        self.quality_level.store(quality_level, Ordering::Relaxed); // Store VNC quality level
                                        #[cfg(feature = "debug-logging")]
                                        info!("Client requested quality level {quality_level}, using JPEG quality {quality}");
                                    }

                                    // Check for compression level pseudo-encodings (-256 to -247)
                                    if (ENCODING_COMPRESS_LEVEL_0..=ENCODING_COMPRESS_LEVEL_9).contains(&encoding) {
                                        // -256 = level 0 (lowest/fastest), -247 = level 9 (highest/slowest)
                                        let compression_level = (encoding - ENCODING_COMPRESS_LEVEL_0) as u8;
                                        // Use compression level directly (0=fastest, 9=best compression)
                                        self.compression_level.store(compression_level, Ordering::Relaxed);
                                        #[cfg(feature = "debug-logging")]
                                        info!("Client requested compression level {compression_level}, using zlib level {compression_level}");
                                    }
                                }
                                self.encodings.write().await.clone_from(&encodings_list);
                                #[cfg(feature = "debug-logging")]
                                info!("Client set {count} encodings: {encodings_list:?}");
                            }
                            CLIENT_MSG_FRAMEBUFFER_UPDATE_REQUEST => {
                                if buf.len() < 10 { // 1 + 1 incremental + 8 (x, y, w, h)
                                    break;
                                }
                                buf.advance(1); // message type
                                let incremental = buf.get_u8() != 0;
                                let x = buf.get_u16();
                                let y = buf.get_u16();
                                let width = buf.get_u16();
                                let height = buf.get_u16();

                                #[cfg(feature = "debug-logging")]
                                info!("FramebufferUpdateRequest: incremental={incremental}, region=({x},{y} {width}x{height})");

                                // Track requested region (standard VNC protocol cl->requestedRegion)
                                *self.requested_region.write().await = Some(DirtyRegion::new(x, y, width, height));

                                // Enable continuous updates for both incremental and non-incremental requests
                                // The difference is handled below: non-incremental clears and adds full region
                                self.continuous_updates.store(true, Ordering::Relaxed);

                                // Handle non-incremental updates (full refresh)
                                if !incremental {
                                    // Clear existing regions and mark full requested region as dirty
                                    let full_region = DirtyRegion::new(x, y, width, height);
                                    let mut regions = self.modified_regions.write().await;
                                    regions.clear();
                                    regions.push(full_region);
                                    #[cfg(feature = "debug-logging")]
                                    info!("Non-incremental update: added full region to dirty list");
                                }

                                // Start deferring if we have regions to send
                                // Note: There's a small window where regions could be drained between
                                // the check and the store, but this is acceptable - at worst we defer
                                // when the queue is already empty (harmless). Using a write lock here
                                // would hurt performance on this hot path.
                                {
                                    let regions = self.modified_regions.read().await;
                                    if !regions.is_empty() && self.start_deferring_nanos.load(Ordering::Relaxed) == 0 {
                                        // Not currently deferring, start now
                                        let nanos = Instant::now().duration_since(self.creation_time).as_nanos() as u64;
                                        self.start_deferring_nanos.store(nanos, Ordering::Relaxed);
                                    }
                                }
                            }
                            CLIENT_MSG_KEY_EVENT => {
                                if buf.len() < 8 { // 1 + 1 down + 2 padding + 4 key
                                    break;
                                }
                                buf.advance(1); // message type
                                let down = buf.get_u8() != 0;
                                buf.advance(2); // padding
                                let key = buf.get_u32();

                                let _ = self.event_tx.send(ClientEvent::KeyPress { down, key });
                            }
                            CLIENT_MSG_POINTER_EVENT => {
                                if buf.len() < 6 { // 1 + 1 button + 2 x + 2 y
                                    break;
                                }
                                buf.advance(1); // message type
                                let button_mask = buf.get_u8();
                                let x = buf.get_u16();
                                let y = buf.get_u16();

                                let _ = self.event_tx.send(ClientEvent::PointerMove {
                                    x,
                                    y,
                                    button_mask,
                                });
                            }
                            CLIENT_MSG_CLIENT_CUT_TEXT => {
                                if buf.len() < 8 { // 1 + 3 padding + 4 length
                                    break;
                                }
                                buf.advance(1); // message type
                                buf.advance(3); // padding
                                let length = buf.get_u32() as usize;

                                if length > MAX_CUT_TEXT {
                                    error!("Cut text too large: {length} bytes (max {MAX_CUT_TEXT}), disconnecting client");
                                    let _ = self.event_tx.send(ClientEvent::Disconnected);
                                    return Err(std::io::Error::new(
                                        std::io::ErrorKind::InvalidData,
                                        "Cut text too large"
                                    ));
                                }

                                if buf.len() < length {
                                    break; // Need more data
                                }
                                let text_bytes = buf.split_to(length);
                                if let Ok(text) = String::from_utf8(text_bytes.to_vec()) {
                                    let _ = self.event_tx.send(ClientEvent::CutText { text });
                                }
                            }
                            _ => {
                                error!("Unknown message type: {msg_type}, disconnecting client");
                                let _ = self.event_tx.send(ClientEvent::Disconnected);
                                return Err(std::io::Error::new(
                                    std::io::ErrorKind::InvalidData,
                                    format!("Unknown message type: {msg_type}")
                                ));
                            }
                        }
                    }
                }

                // Periodically check if we should send updates (standard VNC protocol style)
                _ = check_interval.tick() => {
                    let continuous = self.continuous_updates.load(Ordering::Relaxed);
                    if continuous {
                        // Check if we have regions and deferral time has elapsed
                        // Regions are already pushed to us by framebuffer (no merge needed!)
                        let should_send = {
                            let regions = self.modified_regions.read().await;
                            if regions.is_empty() {
                                false
                            } else {
                                let defer_nanos = self.start_deferring_nanos.load(Ordering::Relaxed);
                                if defer_nanos == 0 {
                                    // Not currently deferring, start now
                                    let nanos = Instant::now().duration_since(self.creation_time).as_nanos() as u64;
                                    self.start_deferring_nanos.store(nanos, Ordering::Relaxed);
                                    false // Don't send yet, just started deferring
                                } else {
                                    // Check if defer time elapsed
                                    let defer_start = self.creation_time + Duration::from_nanos(defer_nanos);
                                    let now = Instant::now();
                                    let elapsed = now.duration_since(defer_start);
                                    let last_sent = *self.last_update_sent.read().await;
                                    let time_since_last = now.duration_since(last_sent);
                                    let min_interval = Duration::from_millis(33); // ~30 FPS max

                                    elapsed >= self.defer_update_time && time_since_last >= min_interval
                                }
                            }
                        };

                        if should_send {
                            self.send_batched_update().await?;
                        }
                    }
                }
            }
        }
    }

    /// Sends a batched framebuffer update message to the client.
    ///
    /// This function implements standard VNC protocol's update sending algorithm:
    /// 1. Send `CopyRect` regions first (from `copy_region` with stored offset)
    /// 2. Then send modified regions (from `modified_regions`)
    ///
    /// The update includes multiple rectangles in a single message to improve efficiency.
    ///
    /// # Returns
    ///
    /// A `Result` which is `Ok(())` on successful transmission of the update, or
    /// `Err(std::io::Error)` if an I/O error occurs during encoding or sending.
    #[allow(clippy::too_many_lines)] // VNC framebuffer update encoding requires handling all encoding types
    #[allow(clippy::cast_possible_truncation)] // VNC protocol rectangle headers use u16 dimensions
    async fn send_batched_update(&mut self) -> Result<(), std::io::Error> {
        // Get requested region (standard VNC protocol: requestedRegion)
        let requested = *self.requested_region.read().await;

        #[cfg(feature = "debug-logging")]
        info!("send_batched_update called, requested region: {requested:?}");

        // STEP 1: Get copy regions to send (standard VNC protocol: copyRegion sent FIRST)
        let (copy_regions_to_send, copy_src_offset): (Vec<DirtyRegion>, Option<(i16, i16)>) = {
            let mut copy_regions = self.copy_region.write().await;
            let mut copy_offset = self.copy_offset.write().await;

            if copy_regions.is_empty() {
                (Vec::new(), None)
            } else {
                let offset = *copy_offset;
                let regions: Vec<DirtyRegion> = if let Some(req) = requested {
                    // Filter and drain: only take regions that intersect with requested region
                    // This preserves non-intersecting regions for later updates
                    let mut result = Vec::new();
                    copy_regions.retain(|region| {
                        if let Some(intersection) = region.intersect(&req) {
                            result.push(intersection);
                            false // Remove from copy_regions (drained)
                        } else {
                            true // Keep in copy_regions for later
                        }
                    });
                    result
                } else {
                    copy_regions.drain(..).collect()
                };

                // If we drained all regions, clear the offset
                if copy_regions.is_empty() {
                    *copy_offset = None;
                }

                (regions, offset)
            }
        };

        // STEP 2: Get modified regions to send (standard VNC protocol: modifiedRegion sent AFTER copyRegion)
        let modified_regions_to_send: Vec<DirtyRegion> = {
            let mut regions = self.modified_regions.write().await;

            if regions.is_empty() {
                Vec::new()
            } else {
                // Calculate how many regions we can send
                let remaining_slots = self
                    .max_rects_per_update
                    .saturating_sub(copy_regions_to_send.len());
                let num_rects = regions.len().min(remaining_slots);

                if let Some(req) = requested {
                    // Filter and drain: only take regions that intersect with requested region
                    // This preserves non-intersecting regions for later updates
                    let mut result = Vec::new();
                    let mut drained_count = 0;

                    regions.retain(|region| {
                        if drained_count >= num_rects {
                            true // Keep remaining regions (hit limit)
                        } else if let Some(intersection) = region.intersect(&req) {
                            result.push(intersection);
                            drained_count += 1;
                            false // Remove from regions (drained)
                        } else {
                            true // Keep in regions for later (doesn't intersect)
                        }
                    });
                    result
                } else {
                    // No requested region set, drain up to num_rects
                    regions.drain(..num_rects).collect()
                }
            }
        };

        // If no regions to send at all, nothing to do
        if copy_regions_to_send.is_empty() && modified_regions_to_send.is_empty() {
            #[cfg(feature = "debug-logging")]
            info!(
                "No regions to send (copy={}, modified={})",
                copy_regions_to_send.len(),
                modified_regions_to_send.len()
            );
            return Ok(());
        }

        #[cfg_attr(not(feature = "debug-logging"), allow(unused_variables))]
        let start = Instant::now();

        // Calculate total rectangles including CoRRE tiles
        // For CoRRE encoding, large rectangles are split into 255x255 tiles
        let mut total_rects = copy_regions_to_send.len();

        // Determine preferred encoding from client's list
        // Select the first encoding that the server supports, skipping COPYRECT
        let encodings = self.encodings.read().await;
        let preferred_encoding = encodings
            .iter()
            .find(|&&enc| {
                // Skip COPYRECT - it's only for copy operations, not general encoding
                if enc == ENCODING_COPYRECT {
                    return false;
                }
                // Check if this encoding is supported
                // Either it has explicit handling in client.rs or get_encoder returns Some
                matches!(
                    enc,
                    ENCODING_ZLIB
                        | ENCODING_ZLIBHEX
                        | ENCODING_ZRLE
                        | ENCODING_ZYWRLE
                        | ENCODING_TIGHT
                ) || encoding::get_encoder(enc).is_some()
            })
            .copied()
            .unwrap_or(ENCODING_RAW);
        drop(encodings);

        #[cfg(feature = "debug-logging")]
        info!("DEBUG: preferred_encoding = {preferred_encoding}");

        #[cfg(feature = "debug-logging")]
        info!(
            "DEBUG: modified_regions_to_send.len() = {}",
            modified_regions_to_send.len()
        );

        #[cfg(feature = "debug-logging")]
        info!(
            "DEBUG: copy_regions_to_send.len() = {}",
            copy_regions_to_send.len()
        );

        // For TIGHT encoding, pre-encode regions to determine rectangle count
        let mut tight_encoded_regions = Vec::new();
        if preferred_encoding == ENCODING_TIGHT {
            #[cfg(feature = "debug-logging")]
            info!(
                "DEBUG: Entering TIGHT pre-encoding block, {} regions",
                modified_regions_to_send.len()
            );

            // Get client's pixel format to pass to encoder
            let pixel_format = self.pixel_format.read().await;
            let client_format_clone = pixel_format.clone();
            drop(pixel_format);

            #[cfg(feature = "debug-logging")]
            info!(
                "DEBUG: Client pixel format: {}bpp",
                client_format_clone.bits_per_pixel
            );

            let mut tight_streams = self.tight_zlib_streams.write().await;

            #[cfg(feature = "debug-logging")]
            info!("DEBUG: Acquired tight_zlib_streams lock");

            for region in &modified_regions_to_send {
                #[cfg(feature = "debug-logging")]
                info!(
                    "DEBUG: Processing region {}x{} at ({}, {})",
                    region.width, region.height, region.x, region.y
                );

                let pixel_data = match self
                    .framebuffer
                    .get_rect(region.x, region.y, region.width, region.height)
                    .await
                {
                    Ok(data) => {
                        #[cfg(feature = "debug-logging")]
                        info!("DEBUG: Got pixel data, {} bytes", data.len());
                        data
                    }
                    Err(e) => {
                        error!(
                            "Failed to get rectangle ({}, {}, {}, {}): {}",
                            region.x, region.y, region.width, region.height, e
                        );
                        continue;
                    }
                };

                #[cfg(feature = "debug-logging")]
                info!(
                    "DEBUG: Calling encode_tight_rects for {}x{} with {}bpp",
                    region.width, region.height, client_format_clone.bits_per_pixel
                );

                let sub_rects = encoding::tight::encode_tight_rects(
                    &pixel_data,
                    region.width,
                    region.height,
                    self.quality_level.load(Ordering::Relaxed),
                    self.compression_level.load(Ordering::Relaxed),
                    &client_format_clone,
                    &mut *tight_streams,
                );

                #[cfg(feature = "debug-logging")]
                info!(
                    "DEBUG: encode_tight_rects returned {} sub-rectangles",
                    sub_rects.len()
                );

                #[cfg(feature = "debug-logging")]
                info!(
                    "TIGHT: region {}x{} split into {} sub-rectangles",
                    region.width,
                    region.height,
                    sub_rects.len()
                );

                total_rects += sub_rects.len();
                tight_encoded_regions.push((region, sub_rects));
            }
            drop(tight_streams);

            #[cfg(feature = "debug-logging")]
            info!("DEBUG: TIGHT pre-encoding complete, total_rects={total_rects}");
        } else {
            // Count rectangles for modified regions (accounting for CoRRE tiling)
            for region in &modified_regions_to_send {
                if preferred_encoding == ENCODING_CORRE
                    && (region.width > 255 || region.height > 255)
                {
                    // Count how many tiles this region will be split into
                    let num_tiles_x = region.width.div_ceil(255) as usize;
                    let num_tiles_y = region.height.div_ceil(255) as usize;
                    total_rects += num_tiles_x * num_tiles_y;
                } else {
                    total_rects += 1;
                }
            }
        }

        let mut response = BytesMut::new();

        // Message type
        response.put_u8(SERVER_MSG_FRAMEBUFFER_UPDATE);
        response.put_u8(0); // padding
        response.put_u16(total_rects as u16); // number of rectangles

        #[cfg(feature = "debug-logging")]
        info!("Writing framebuffer update header: total_rects={total_rects}");

        #[cfg_attr(
            not(feature = "debug-logging"),
            allow(unused_variables, unused_assignments, unused_mut)
        )]
        let mut encoding_name = match preferred_encoding {
            ENCODING_TIGHT => "TIGHT",
            ENCODING_TIGHTPNG => "TIGHTPNG",
            ENCODING_ZYWRLE => "ZYWRLE",
            ENCODING_ZRLE => "ZRLE",
            ENCODING_ZLIBHEX => "ZLIBHEX",
            ENCODING_ZLIB => "ZLIB",
            ENCODING_HEXTILE => "HEXTILE",
            ENCODING_RRE => "RRE",
            ENCODING_CORRE => "CORRE",
            _ => "RAW",
        };

        #[cfg_attr(
            not(feature = "debug-logging"),
            allow(unused_variables, unused_assignments)
        )]
        let mut total_pixels = 0u64;
        #[cfg_attr(
            not(feature = "debug-logging"),
            allow(unused_variables, unused_assignments)
        )]
        let mut copy_rect_count = 0;

        // Load quality/compression settings atomically
        let jpeg_quality = self.jpeg_quality.load(Ordering::Relaxed);
        let compression_level = self.compression_level.load(Ordering::Relaxed);
        let _quality_level = self.quality_level.load(Ordering::Relaxed);

        // STEP 1: Send copy regions FIRST (standard VNC protocol style)
        if let Some((dx, dy)) = copy_src_offset {
            for region in &copy_regions_to_send {
                // Calculate source position from destination + offset
                // In standard VNC protocol: src = dest + (dx, dy)
                #[allow(clippy::cast_sign_loss)]
                // CopyRect offset calculation: dx/dy are i16, sum guaranteed positive
                let src_x = (i32::from(region.x) + i32::from(dx)) as u16;
                #[allow(clippy::cast_sign_loss)]
                // CopyRect offset calculation: dx/dy are i16, sum guaranteed positive
                let src_y = (i32::from(region.y) + i32::from(dy)) as u16;

                // Use CopyRect encoding
                let rect = Rectangle {
                    x: region.x,
                    y: region.y,
                    width: region.width,
                    height: region.height,
                    encoding: ENCODING_COPYRECT,
                };
                rect.write_header(&mut response);

                // CopyRect data is just src_x and src_y
                response.put_u16(src_x);
                response.put_u16(src_y);

                total_pixels += u64::from(region.width) * u64::from(region.height);
                copy_rect_count += 1;
            }
        }

        // STEP 2: Send modified regions (standard VNC protocol: sent AFTER copy regions)

        #[cfg(feature = "debug-logging")]
        info!("DEBUG: Starting STEP 2 - Send modified regions");

        // Handle TIGHT encoding separately (already pre-encoded)
        if preferred_encoding == ENCODING_TIGHT {
            use crate::protocol::UPDATE_BUF_SIZE;

            #[cfg(feature = "debug-logging")]
            info!(
                "DEBUG: In TIGHT output section, tight_encoded_regions.len()={}",
                tight_encoded_regions.len()
            );

            #[cfg(feature = "debug-logging")]
            let mut rect_count = 0;

            for (region, sub_rects) in &tight_encoded_regions {
                #[cfg(feature = "debug-logging")]
                info!(
                    "DEBUG: Processing output region {}x{} with {} sub-rects",
                    region.width,
                    region.height,
                    sub_rects.len()
                );

                for (rel_x, rel_y, w, h, encoded) in sub_rects {
                    // Calculate size of this rectangle (header + data)
                    let rect_size = 12 + encoded.len(); // 12 bytes header + encoded data

                    // Check if adding this rectangle would exceed buffer limit
                    if response.len() + rect_size > UPDATE_BUF_SIZE {
                        #[cfg(feature = "debug-logging")]
                        info!("DEBUG: Buffer limit reached ({} bytes), flushing to continue streaming", response.len());

                        // Send current buffer chunk
                        let mut send_mutex = self.write_stream.lock().await;
                        send_mutex.write_all(&response).await?;
                        drop(send_mutex);

                        // Clear buffer and continue streaming rectangles
                        // Header was already sent in first flush, subsequent flushes are just raw rectangle data
                        response.clear();
                    }

                    // Sub-rectangle coordinates are relative to region origin
                    // Convert to absolute screen coordinates
                    let rect = Rectangle {
                        x: region.x + rel_x,
                        y: region.y + rel_y,
                        width: *w,
                        height: *h,
                        encoding: ENCODING_TIGHT,
                    };

                    #[cfg(feature = "debug-logging")]
                    info!("RECT #{}: {}x{} at ({},{}), TIGHT data={} bytes, response_size_before={}, response_size_after={}",
                        rect_count, w, h, region.x + rel_x, region.y + rel_y, encoded.len(), response.len(), response.len() + rect_size);

                    rect.write_header(&mut response);
                    response.extend_from_slice(encoded);

                    total_pixels += u64::from(*w) * u64::from(*h);

                    #[cfg(feature = "debug-logging")]
                    {
                        rect_count += 1;
                    }
                }
            }

            #[cfg(feature = "debug-logging")]
            info!(
                "DEBUG: TIGHT output complete, wrote {} rectangle headers, response.len()={}",
                rect_count,
                response.len()
            );
        } else {
            // Handle other encodings
            for region in &modified_regions_to_send {
                // For CoRRE encoding: split large rectangles into 255x255 tiles
                // (CoRRE uses u8 coordinates, so dimensions must be ≤255)
                if preferred_encoding == ENCODING_CORRE
                    && (region.width > 255 || region.height > 255)
                {
                    #[cfg(feature = "debug-logging")]
                    info!(
                        "CoRRE: Splitting {}x{} region into 255x255 tiles",
                        region.width, region.height
                    );
                    // Split rectangle into tiles ≤255x255 per RFC 6143 CoRRE specification
                    let mut y = 0;
                    while y < region.height {
                        let tile_height = std::cmp::min(255, region.height - y);
                        let mut x = 0;
                        while x < region.width {
                            let tile_width = std::cmp::min(255, region.width - x);
                            #[cfg(feature = "debug-logging")]
                            info!(
                                "CoRRE: Encoding tile at ({},{}) size {}x{}",
                                region.x + x,
                                region.y + y,
                                tile_width,
                                tile_height
                            );

                            // Get pixel data for this tile
                            let tile_pixel_data = match self
                                .framebuffer
                                .get_rect(region.x + x, region.y + y, tile_width, tile_height)
                                .await
                            {
                                Ok(data) => data,
                                Err(e) => {
                                    error!(
                                        "Failed to get rectangle ({}, {}, {}, {}): {}",
                                        region.x + x,
                                        region.y + y,
                                        tile_width,
                                        tile_height,
                                        e
                                    );
                                    x += tile_width;
                                    continue;
                                }
                            };

                            // Encode this tile with CoRRE
                            if let Some(encoder) = encoding::get_encoder(ENCODING_CORRE) {
                                let encoded = encoder.encode(
                                    &tile_pixel_data,
                                    tile_width,
                                    tile_height,
                                    jpeg_quality,
                                    compression_level,
                                );

                                // Calculate nSubrects from encoded buffer size
                                // Encoder returns: bgColor(4) + subrects, each subrect is 8 bytes
                                let n_subrects = if encoded.len() >= 4 {
                                    (encoded.len() - 4) / 8
                                } else {
                                    0
                                };

                                // Write rectangle header for this tile
                                let rect = Rectangle {
                                    x: region.x + x,
                                    y: region.y + y,
                                    width: tile_width,
                                    height: tile_height,
                                    encoding: ENCODING_CORRE,
                                };
                                rect.write_header(&mut response);

                                // Write RRE header (nSubrects in big-endian) - protocol layer responsibility
                                // CoRRE uses same header structure as RRE per RFC 6143
                                response.put_u32(n_subrects as u32);

                                // Write encoder output (background color + subrectangle data)
                                response.extend_from_slice(&encoded);

                                total_pixels += u64::from(tile_width) * u64::from(tile_height);
                            }

                            x += tile_width;
                        }
                        y += tile_height;
                    }
                    continue; // Skip normal encoding path for this region
                }

                // Get pixel data
                let pixel_data = match self
                    .framebuffer
                    .get_rect(region.x, region.y, region.width, region.height)
                    .await
                {
                    Ok(data) => data,
                    Err(e) => {
                        error!(
                            "Failed to get rectangle ({}, {}, {}, {}): {}",
                            region.x, region.y, region.width, region.height, e
                        );
                        continue; // Skip this invalid rectangle
                    }
                };

                // Apply pixel format translation and encode
                // Translation happens before encoding per RFC 6143
                let client_pixel_format = self.pixel_format.read().await;
                let server_format = PixelFormat::rgba32();

                let (actual_encoding, encoded) = if preferred_encoding == ENCODING_RAW {
                    // For Raw encoding: translation IS the encoding (like standard VNC protocol)
                    // Just translate and send directly, no additional processing
                    let translated = if client_pixel_format.is_compatible_with_rgba32() {
                        // Fast path: no translation, but still need to strip alpha
                        let mut buf = BytesMut::with_capacity(
                            (region.width as usize * region.height as usize) * 4,
                        );
                        for chunk in pixel_data.chunks_exact(4) {
                            buf.put_u8(chunk[0]); // R
                            buf.put_u8(chunk[1]); // G
                            buf.put_u8(chunk[2]); // B
                            buf.put_u8(0); // Padding (not alpha)
                        }
                        buf
                    } else {
                        // Translate from server format (RGBA32) to client's requested format
                        translate::translate_pixels(
                            &pixel_data,
                            &server_format,
                            &client_pixel_format,
                        )
                    };
                    (ENCODING_RAW, translated)
                } else if preferred_encoding == ENCODING_ZLIB {
                    // Translate pixels to client format first
                    let translated = if client_pixel_format.is_compatible_with_rgba32() {
                        // Fast path: no translation, but still need to strip alpha
                        let mut buf = BytesMut::with_capacity(
                            (region.width as usize * region.height as usize) * 4,
                        );
                        for chunk in pixel_data.chunks_exact(4) {
                            buf.put_u8(chunk[0]); // R
                            buf.put_u8(chunk[1]); // G
                            buf.put_u8(chunk[2]); // B
                            buf.put_u8(0); // Padding (not alpha)
                        }
                        buf
                    } else {
                        // Translate from server format (RGBA32) to client's requested format
                        translate::translate_pixels(
                            &pixel_data,
                            &server_format,
                            &client_pixel_format,
                        )
                    };

                    // Initialize ZLIB compressor lazily on first use
                    let mut zlib_lock = self.zlib_compressor.write().await;
                    if zlib_lock.is_none() {
                        *zlib_lock = Some(Compress::new(
                            Compression::new(u32::from(compression_level)),
                            true,
                        ));
                        #[cfg(feature = "debug-logging")]
                        info!("Initialized ZLIB compressor with level {compression_level}");
                    }
                    let zlib_comp = zlib_lock.as_mut().unwrap();

                    match encoding::encode_zlib_persistent(&translated, zlib_comp) {
                        Ok(data) => (ENCODING_ZLIB, BytesMut::from(&data[..])),
                        Err(e) => {
                            error!("ZLIB encoding failed: {e}, falling back to RAW");
                            #[cfg(feature = "debug-logging")]
                            {
                                encoding_name = "RAW";
                            }
                            // translated already contains the correctly formatted data
                            (ENCODING_RAW, translated)
                        }
                    }
                } else if preferred_encoding == ENCODING_ZLIBHEX {
                    // Translate pixels to client format first
                    let translated = if client_pixel_format.is_compatible_with_rgba32() {
                        // Fast path: no translation, but still need to strip alpha
                        let mut buf = BytesMut::with_capacity(
                            (region.width as usize * region.height as usize) * 4,
                        );
                        for chunk in pixel_data.chunks_exact(4) {
                            buf.put_u8(chunk[0]); // R
                            buf.put_u8(chunk[1]); // G
                            buf.put_u8(chunk[2]); // B
                            buf.put_u8(0); // Padding (not alpha)
                        }
                        buf
                    } else {
                        // Translate from server format (RGBA32) to client's requested format
                        translate::translate_pixels(
                            &pixel_data,
                            &server_format,
                            &client_pixel_format,
                        )
                    };

                    // Initialize ZLIBHEX compressor lazily on first use
                    let mut zlibhex_lock = self.zlibhex_compressor.write().await;
                    if zlibhex_lock.is_none() {
                        *zlibhex_lock = Some(Compress::new(
                            Compression::new(u32::from(compression_level)),
                            true,
                        ));
                        #[cfg(feature = "debug-logging")]
                        info!("Initialized ZLIBHEX compressor with level {compression_level}");
                    }
                    let zlibhex_comp = zlibhex_lock.as_mut().unwrap();

                    match encoding::encode_zlibhex_persistent(
                        &translated,
                        region.width,
                        region.height,
                        zlibhex_comp,
                    ) {
                        Ok(data) => (ENCODING_ZLIBHEX, BytesMut::from(&data[..])),
                        Err(e) => {
                            error!("ZLIBHEX encoding failed: {e}, falling back to RAW");
                            #[cfg(feature = "debug-logging")]
                            {
                                encoding_name = "RAW";
                            }
                            // translated already contains the correctly formatted data
                            (ENCODING_RAW, translated)
                        }
                    }
                } else if preferred_encoding == ENCODING_ZRLE {
                    // Translate pixels to client format first
                    let translated = if client_pixel_format.is_compatible_with_rgba32() {
                        // Fast path: no translation, but still need to strip alpha
                        let mut buf = BytesMut::with_capacity(
                            (region.width as usize * region.height as usize) * 4,
                        );
                        for chunk in pixel_data.chunks_exact(4) {
                            buf.put_u8(chunk[0]); // R
                            buf.put_u8(chunk[1]); // G
                            buf.put_u8(chunk[2]); // B
                            buf.put_u8(0); // Padding (not alpha)
                        }
                        buf
                    } else {
                        // Translate from server format (RGBA32) to client's requested format
                        translate::translate_pixels(
                            &pixel_data,
                            &server_format,
                            &client_pixel_format,
                        )
                    };

                    // Initialize ZRLE compressor lazily on first use
                    let mut zrle_lock = self.zrle_compressor.write().await;
                    if zrle_lock.is_none() {
                        *zrle_lock = Some(Compress::new(
                            Compression::new(u32::from(compression_level)),
                            true,
                        ));
                        #[cfg(feature = "debug-logging")]
                        info!("Initialized ZRLE compressor with level {compression_level}");
                    }
                    let zrle_comp = zrle_lock.as_mut().unwrap();

                    // Use client's pixel format for encoding
                    match encoding::encode_zrle_persistent(
                        &translated,
                        region.width,
                        region.height,
                        &client_pixel_format,
                        zrle_comp,
                    ) {
                        Ok(data) => (ENCODING_ZRLE, BytesMut::from(&data[..])),
                        Err(e) => {
                            error!("ZRLE encoding failed: {e}, falling back to RAW");
                            #[cfg(feature = "debug-logging")]
                            {
                                encoding_name = "RAW";
                            }
                            // translated already contains the correctly formatted data
                            (ENCODING_RAW, translated)
                        }
                    }
                } else if preferred_encoding == ENCODING_ZYWRLE {
                    // ZYWRLE: Apply wavelet preprocessing then use ZRLE encoder
                    let level = self.zywrle_level.load(Ordering::Relaxed) as usize;

                    // Allocate coefficient buffer for wavelet transform
                    let buf_size = (region.width as usize) * (region.height as usize);
                    let mut coeff_buf = vec![0i32; buf_size];

                    // Apply ZYWRLE wavelet preprocessing
                    let result = if let Some(transformed_data) = encoding::zywrle_analyze(
                        &pixel_data,
                        region.width as usize,
                        region.height as usize,
                        level,
                        &mut coeff_buf,
                    ) {
                        // Translate the wavelet-transformed data to client format
                        let translated = if client_pixel_format.is_compatible_with_rgba32() {
                            // Fast path: no translation, but still need to strip alpha
                            let mut buf = BytesMut::with_capacity(
                                (region.width as usize * region.height as usize) * 4,
                            );
                            for chunk in transformed_data.chunks_exact(4) {
                                buf.put_u8(chunk[0]); // R
                                buf.put_u8(chunk[1]); // G
                                buf.put_u8(chunk[2]); // B
                                buf.put_u8(0); // Padding (not alpha)
                            }
                            buf
                        } else {
                            // Translate from server format (RGBA32) to client's requested format
                            translate::translate_pixels(
                                &transformed_data,
                                &server_format,
                                &client_pixel_format,
                            )
                        };

                        // Now encode the translated data with ZRLE (shares the ZRLE compressor)
                        let mut zrle_lock = self.zrle_compressor.write().await;
                        if zrle_lock.is_none() {
                            *zrle_lock = Some(Compress::new(
                                Compression::new(u32::from(compression_level)),
                                true,
                            ));
                            #[cfg(feature = "debug-logging")]
                            info!(
                            "Initialized ZRLE compressor for ZYWRLE with level {compression_level}"
                        );
                        }
                        let zrle_comp = zrle_lock.as_mut().unwrap();

                        // Use client's pixel format for encoding
                        match encoding::encode_zrle_persistent(
                            &translated,
                            region.width,
                            region.height,
                            &client_pixel_format,
                            zrle_comp,
                        ) {
                            Ok(data) => (ENCODING_ZYWRLE, BytesMut::from(&data[..])),
                            Err(e) => {
                                error!("ZYWRLE encoding failed: {e}, falling back to RAW");
                                #[cfg(feature = "debug-logging")]
                                {
                                    encoding_name = "RAW";
                                }
                                // translated already contains the correctly formatted data
                                (ENCODING_RAW, translated)
                            }
                        }
                    } else {
                        // Analysis failed (dimensions too small), fall back to RAW with translation
                        error!(
                            "ZYWRLE analysis failed (dimensions too small), falling back to RAW"
                        );
                        #[cfg(feature = "debug-logging")]
                        {
                            encoding_name = "RAW";
                        }
                        // Translate original pixel_data for RAW fallback
                        let translated = if client_pixel_format.is_compatible_with_rgba32() {
                            let mut buf = BytesMut::with_capacity(
                                (region.width as usize * region.height as usize) * 4,
                            );
                            for chunk in pixel_data.chunks_exact(4) {
                                buf.put_u8(chunk[0]); // R
                                buf.put_u8(chunk[1]); // G
                                buf.put_u8(chunk[2]); // B
                                buf.put_u8(0); // Padding
                            }
                            buf
                        } else {
                            translate::translate_pixels(
                                &pixel_data,
                                &server_format,
                                &client_pixel_format,
                            )
                        };
                        (ENCODING_RAW, translated)
                    };
                    result
                } else if let Some(encoder) = encoding::get_encoder(preferred_encoding) {
                    // For other encodings (TightPng, Hextile): translate first then encode
                    let translated = if client_pixel_format.is_compatible_with_rgba32() {
                        // Fast path: no translation, but still need to strip alpha
                        let mut buf = BytesMut::with_capacity(
                            (region.width as usize * region.height as usize) * 4,
                        );
                        for chunk in pixel_data.chunks_exact(4) {
                            buf.put_u8(chunk[0]); // R
                            buf.put_u8(chunk[1]); // G
                            buf.put_u8(chunk[2]); // B
                            buf.put_u8(0); // Padding (not alpha)
                        }
                        buf
                    } else {
                        // Translate from server format (RGBA32) to client's requested format
                        translate::translate_pixels(
                            &pixel_data,
                            &server_format,
                            &client_pixel_format,
                        )
                    };
                    (
                        preferred_encoding,
                        encoder.encode(
                            &translated,
                            region.width,
                            region.height,
                            jpeg_quality,
                            compression_level,
                        ),
                    )
                } else {
                    // Fallback to RAW encoding if preferred encoding is not available
                    error!("Encoding {preferred_encoding} not available, falling back to RAW");
                    #[cfg(feature = "debug-logging")]
                    {
                        encoding_name = "RAW"; // Update encoding name to reflect fallback
                    }
                    // Translate for RAW fallback
                    let translated = if client_pixel_format.is_compatible_with_rgba32() {
                        let mut buf = BytesMut::with_capacity(
                            (region.width as usize * region.height as usize) * 4,
                        );
                        for chunk in pixel_data.chunks_exact(4) {
                            buf.put_u8(chunk[0]); // R
                            buf.put_u8(chunk[1]); // G
                            buf.put_u8(chunk[2]); // B
                            buf.put_u8(0); // Padding
                        }
                        buf
                    } else {
                        translate::translate_pixels(
                            &pixel_data,
                            &server_format,
                            &client_pixel_format,
                        )
                    };
                    (ENCODING_RAW, translated)
                };

                // Write rectangle header with actual encoding used
                let rect = Rectangle {
                    x: region.x,
                    y: region.y,
                    width: region.width,
                    height: region.height,
                    encoding: actual_encoding,
                };
                rect.write_header(&mut response);
                response.extend_from_slice(&encoded);

                total_pixels += u64::from(region.width) * u64::from(region.height);
            }
        }

        // Acquire send mutex to prevent interleaved writes
        #[cfg(feature = "debug-logging")]
        info!("DEBUG: About to send response, total_rects={}, response.len()={}, copy_rect_count={}, modified_regions={}",
            total_rects, response.len(), copy_rect_count, modified_regions_to_send.len());

        let lock = self.send_mutex.lock().await;

        #[cfg(feature = "debug-logging")]
        info!(
            "DEBUG: Acquired send_mutex, calling write_all with {} bytes",
            response.len()
        );

        self.write_stream.lock().await.write_all(&response).await?;

        #[cfg(feature = "debug-logging")]
        info!("DEBUG: write_all completed successfully");

        drop(lock);

        // Reset deferral timer and update last sent time
        self.start_deferring_nanos.store(0, Ordering::Relaxed); // Reset deferral
        *self.last_update_sent.write().await = Instant::now();

        #[cfg(feature = "debug-logging")]
        {
            let elapsed = start.elapsed();
            info!(
                "Sent {} rects ({} CopyRect + {} encoded, {} pixels total) using {} ({} bytes, {}ms encode+send)",
                total_rects, copy_rect_count, modified_regions_to_send.len(), total_pixels, encoding_name, response.len(), elapsed.as_millis()
            );
        }

        Ok(())
    }

    /// Sends a `ServerCutText` message to the client, updating its clipboard.
    ///
    /// # Arguments
    ///
    /// * `text` - The string to be sent as the clipboard content.
    ///
    /// # Returns
    ///
    /// `Ok(())` on successful transmission, or `Err(std::io::Error)` if an I/O error occurs.
    #[allow(clippy::cast_possible_truncation)] // Clipboard text length limited to u32 per VNC protocol
    pub async fn send_cut_text(&mut self, text: String) -> Result<(), std::io::Error> {
        let mut msg = BytesMut::new();
        msg.put_u8(SERVER_MSG_SERVER_CUT_TEXT);
        msg.put_bytes(0, 3); // padding
        msg.put_u32(text.len() as u32);
        msg.put_slice(text.as_bytes());

        // Acquire send mutex to prevent interleaved writes
        let _lock = self.send_mutex.lock().await;
        self.write_stream.lock().await.write_all(&msg).await?;
        Ok(())
    }

    /// Returns the unique client ID assigned by the server.
    pub fn get_client_id(&self) -> usize {
        self.client_id
    }

    /// Returns a clone of the Arc containing the write half of the TCP stream.
    ///
    /// This allows external code to close the write half directly for shutdown,
    /// which will cause reads on the read half to fail naturally.
    pub fn get_write_stream_handle(
        &self,
    ) -> Arc<tokio::sync::Mutex<Box<dyn tokio::io::AsyncWrite + Unpin + Send + Sync>>> {
        self.write_stream.clone()
    }

    /// Returns the remote host address of the connected client.
    pub fn get_remote_host(&self) -> &str {
        self.remote_host.as_deref().unwrap_or("unknown")
    }

    /// Returns the destination port for repeater connections.
    /// Returns -1 for direct connections (not using a repeater).
    pub fn get_destination_port(&self) -> i32 {
        self.destination_port.map_or(-1, i32::from)
    }

    /// Returns the repeater ID if this client is connected via a repeater.
    /// Returns None for direct connections.
    pub fn get_repeater_id(&self) -> Option<&str> {
        self.repeater_id.as_deref()
    }

    /// Sets the connection metadata for reverse connections.
    pub fn set_connection_metadata(&mut self, destination_port: Option<u16>) {
        self.destination_port = destination_port;
    }

    /// Sets the repeater metadata for repeater connections.
    pub fn set_repeater_metadata(&mut self, repeater_id: String, destination_port: Option<u16>) {
        self.repeater_id = Some(repeater_id);
        self.destination_port = destination_port;
    }
}

/// Ensures proper cleanup when `VncClient` is dropped.
///
/// When `VncClient` is dropped, the read half of the TCP stream (`read_stream: OwnedReadHalf`)
/// is automatically closed because it's an owned field. This completes the client disconnect
/// sequence after the write half has been closed separately during shutdown.
///
/// The log message helps diagnose the shutdown sequence by confirming when `VncClient`
/// objects are actually being dropped and their TCP read streams are closing.
impl Drop for VncClient {
    fn drop(&mut self) {
        #[cfg(feature = "debug-logging")]
        log::info!(
            "VncClient {} is being dropped (read half will close now)",
            self.client_id
        );
    }
}
