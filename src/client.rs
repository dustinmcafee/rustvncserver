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
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};
use tokio::sync::RwLock;
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use log::{info, error};
use flate2::Compress;
use flate2::Compression;

use crate::vnc::protocol::*;
use crate::vnc::framebuffer::{Framebuffer, DirtyRegion};
use crate::vnc::auth::VncAuth;
use crate::vnc::encoding;
use crate::vnc::translate;


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

/// Manages a single VNC client connection, handling communication, framebuffer updates,
/// and client input events.
///
/// This struct encapsulates the state and logic for interacting with a connected VNC viewer.
/// It is responsible for sending framebuffer updates to the client based on dirty regions,
/// processing incoming client messages (e.g., key events, pointer events, pixel format requests),
/// and managing client-specific settings like preferred encodings and JPEG quality.
pub struct VncClient {
    /// The underlying TCP stream for communication with the VNC client.
    stream: TcpStream,
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
    /// A flag indicating whether the client has requested continuous framebuffer updates, stored as an `AtomicBool`.
    continuous_updates: AtomicBool, // Atomic - simple bool flag
    /// A shared, locked vector of `DirtyRegion`s specific to this client.
    /// These regions represent areas of the framebuffer that have been modified and need to be sent to the client.
    modified_regions: Arc<RwLock<Vec<DirtyRegion>>>, // Per-client dirty regions (libvncserver style - receives pushes from framebuffer)
    /// The region specifically requested by the client for an update, protected by a `RwLock`.
    /// It is written by the message handler and read by the encoder.
    requested_region: RwLock<Option<DirtyRegion>>, // Protected - written by message handler, read by encoder
    /// CopyRect tracking (libvncserver style): destination regions to be copied
    copy_region: Arc<RwLock<Vec<DirtyRegion>>>, // Destination regions for CopyRect
    /// Translation vector for CopyRect: (dx, dy) where src = dest + (dx, dy)
    copy_offset: RwLock<Option<(i16, i16)>>, // (dx, dy) translation for copy operations
    /// The duration to defer sending updates, matching `libvncserver`'s default.
    defer_update_time: Duration, // Constant - set once at init
    /// The timestamp (in nanoseconds since creation) when deferring of updates began (0 if not deferring).
    /// Stored as an `AtomicU64` for atomic access.
    start_deferring_nanos: AtomicU64, // Atomic - nanos since creation (0 = not deferring)
    /// The `Instant` when this `VncClient` instance was created, used for calculating elapsed time.
    creation_time: Instant, // Constant - for calculating elapsed time
    /// The maximum number of rectangles to send in a single framebuffer update message, matching `libvncserver`'s default.
    max_rects_per_update: usize, // Constant - set once at init
    /// A mutex used to ensure exclusive access to the client's `TcpStream` for sending data,
    /// preventing interleaved writes from concurrent tasks.
    send_mutex: Arc<tokio::sync::Mutex<()>>,
    /// Persistent zlib compressor for Zlib encoding (RFC 6143: one stream per connection).
    /// Protected by RwLock since encoding happens during send_batched_update.
    zlib_compressor: RwLock<Option<Compress>>,
    /// Persistent zlib compressor for ZlibHex encoding (RFC 6143: one stream per connection).
    /// Protected by RwLock since encoding happens during send_batched_update.
    zlibhex_compressor: RwLock<Option<Compress>>,
    /// Persistent zlib compressor for ZRLE encoding (RFC 6143: one stream per connection).
    /// Protected by RwLock since encoding happens during send_batched_update.
    #[allow(dead_code)]
    zrle_compressor: RwLock<Option<Compress>>,
    /// ZYWRLE quality level (0 = disabled, 1-3 = quality levels, higher = better quality).
    /// Stored as AtomicU8 for atomic access. Updated based on client's quality setting.
    zywrle_level: AtomicU8, // Atomic - updated when ZYWRLE encoding is detected
    /// Remote host address (IP:port) of the connected client
    remote_host: String,
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
    /// * `stream` - The `TcpStream` representing the established connection to the VNC client.
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
    pub async fn new(
        client_id: usize,
        mut stream: TcpStream,
        framebuffer: Framebuffer,
        desktop_name: String,
        password: Option<String>,
        event_tx: mpsc::UnboundedSender<ClientEvent>,
    ) -> Result<Self, std::io::Error> {
        // Capture remote host address before handshake
        let remote_host = stream.peer_addr()
            .map(|addr| addr.to_string())
            .unwrap_or_else(|_| "unknown".to_string());

        // Disable Nagle's algorithm for immediate frame delivery
        stream.set_nodelay(true)?;

        // Send protocol version
        stream.write_all(PROTOCOL_VERSION.as_bytes()).await?;

        // Read client protocol version
        let mut version_buf = vec![0u8; 12];
        stream.read_exact(&mut version_buf).await?;
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

        info!("VNC client handshake completed");

        let creation_time = Instant::now();

        Ok(Self {
            stream,
            framebuffer,
            pixel_format: RwLock::new(PixelFormat::rgba32()),
            encodings: RwLock::new(vec![ENCODING_RAW]),
            event_tx,
            last_update_sent: RwLock::new(creation_time),
            jpeg_quality: AtomicU8::new(80), // Default quality
            compression_level: AtomicU8::new(6), // Default zlib compression (balanced)
            continuous_updates: AtomicBool::new(false),
            modified_regions: Arc::new(RwLock::new(Vec::new())),
            requested_region: RwLock::new(None),
            copy_region: Arc::new(RwLock::new(Vec::new())), // Initialize empty copy region
            copy_offset: RwLock::new(None), // No copy offset initially
            defer_update_time: Duration::from_millis(5), // Match libvncserver default
            start_deferring_nanos: AtomicU64::new(0), // 0 = not deferring
            creation_time,
            max_rects_per_update: 50, // Match libvncserver default
            send_mutex: Arc::new(tokio::sync::Mutex::new(())),
            zlib_compressor: RwLock::new(None), // Initialized lazily when first used
            zlibhex_compressor: RwLock::new(None), // Initialized lazily when first used
            zrle_compressor: RwLock::new(None), // Initialized lazily when first used
            zywrle_level: AtomicU8::new(0), // Disabled by default, updated when ZYWRLE is requested
            remote_host,
            destination_port: None, // None for direct inbound connections
            repeater_id: None, // None for direct inbound connections
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

    /// Schedules a copy operation for this client (libvncserver style).
    ///
    /// This method adds a region to be sent using CopyRect encoding with the specified offset.
    /// According to libvncserver's algorithm, if a copy operation with a different offset
    /// already exists, the old copy region is treated as modified.
    ///
    /// # Arguments
    ///
    /// * `region` - The destination region to be copied.
    /// * `dx` - The X offset from destination to source (src_x = dest_x + dx).
    /// * `dy` - The Y offset from destination to source (src_y = dest_y + dy).
    pub async fn schedule_copy_region(&self, region: DirtyRegion, dx: i16, dy: i16) {
        let mut copy_regions = self.copy_region.write().await;
        let mut copy_offset = self.copy_offset.write().await;
        let mut modified_regions = self.modified_regions.write().await;

        // Check if we have an existing copy with a different offset
        if let Some((existing_dx, existing_dy)) = *copy_offset {
            if existing_dx != dx || existing_dy != dy {
                // Different offset - treat existing copy region as modified
                // This matches libvncserver's behavior in rfbScheduleCopyRegion
                modified_regions.extend(copy_regions.drain(..));
                copy_regions.clear();
            }
        }

        // Add the new region to copy_region
        copy_regions.push(region);
        *copy_offset = Some((dx, dy));
    }

    /// Enters the main message loop for the VncClient, handling incoming data from the client
    /// and periodically sending framebuffer updates.
    ///
    /// This function continuously reads from the client's `TcpStream` and processes VNC messages
    /// such as `SetPixelFormat`, `SetEncodings`, `FramebufferUpdateRequest`, `KeyEvent`,
    /// `PointerEvent`, and `ClientCutText`. It also uses a `tokio::time::interval` to
    /// periodically check if batched framebuffer updates should be sent to the client,
    /// based on dirty regions and deferral logic.
    ///
    /// # Returns
    ///
    /// `Ok(())` if the client disconnects gracefully.
    /// Returns `Err(std::io::Error)` if an I/O error occurs or an invalid message is received.
    pub async fn handle_messages(&mut self) -> Result<(), std::io::Error> {
        let mut buf = BytesMut::with_capacity(4096);
        let mut check_interval = tokio::time::interval(tokio::time::Duration::from_millis(16)); // Check for updates ~60 times/sec

        loop {
            tokio::select! {
                // Handle incoming client messages
                result = self.stream.read_buf(&mut buf) => {
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
                                let compatible = requested_format.is_compatible_with_rgba32();
                                *self.pixel_format.write().await = requested_format.clone();

                                if compatible {
                                    info!("Client set pixel format: RGBA32 (no translation needed)");
                                } else {
                                    info!(
                                        "Client set pixel format: {}bpp, depth {}, R{}:{}  G{}:{} B{}:{} (will translate from RGBA32)",
                                        requested_format.bits_per_pixel,
                                        requested_format.depth,
                                        requested_format.red_shift, requested_format.red_max,
                                        requested_format.green_shift, requested_format.green_max,
                                        requested_format.blue_shift, requested_format.blue_max
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
                                        // Use libvncserver's quality mapping (TigerVNC compatible)
                                        // Reference: libvncserver/src/libvncserver/rfbserver.c:109
                                        const TIGHT2TURBO_QUAL: [u8; 10] = [15, 29, 41, 42, 62, 77, 79, 86, 92, 100];
                                        let quality = TIGHT2TURBO_QUAL[quality_level as usize];
                                        self.jpeg_quality.store(quality, Ordering::Relaxed);
                                        info!("Client requested quality level {}, using JPEG quality {}", quality_level, quality);
                                    }

                                    // Check for compression level pseudo-encodings (-256 to -247)
                                    if (ENCODING_COMPRESS_LEVEL_0..=ENCODING_COMPRESS_LEVEL_9).contains(&encoding) {
                                        // -256 = level 0 (lowest/fastest), -247 = level 9 (highest/slowest)
                                        let compression_level = (encoding - ENCODING_COMPRESS_LEVEL_0) as u8;
                                        // Use compression level directly (0=fastest, 9=best compression)
                                        self.compression_level.store(compression_level, Ordering::Relaxed);
                                        info!("Client requested compression level {}, using zlib level {}", compression_level, compression_level);
                                    }
                                }
                                *self.encodings.write().await = encodings_list.clone();
                                info!("Client set {} encodings: {:?}", count, encodings_list);
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

                                info!("FramebufferUpdateRequest: incremental={}, region=({},{} {}x{})", incremental, x, y, width, height);

                                // Track requested region (libvncserver cl->requestedRegion)
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

                                // Limit clipboard size to prevent memory exhaustion attacks
                                const MAX_CUT_TEXT: usize = 10 * 1024 * 1024; // 10MB limit
                                if length > MAX_CUT_TEXT {
                                    error!("Cut text too large: {} bytes (max {}), disconnecting client", length, MAX_CUT_TEXT);
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
                                error!("Unknown message type: {}, disconnecting client", msg_type);
                                let _ = self.event_tx.send(ClientEvent::Disconnected);
                                return Err(std::io::Error::new(
                                    std::io::ErrorKind::InvalidData,
                                    format!("Unknown message type: {}", msg_type)
                                ));
                            }
                        }
                    }
                }

                // Periodically check if we should send updates (libvncserver style)
                _ = check_interval.tick() => {
                    let continuous = self.continuous_updates.load(Ordering::Relaxed);
                    if continuous {
                        // Check if we have regions and deferral time has elapsed
                        // Regions are already pushed to us by framebuffer (no merge needed!)
                        let should_send = {
                            let regions = self.modified_regions.read().await;
                            if !regions.is_empty() {
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
                            } else {
                                false
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
    /// This function implements libvncserver's update sending algorithm:
    /// 1. Send CopyRect regions first (from copy_region with stored offset)
    /// 2. Then send modified regions (from modified_regions)
    ///
    /// The update includes multiple rectangles in a single message to improve efficiency.
    ///
    /// # Returns
    ///
    /// A `Result` which is `Ok(())` on successful transmission of the update, or
    /// `Err(std::io::Error)` if an I/O error occurs during encoding or sending.
    async fn send_batched_update(&mut self) -> Result<(), std::io::Error> {
        // Get requested region (libvncserver: requestedRegion)
        let requested = *self.requested_region.read().await;

        info!("send_batched_update called, requested region: {:?}", requested);

        // STEP 1: Get copy regions to send (libvncserver: copyRegion sent FIRST)
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

        // STEP 2: Get modified regions to send (libvncserver: modifiedRegion sent AFTER copyRegion)
        let modified_regions_to_send: Vec<DirtyRegion> = {
            let mut regions = self.modified_regions.write().await;

            if regions.is_empty() {
                Vec::new()
            } else {
                // Calculate how many regions we can send
                let remaining_slots = self.max_rects_per_update.saturating_sub(copy_regions_to_send.len());
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
            info!("No regions to send (copy={}, modified={})", copy_regions_to_send.len(), modified_regions_to_send.len());
            return Ok(());
        }

        let start = Instant::now();
        let total_rects = copy_regions_to_send.len() + modified_regions_to_send.len();

        let mut response = BytesMut::new();

        // Message type
        response.put_u8(SERVER_MSG_FRAMEBUFFER_UPDATE);
        response.put_u8(0); // padding
        response.put_u16(total_rects as u16); // number of rectangles

        // Choose best encoding supported by client
        let encodings = self.encodings.read().await;
        // Priority order: TIGHT > TIGHTPNG > ZRLE > ZYWRLE > ZLIBHEX > ZLIB > HEXTILE > RAW
        // This matches libvncserver's typical priority (Tight offers best compression/speed trade-off)
        // ZLIB, ZLIBHEX, ZRLE, and ZYWRLE all use persistent compression (RFC 6143 compliant)
        let preferred_encoding = if encodings.contains(&ENCODING_TIGHT) {
            ENCODING_TIGHT
        } else if encodings.contains(&ENCODING_TIGHTPNG) {
            ENCODING_TIGHTPNG
        } else if encodings.contains(&ENCODING_ZRLE) {
            ENCODING_ZRLE
        } else if encodings.contains(&ENCODING_ZYWRLE) {
            // Update ZYWRLE level based on quality setting (matches libvncserver logic)
            let quality = self.jpeg_quality.load(Ordering::Relaxed);
            let level = if quality < 42 {  // quality_level < 3
                3  // Lowest quality, highest compression
            } else if quality < 79 {  // quality_level < 6
                2  // Medium quality
            } else {
                1  // Highest quality, lowest compression
            };
            self.zywrle_level.store(level, Ordering::Relaxed);
            ENCODING_ZYWRLE
        } else if encodings.contains(&ENCODING_ZLIBHEX) {
            ENCODING_ZLIBHEX
        } else if encodings.contains(&ENCODING_ZLIB) {
            ENCODING_ZLIB
        } else if encodings.contains(&ENCODING_HEXTILE) {
            ENCODING_HEXTILE
        } else {
            ENCODING_RAW
        };
        drop(encodings); // Release lock

        let mut encoding_name = match preferred_encoding {
            ENCODING_TIGHT => "TIGHT",
            ENCODING_TIGHTPNG => "TIGHTPNG",
            ENCODING_ZYWRLE => "ZYWRLE",
            ENCODING_ZRLE => "ZRLE",
            ENCODING_ZLIBHEX => "ZLIBHEX",
            ENCODING_ZLIB => "ZLIB",
            _ => "RAW",
        };

        let mut total_pixels = 0u64;
        let mut copy_rect_count = 0;

        // Load quality/compression settings atomically
        let jpeg_quality = self.jpeg_quality.load(Ordering::Relaxed);
        let compression_level = self.compression_level.load(Ordering::Relaxed);

        // STEP 1: Send copy regions FIRST (libvncserver style)
        if let Some((dx, dy)) = copy_src_offset {
            for region in &copy_regions_to_send {
                // Calculate source position from destination + offset
                // In libvncserver: src = dest + (dx, dy)
                let src_x = (region.x as i32 + dx as i32) as u16;
                let src_y = (region.y as i32 + dy as i32) as u16;

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

                total_pixels += (region.width as u64) * (region.height as u64);
                copy_rect_count += 1;
            }
        }

        // STEP 2: Send modified regions (libvncserver: sent AFTER copy regions)
        for region in &modified_regions_to_send {

            // Get pixel data
            let pixel_data = match self.framebuffer.get_rect(region.x, region.y, region.width, region.height).await {
                Ok(data) => data,
                Err(e) => {
                    error!("Failed to get rectangle ({}, {}, {}, {}): {}",
                           region.x, region.y, region.width, region.height, e);
                    continue; // Skip this invalid rectangle
                }
            };

            // Apply pixel format translation and encode
            // Note: Following libvncserver's approach where translation happens before encoding
            let client_pixel_format = self.pixel_format.read().await;
            let server_format = PixelFormat::rgba32();

            let (actual_encoding, encoded) = if preferred_encoding == ENCODING_RAW {
                // For Raw encoding: translation IS the encoding (like libvncserver)
                // Just translate and send directly, no additional processing
                let translated = if client_pixel_format.is_compatible_with_rgba32() {
                    // Fast path: no translation, but still need to strip alpha
                    let mut buf = BytesMut::with_capacity((region.width as usize * region.height as usize) * 4);
                    for chunk in pixel_data.chunks_exact(4) {
                        buf.put_u8(chunk[0]); // R
                        buf.put_u8(chunk[1]); // G
                        buf.put_u8(chunk[2]); // B
                        buf.put_u8(0);        // Padding (not alpha)
                    }
                    buf
                } else {
                    // Translate from server format (RGBA32) to client's requested format
                    translate::translate_pixels(&pixel_data, &server_format, &*client_pixel_format)
                };
                (ENCODING_RAW, translated)
            } else if preferred_encoding == ENCODING_ZLIB {
                // Translate pixels to client format first (libvncserver: translateFn before encode)
                let translated = if client_pixel_format.is_compatible_with_rgba32() {
                    // Fast path: no translation, but still need to strip alpha
                    let mut buf = BytesMut::with_capacity((region.width as usize * region.height as usize) * 4);
                    for chunk in pixel_data.chunks_exact(4) {
                        buf.put_u8(chunk[0]); // R
                        buf.put_u8(chunk[1]); // G
                        buf.put_u8(chunk[2]); // B
                        buf.put_u8(0);        // Padding (not alpha)
                    }
                    buf
                } else {
                    // Translate from server format (RGBA32) to client's requested format
                    translate::translate_pixels(&pixel_data, &server_format, &*client_pixel_format)
                };

                // Initialize ZLIB compressor lazily on first use
                let mut zlib_lock = self.zlib_compressor.write().await;
                if zlib_lock.is_none() {
                    *zlib_lock = Some(Compress::new(Compression::new(compression_level as u32), true));
                    info!("Initialized ZLIB compressor with level {}", compression_level);
                }
                let zlib_comp = zlib_lock.as_mut().unwrap();

                match encoding::encode_zlib_persistent(&translated, zlib_comp) {
                    Ok(data) => (ENCODING_ZLIB, BytesMut::from(&data[..])),
                    Err(e) => {
                        error!("ZLIB encoding failed: {}, falling back to RAW", e);
                        encoding_name = "RAW";
                        // translated already contains the correctly formatted data
                        (ENCODING_RAW, translated)
                    }
                }
            } else if preferred_encoding == ENCODING_ZLIBHEX {
                // Translate pixels to client format first (libvncserver: translateFn before encode)
                let translated = if client_pixel_format.is_compatible_with_rgba32() {
                    // Fast path: no translation, but still need to strip alpha
                    let mut buf = BytesMut::with_capacity((region.width as usize * region.height as usize) * 4);
                    for chunk in pixel_data.chunks_exact(4) {
                        buf.put_u8(chunk[0]); // R
                        buf.put_u8(chunk[1]); // G
                        buf.put_u8(chunk[2]); // B
                        buf.put_u8(0);        // Padding (not alpha)
                    }
                    buf
                } else {
                    // Translate from server format (RGBA32) to client's requested format
                    translate::translate_pixels(&pixel_data, &server_format, &*client_pixel_format)
                };

                // Initialize ZLIBHEX compressor lazily on first use
                let mut zlibhex_lock = self.zlibhex_compressor.write().await;
                if zlibhex_lock.is_none() {
                    *zlibhex_lock = Some(Compress::new(Compression::new(compression_level as u32), true));
                    info!("Initialized ZLIBHEX compressor with level {}", compression_level);
                }
                let zlibhex_comp = zlibhex_lock.as_mut().unwrap();

                match encoding::encode_zlibhex_persistent(&translated, region.width, region.height, zlibhex_comp) {
                    Ok(data) => (ENCODING_ZLIBHEX, BytesMut::from(&data[..])),
                    Err(e) => {
                        error!("ZLIBHEX encoding failed: {}, falling back to RAW", e);
                        encoding_name = "RAW";
                        // translated already contains the correctly formatted data
                        (ENCODING_RAW, translated)
                    }
                }
            } else if preferred_encoding == ENCODING_ZRLE {
                // Translate pixels to client format first (libvncserver: translateFn before encode)
                let translated = if client_pixel_format.is_compatible_with_rgba32() {
                    // Fast path: no translation, but still need to strip alpha
                    let mut buf = BytesMut::with_capacity((region.width as usize * region.height as usize) * 4);
                    for chunk in pixel_data.chunks_exact(4) {
                        buf.put_u8(chunk[0]); // R
                        buf.put_u8(chunk[1]); // G
                        buf.put_u8(chunk[2]); // B
                        buf.put_u8(0);        // Padding (not alpha)
                    }
                    buf
                } else {
                    // Translate from server format (RGBA32) to client's requested format
                    translate::translate_pixels(&pixel_data, &server_format, &*client_pixel_format)
                };

                // Initialize ZRLE compressor lazily on first use
                let mut zrle_lock = self.zrle_compressor.write().await;
                if zrle_lock.is_none() {
                    *zrle_lock = Some(Compress::new(Compression::new(compression_level as u32), true));
                    info!("Initialized ZRLE compressor with level {}", compression_level);
                }
                let zrle_comp = zrle_lock.as_mut().unwrap();

                // Use client's pixel format for encoding
                match encoding::encode_zrle_persistent(&translated, region.width, region.height, &*client_pixel_format, zrle_comp) {
                    Ok(data) => (ENCODING_ZRLE, BytesMut::from(&data[..])),
                    Err(e) => {
                        error!("ZRLE encoding failed: {}, falling back to RAW", e);
                        encoding_name = "RAW";
                        // translated already contains the correctly formatted data
                        (ENCODING_RAW, translated)
                    }
                }
            } else if preferred_encoding == ENCODING_ZYWRLE {
                // ZYWRLE: Apply wavelet preprocessing then use ZRLE encoder (libvncserver approach)
                let level = self.zywrle_level.load(Ordering::Relaxed) as usize;

                // Allocate coefficient buffer for wavelet transform
                let buf_size = (region.width as usize) * (region.height as usize);
                let mut coeff_buf = vec![0i32; buf_size];

                // Apply ZYWRLE wavelet preprocessing (matches libvncserver's ZYWRLE_ANALYZE)
                let result = if let Some(transformed_data) = encoding::zywrle_analyze(
                    &pixel_data,
                    region.width as usize,
                    region.height as usize,
                    level,
                    &mut coeff_buf
                ) {
                    // Translate the wavelet-transformed data to client format
                    let translated = if client_pixel_format.is_compatible_with_rgba32() {
                        // Fast path: no translation, but still need to strip alpha
                        let mut buf = BytesMut::with_capacity((region.width as usize * region.height as usize) * 4);
                        for chunk in transformed_data.chunks_exact(4) {
                            buf.put_u8(chunk[0]); // R
                            buf.put_u8(chunk[1]); // G
                            buf.put_u8(chunk[2]); // B
                            buf.put_u8(0);        // Padding (not alpha)
                        }
                        buf
                    } else {
                        // Translate from server format (RGBA32) to client's requested format
                        translate::translate_pixels(&transformed_data, &server_format, &*client_pixel_format)
                    };

                    // Now encode the translated data with ZRLE (shares the ZRLE compressor)
                    let mut zrle_lock = self.zrle_compressor.write().await;
                    if zrle_lock.is_none() {
                        *zrle_lock = Some(Compress::new(Compression::new(compression_level as u32), true));
                        info!("Initialized ZRLE compressor for ZYWRLE with level {}", compression_level);
                    }
                    let zrle_comp = zrle_lock.as_mut().unwrap();

                    // Use client's pixel format for encoding
                    match encoding::encode_zrle_persistent(&translated, region.width, region.height, &*client_pixel_format, zrle_comp) {
                        Ok(data) => (ENCODING_ZYWRLE, BytesMut::from(&data[..])),
                        Err(e) => {
                            error!("ZYWRLE encoding failed: {}, falling back to RAW", e);
                            encoding_name = "RAW";
                            // translated already contains the correctly formatted data
                            (ENCODING_RAW, translated)
                        }
                    }
                } else {
                    // Analysis failed (dimensions too small), fall back to RAW with translation
                    error!("ZYWRLE analysis failed (dimensions too small), falling back to RAW");
                    encoding_name = "RAW";
                    // Translate original pixel_data for RAW fallback
                    let translated = if client_pixel_format.is_compatible_with_rgba32() {
                        let mut buf = BytesMut::with_capacity((region.width as usize * region.height as usize) * 4);
                        for chunk in pixel_data.chunks_exact(4) {
                            buf.put_u8(chunk[0]); // R
                            buf.put_u8(chunk[1]); // G
                            buf.put_u8(chunk[2]); // B
                            buf.put_u8(0);        // Padding
                        }
                        buf
                    } else {
                        translate::translate_pixels(&pixel_data, &server_format, &*client_pixel_format)
                    };
                    (ENCODING_RAW, translated)
                };
                result
            } else if let Some(encoder) = encoding::get_encoder(preferred_encoding) {
                // For other encodings (Tight, TightPng, Hextile): translate first then encode
                let translated = if client_pixel_format.is_compatible_with_rgba32() {
                    // Fast path: no translation, but still need to strip alpha
                    let mut buf = BytesMut::with_capacity((region.width as usize * region.height as usize) * 4);
                    for chunk in pixel_data.chunks_exact(4) {
                        buf.put_u8(chunk[0]); // R
                        buf.put_u8(chunk[1]); // G
                        buf.put_u8(chunk[2]); // B
                        buf.put_u8(0);        // Padding (not alpha)
                    }
                    buf
                } else {
                    // Translate from server format (RGBA32) to client's requested format
                    translate::translate_pixels(&pixel_data, &server_format, &*client_pixel_format)
                };
                (preferred_encoding, encoder.encode(&translated, region.width, region.height, jpeg_quality, compression_level))
            } else {
                // Fallback to RAW encoding if preferred encoding is not available
                error!("Encoding {} not available, falling back to RAW", preferred_encoding);
                encoding_name = "RAW"; // Update encoding name to reflect fallback
                // Translate for RAW fallback
                let translated = if client_pixel_format.is_compatible_with_rgba32() {
                    let mut buf = BytesMut::with_capacity((region.width as usize * region.height as usize) * 4);
                    for chunk in pixel_data.chunks_exact(4) {
                        buf.put_u8(chunk[0]); // R
                        buf.put_u8(chunk[1]); // G
                        buf.put_u8(chunk[2]); // B
                        buf.put_u8(0);        // Padding
                    }
                    buf
                } else {
                    translate::translate_pixels(&pixel_data, &server_format, &*client_pixel_format)
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

            total_pixels += (region.width as u64) * (region.height as u64);
        }

        // Acquire send mutex to prevent interleaved writes
        let _lock = self.send_mutex.lock().await;
        self.stream.write_all(&response).await?;
        drop(_lock);

        // Reset deferral timer and update last sent time
        self.start_deferring_nanos.store(0, Ordering::Relaxed); // Reset deferral
        *self.last_update_sent.write().await = Instant::now();

        let elapsed = start.elapsed();
        info!(
            "Sent {} rects ({} CopyRect + {} encoded, {} pixels total) using {} ({} bytes, {}ms encode+send)",
            total_rects, copy_rect_count, modified_regions_to_send.len(), total_pixels, encoding_name, response.len(), elapsed.as_millis()
        );

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
    pub async fn send_cut_text(&mut self, text: String) -> Result<(), std::io::Error> {
        let mut msg = BytesMut::new();
        msg.put_u8(SERVER_MSG_SERVER_CUT_TEXT);
        msg.put_bytes(0, 3); // padding
        msg.put_u32(text.len() as u32);
        msg.put_slice(text.as_bytes());

        // Acquire send mutex to prevent interleaved writes
        let _lock = self.send_mutex.lock().await;
        self.stream.write_all(&msg).await?;
        Ok(())
    }

    /// Returns the unique client ID assigned by the server.
    pub fn get_client_id(&self) -> usize {
        self.client_id
    }

    /// Returns the remote host address of the connected client.
    pub fn get_remote_host(&self) -> &str {
        &self.remote_host
    }

    /// Returns the destination port for repeater connections.
    /// Returns -1 for direct connections (not using a repeater).
    pub fn get_destination_port(&self) -> i32 {
        self.destination_port.map(|p| p as i32).unwrap_or(-1)
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
