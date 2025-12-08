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

//! VNC server implementation for managing client connections and framebuffer distribution.
//!
//! This module provides the main VNC server functionality, including:
//! - TCP listener for incoming client connections
//! - Client session management
//! - Event routing between clients and the application layer
//! - VNC repeater support for reverse connections
//!
//! # Architecture
//!
//! The server uses an event-driven architecture where:
//! - Each client runs in its own asynchronous task
//! - Client events (keyboard, mouse, clipboard) are forwarded to the application via channels
//! - The framebuffer automatically notifies all clients of screen changes
//! - Server events (connect/disconnect) are emitted for the application to handle

use log::error;
#[cfg(feature = "debug-logging")]
use log::info;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, RwLock};

use crate::client::{ClientEvent, VncClient};
use crate::framebuffer::{DirtyRegionReceiver, Framebuffer};
use crate::repeater;

/// Global atomic counter for assigning unique client IDs.
///
/// This counter is incremented for each new client connection to ensure
/// each client has a unique identifier throughout the server's lifetime.
static NEXT_CLIENT_ID: AtomicU64 = AtomicU64::new(1);

/// Represents a VNC server instance.
///
/// This struct manages the VNC framebuffer, connected clients, and handles server-wide events.
#[derive(Clone)]
pub struct VncServer {
    /// The VNC framebuffer, representing the remote desktop screen.
    framebuffer: Framebuffer,
    /// The name of the desktop, displayed to connected clients.
    desktop_name: String,
    /// Optional password for client authentication.
    password: Option<String>,
    /// A list of currently connected VNC clients, protected by a `RwLock` for concurrent access.
    clients: Arc<RwLock<Vec<Arc<RwLock<VncClient>>>>>,
    /// Write stream handles for direct socket shutdown
    client_write_streams:
        Arc<RwLock<Vec<Arc<tokio::sync::Mutex<Box<dyn tokio::io::AsyncWrite + Unpin + Send + Sync>>>>>>,
    /// Task handles for waiting on client threads to exit
    client_tasks: Arc<RwLock<Vec<tokio::task::JoinHandle<()>>>>,
    /// List of active client IDs for fast lookup during shutdown without locking `VncClient` objects.
    ///
    /// This list is maintained separately from the `clients` list to allow the shutdown process
    /// to quickly retrieve all client IDs without acquiring locks on potentially busy `VncClient`
    /// objects, which could cause delays or deadlocks during server shutdown.
    client_ids: Arc<RwLock<Vec<usize>>>,
    /// Sender for server-wide events, used to notify external components of VNC server activity.
    event_tx: mpsc::UnboundedSender<ServerEvent>,
}

/// Enum representing various events that can occur within the VNC server.
pub enum ServerEvent {
    /// A new client has connected to the VNC server.
    ClientConnected {
        /// The unique identifier for the newly connected client
        client_id: usize,
    },
    /// A client has disconnected from the VNC server.
    ClientDisconnected {
        /// The unique identifier for the disconnected client
        client_id: usize,
    },
    /// A key press or release event was received from a client.
    KeyPress {
        /// The unique identifier of the client that sent the event
        client_id: usize,
        /// Boolean indicating if the key was pressed (`true`) or released (`false`)
        down: bool,
        /// The VNC keysym value of the key
        key: u32,
    },
    /// A pointer (mouse) movement or button event was received from a client.
    PointerMove {
        /// The unique identifier of the client that sent the event
        client_id: usize,
        /// The X coordinate of the pointer
        x: u16,
        /// The Y coordinate of the pointer
        y: u16,
        /// A bitmask indicating the state of mouse buttons
        button_mask: u8,
    },
    /// Cut text (clipboard) data was received from a client.
    CutText {
        /// The unique identifier of the client that sent the event
        client_id: usize,
        /// The cut text string
        text: String,
    },
}

impl VncServer {
    /// Creates a new `VncServer` instance.
    ///
    /// This function initializes the framebuffer, sets up desktop name and password, and prepares
    /// channels for server events.
    ///
    /// # Arguments
    ///
    /// * `width` - The width of the VNC framebuffer.
    /// * `height` - The height of the VNC framebuffer.
    /// * `desktop_name` - The name of the desktop to be advertised to clients.
    /// * `password` - An optional password for client authentication.
    ///
    /// # Returns
    ///
    /// A tuple containing:
    /// * The `VncServer` instance itself.
    /// * An `mpsc::UnboundedReceiver<ServerEvent>` to receive events generated by the server.
    #[must_use]
    pub fn new(
        width: u16,
        height: u16,
        desktop_name: String,
        password: Option<String>,
    ) -> (Self, mpsc::UnboundedReceiver<ServerEvent>) {
        let framebuffer = Framebuffer::new(width, height);
        let (event_tx, event_rx) = mpsc::unbounded_channel();

        let server = Self {
            framebuffer,
            desktop_name,
            password,
            clients: Arc::new(RwLock::new(Vec::new())),
            client_write_streams: Arc::new(RwLock::new(Vec::new())),
            client_tasks: Arc::new(RwLock::new(Vec::new())),
            client_ids: Arc::new(RwLock::new(Vec::new())),
            event_tx,
        };

        (server, event_rx)
    }

    /// Starts the VNC server, listening for incoming client connections on the specified port.
    ///
    /// This function enters an infinite loop, accepting new TCP connections and spawning
    /// a new asynchronous task to handle each client.
    ///
    /// # Arguments
    ///
    /// * `port` - The TCP port on which the server will listen for connections.
    ///
    /// # Returns
    ///
    /// `Ok(())` if the server starts successfully and listens indefinitely.
    ///
    /// # Errors
    ///
    /// Returns `Err(std::io::Error)` if there is an issue binding to the port or accepting connections.
    #[allow(clippy::cast_possible_truncation)] // Client ID counter limited to u64::MAX, safe on 64-bit platforms
    pub async fn listen(&self, port: u16) -> Result<(), std::io::Error> {
        let listener = TcpListener::bind(format!("0.0.0.0:{port}")).await?;
        log::info!("VNC Server listening on port {port}");

        loop {
            match listener.accept().await {
                Ok((stream, addr)) => {
                    #[cfg(feature = "debug-logging")]
                    info!("New VNC client connection from: {addr}");

                    // Safely increment client ID counter and check for overflow
                    let client_id_raw = NEXT_CLIENT_ID.fetch_add(1, Ordering::SeqCst);
                    if client_id_raw == 0 || client_id_raw >= u64::MAX - 1000 {
                        error!("Client ID counter overflow, rejecting connection from {addr}");
                        continue;
                    }
                    let client_id = client_id_raw as usize;

                    let framebuffer = self.framebuffer.clone();
                    let desktop_name = self.desktop_name.clone();
                    let password = self.password.clone();
                    let clients = self.clients.clone();
                    let client_write_streams = self.client_write_streams.clone();
                    let client_tasks = self.client_tasks.clone();
                    let client_tasks_for_spawn = client_tasks.clone();
                    let client_ids = self.client_ids.clone();
                    let server_event_tx = self.event_tx.clone();

                    let handle = tokio::spawn(async move {
                        if let Err(e) = Self::handle_client(
                            stream,
                            client_id,
                            framebuffer,
                            desktop_name,
                            password,
                            clients,
                            client_write_streams,
                            client_tasks_for_spawn,
                            client_ids,
                            server_event_tx,
                        )
                        .await
                        {
                            error!("Client {client_id} error: {e}");
                        }
                    });

                    // Store the handle_client task handle for joining later
                    client_tasks.write().await.push(handle);
                }
                Err(e) => {
                    error!("Error accepting connection: {e}");
                }
            }
        }
    }

    /// Accept a VNC client connection from a generic stream.
    ///
    /// This method allows accepting VNC connections from any stream that implements
    /// `AsyncRead + AsyncWrite + Unpin + Send`, such as TCP, UDP with reliability layer,
    /// WebSocket, or other custom transports.
    ///
    /// # Arguments
    ///
    /// * `stream` - A stream implementing `AsyncRead + AsyncWrite + Unpin + Send`
    /// * `client_id` - Optional client ID. If None, a new ID will be generated.
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` if the client was successfully handled, or an `std::io::Error` on failure.
    pub async fn from_socket<S>(
        &self,
        stream: S,
        client_id: Option<usize>,
    ) -> Result<(), std::io::Error>
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + Sync + 'static,
    {
        // Generate client ID if not provided
        let client_id = client_id.unwrap_or_else(|| {
            let client_id_raw = NEXT_CLIENT_ID.fetch_add(1, Ordering::SeqCst);
            if client_id_raw == 0 || client_id_raw >= u64::MAX - 1000 {
                // Wrap around to 1 if overflow
                NEXT_CLIENT_ID.store(1, Ordering::SeqCst);
                1
            } else {
                client_id_raw as usize
            }
        });

        let framebuffer = self.framebuffer.clone();
        let desktop_name = self.desktop_name.clone();
        let password = self.password.clone();
        let clients = self.clients.clone();
        let client_write_streams = self.client_write_streams.clone();
        let client_tasks = self.client_tasks.clone();
        let client_ids = self.client_ids.clone();
        let server_event_tx = self.event_tx.clone();

        let handle = tokio::spawn(async move {
            if let Err(e) = Self::handle_client(
                stream,
                client_id,
                framebuffer,
                desktop_name,
                password,
                clients,
                client_write_streams,
                client_tasks,
                client_ids,
                server_event_tx,
            )
            .await
            {
                error!("Client {client_id} error: {e}");
            }
        });

        // Store the handle_client task handle for joining later
        self.client_tasks.write().await.push(handle);
        Ok(())
    }

    /// Handles a newly connected VNC client through its entire lifecycle.
    ///
    /// This function performs the VNC handshake, creates a `VncClient` instance, spawns
    /// a message handler task, and processes client events until disconnection. It stores
    /// all necessary handles (task handles, write streams, client IDs) to enable proper
    /// cleanup during server shutdown.
    ///
    /// # Arguments
    ///
    /// * `stream` - The TCP stream for the connected client
    /// * `client_id` - Unique identifier assigned to this client
    /// * `framebuffer` - The framebuffer to send to the client
    /// * `desktop_name` - Name of the desktop session
    /// * `password` - Optional password for authentication
    /// * `clients` - Shared list of all connected `VncClient` instances
    /// * `client_write_streams` - Shared list of write stream handles for socket shutdown
    /// * `client_tasks` - Shared list of task handles for cleanup during shutdown
    /// * `client_ids` - Shared list of client IDs for fast lookup during shutdown
    /// * `server_event_tx` - Channel for sending server events (connect/disconnect/input)
    ///
    /// # Returns
    ///
    /// `Ok(())` when the client disconnects normally, or `Err` if an I/O error occurs.
    #[allow(clippy::too_many_arguments)] // VNC protocol handler requires all shared server state
    async fn handle_client<S>(
        stream: S,
        client_id: usize,
        framebuffer: Framebuffer,
        desktop_name: String,
        password: Option<String>,
        clients: Arc<RwLock<Vec<Arc<RwLock<VncClient>>>>>,
        client_write_streams: Arc<
            RwLock<Vec<Arc<tokio::sync::Mutex<Box<dyn tokio::io::AsyncWrite + Unpin + Send + Sync>>>>>,
        >,
        client_tasks: Arc<RwLock<Vec<tokio::task::JoinHandle<()>>>>,
        client_ids: Arc<RwLock<Vec<usize>>>,
        server_event_tx: mpsc::UnboundedSender<ServerEvent>,
    ) -> Result<(), std::io::Error>
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + Sync + 'static,
    {
        let (client_event_tx, mut client_event_rx) = mpsc::unbounded_channel();

        let client = VncClient::new(
            client_id,
            stream,
            framebuffer.clone(),
            desktop_name,
            password,
            client_event_tx,
        )
        .await?;

        let client_arc = Arc::new(RwLock::new(client));

        // Register client to receive dirty region notifications (standard VNC protocol style)
        let regions_arc = client_arc.read().await.get_receiver_handle();
        let receiver = DirtyRegionReceiver::new(Arc::downgrade(&regions_arc));
        framebuffer.register_receiver(receiver).await;

        // Store the write stream handle for direct socket shutdown
        let write_stream_handle = {
            let client = client_arc.read().await;
            client.get_write_stream_handle()
        };
        client_write_streams.write().await.push(write_stream_handle);

        clients.write().await.push(client_arc.clone());
        client_ids.write().await.push(client_id);

        let _ = server_event_tx.send(ServerEvent::ClientConnected { client_id });

        // Spawn task to handle client messages and store handle for joining
        // Note: The message handler holds a write lock for its duration, which means
        // operations like send_cut_text() will wait for the lock. This is acceptable
        // since clipboard operations are infrequent and the async lock prevents deadlocks.
        let client_arc_clone = client_arc.clone();
        let msg_handle = tokio::spawn(async move {
            let result = {
                let mut client: tokio::sync::RwLockWriteGuard<'_, VncClient> = client_arc_clone.write().await;
                client.handle_messages().await
            };
            if let Err(e) = result {
                error!("Client {client_id} message handling error: {e}");
            }
        });

        // Store the message handler task handle for joining later
        client_tasks.write().await.push(msg_handle);

        // Handle client events
        while let Some(event) = client_event_rx.recv().await {
            match event {
                ClientEvent::KeyPress { down, key } => {
                    let _ = server_event_tx.send(ServerEvent::KeyPress {
                        client_id,
                        down,
                        key,
                    });
                }
                ClientEvent::PointerMove { x, y, button_mask } => {
                    let _ = server_event_tx.send(ServerEvent::PointerMove {
                        client_id,
                        x,
                        y,
                        button_mask,
                    });
                }
                ClientEvent::CutText { text } => {
                    let _ = server_event_tx.send(ServerEvent::CutText { client_id, text });
                }
                ClientEvent::Disconnected => {
                    break;
                }
            }
        }

        // Remove client from list
        let mut clients_guard = clients.write().await;
        clients_guard.retain(|c| !Arc::ptr_eq(c, &client_arc));
        drop(clients_guard);

        let mut client_ids_guard = client_ids.write().await;
        client_ids_guard.retain(|&id| id != client_id);
        drop(client_ids_guard);

        let _ = server_event_tx.send(ServerEvent::ClientDisconnected { client_id });

        log::info!("Client {client_id} disconnected");
        Ok(())
    }

    /// Returns a reference to the server's `Framebuffer`.
    ///
    /// This allows external components to inspect or modify the framebuffer content.
    ///
    /// # Returns
    ///
    /// A reference to the `Framebuffer` instance.
    #[must_use]
    pub fn framebuffer(&self) -> &Framebuffer {
        &self.framebuffer
    }

    /// Returns a mutable reference to the server's `Framebuffer`.
    ///
    /// This allows external components to modify the framebuffer, including resizing.
    ///
    /// # Returns
    ///
    /// A mutable reference to the `Framebuffer` instance.
    #[allow(dead_code)]
    pub fn framebuffer_mut(&mut self) -> &mut Framebuffer {
        &mut self.framebuffer
    }

    /// Sends the provided cut text (clipboard) to all currently connected VNC clients.
    ///
    /// # Arguments
    ///
    /// * `text` - The string content to be sent as cut text.
    ///
    /// # Returns
    ///
    /// `Ok(())` if the text is successfully queued for sending to all clients.
    ///
    /// # Errors
    ///
    /// Returns `Err(std::io::Error)` if an error occurs during the sending process to any client.
    pub async fn send_cut_text_to_all(&self, text: String) -> Result<(), std::io::Error> {
        // Clone the client list before iterating to avoid holding read lock
        // This prevents deadlock with client message handlers
        let clients_snapshot = {
            let clients = self.clients.read().await;
            clients.clone()
        };

        for client in &clients_snapshot {
            let mut client_guard = client.write().await;
            let _ = client_guard.send_cut_text(text.clone()).await;
        }
        Ok(())
    }

    /// Establishes a direct reverse VNC connection to a client viewer.
    ///
    /// This method initiates an outbound TCP connection to a VNC viewer listening
    /// for reverse connections. The function spawns a background task to handle the
    /// connection lifecycle, including performing the VNC handshake, spawning a message
    /// handler task, and processing client events. Task handles, write stream handles,
    /// and client IDs are stored for proper cleanup during server shutdown.
    ///
    /// # Arguments
    ///
    /// * `host` - The hostname or IP address of the VNC viewer.
    /// * `port` - The port on which the VNC viewer is listening.
    ///
    /// # Returns
    ///
    /// `Ok(client_id)` if the reverse connection is successfully established.
    ///
    /// # Errors
    ///
    /// Returns `Err(std::io::Error)` if the connection fails or a client ID overflow occurs.
    #[allow(clippy::too_many_lines)] // VNC reverse connection protocol requires complete handshake and error handling
    #[allow(clippy::cast_possible_truncation)] // Client ID counter limited to u64::MAX, safe on 64-bit platforms
    pub async fn connect_reverse(&self, host: String, port: u16) -> Result<usize, std::io::Error> {
        // Safely increment client ID counter and check for overflow
        let client_id_raw = NEXT_CLIENT_ID.fetch_add(1, Ordering::SeqCst);
        if client_id_raw == 0 || client_id_raw >= u64::MAX - 1000 {
            return Err(std::io::Error::other("Client ID counter overflow"));
        }
        let client_id = client_id_raw as usize;

        #[cfg(feature = "debug-logging")]
        info!("Initiating reverse VNC connection to {host}:{port}");

        let framebuffer = self.framebuffer.clone();
        let desktop_name = self.desktop_name.clone();
        let password = self.password.clone();
        let clients = self.clients.clone();
        let client_write_streams = self.client_write_streams.clone();
        let client_tasks = self.client_tasks.clone();
        let client_ids = self.client_ids.clone();
        let server_event_tx = self.event_tx.clone();

        // Use oneshot channel to wait for connection result before returning
        let (result_tx, result_rx) = tokio::sync::oneshot::channel();

        tokio::spawn(async move {
            let (client_event_tx, mut client_event_rx) = mpsc::unbounded_channel();

            // Establish direct TCP connection to the viewer
            let connection_result = TcpStream::connect(format!("{host}:{port}")).await;

            match connection_result {
                Ok(stream) => {
                    #[cfg(feature = "debug-logging")]
                    info!("TCP connection established to {host}:{port}");

                    // Create VNC client for this reverse connection
                    let client_result = VncClient::new(
                        client_id,
                        stream,
                        framebuffer.clone(),
                        desktop_name,
                        password,
                        client_event_tx,
                    )
                    .await;

                    // Send connection result back to caller
                    let _ = result_tx.send(
                        client_result
                            .as_ref()
                            .map(|_| ())
                            .map_err(|e| std::io::Error::new(e.kind(), e.to_string())),
                    );

                    match client_result {
                        Ok(client) => {
                            log::info!("Reverse connection {client_id} established");

                            let client_arc = Arc::new(RwLock::new(client));

                            // Set connection metadata for client management APIs
                            client_arc.write().await.set_connection_metadata(Some(port));

                            // Register client to receive dirty region notifications
                            let regions_arc = client_arc.read().await.get_receiver_handle();
                            let receiver = DirtyRegionReceiver::new(Arc::downgrade(&regions_arc));
                            framebuffer.register_receiver(receiver).await;

                            // Store the write stream handle for direct socket shutdown
                            let write_stream_handle = {
                                let client_guard = client_arc.read().await;
                                client_guard.get_write_stream_handle()
                            };
                            client_write_streams.write().await.push(write_stream_handle);

                            clients.write().await.push(client_arc.clone());
                            client_ids.write().await.push(client_id);

                            let _ = server_event_tx.send(ServerEvent::ClientConnected { client_id });

                            // Spawn task to handle client messages
                            let client_arc_clone = client_arc.clone();
                            let msg_handle = tokio::spawn(async move {
                                let result = {
                                    let mut client: tokio::sync::RwLockWriteGuard<'_, VncClient> = client_arc_clone.write().await;
                                    client.handle_messages().await
                                };
                                if let Err(e) = result {
                                    error!("Client {client_id} error: {e}");
                                }
                            });
                            client_tasks.write().await.push(msg_handle);

                            // Handle client events
                            while let Some(event) = client_event_rx.recv().await {
                                match event {
                                    ClientEvent::KeyPress { down, key } => {
                                        let _ = server_event_tx.send(ServerEvent::KeyPress {
                                            client_id,
                                            down,
                                            key,
                                        });
                                    }
                                    ClientEvent::PointerMove { x, y, button_mask } => {
                                        let _ = server_event_tx.send(ServerEvent::PointerMove {
                                            client_id,
                                            x,
                                            y,
                                            button_mask,
                                        });
                                    }
                                    ClientEvent::CutText { text } => {
                                        let _ = server_event_tx
                                            .send(ServerEvent::CutText { client_id, text });
                                    }
                                    ClientEvent::Disconnected => {
                                        break;
                                    }
                                }
                            }

                            // Remove client from list
                            let mut clients_guard = clients.write().await;
                            clients_guard.retain(|c| !Arc::ptr_eq(c, &client_arc));
                            drop(clients_guard);

                            let mut client_ids_guard = client_ids.write().await;
                            client_ids_guard.retain(|&id| id != client_id);
                            drop(client_ids_guard);

                            let _ =
                                server_event_tx.send(ServerEvent::ClientDisconnected { client_id });

                            log::info!("Reverse client {client_id} disconnected");
                        }
                        Err(e) => {
                            error!("Failed to initialize VNC client for reverse connection: {e}");
                        }
                    }
                }
                Err(e) => {
                    error!("Failed to connect to {host}:{port}: {e}");
                    let _ = result_tx.send(Err(e));
                }
            }
        });

        // Wait for connection to complete before returning to caller
        match result_rx.await {
            Ok(Ok(())) => Ok(client_id),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(std::io::Error::other(
                "Reverse connection task died unexpectedly",
            )),
        }
    }

    /// Connects the VNC server to a VNC repeater, establishing a reverse connection.
    ///
    /// This allows a client behind a NAT or firewall to connect to the server through a VNC
    /// repeater proxy. The function spawns a background task to handle the connection lifecycle,
    /// including performing the repeater handshake, VNC handshake, spawning a message handler task,
    /// and processing client events. Task handles, write stream handles, and client IDs are stored
    /// for proper cleanup during server shutdown.
    ///
    /// The function waits for the repeater connection to be established before returning the
    /// client ID to the caller.
    ///
    /// # Arguments
    ///
    /// * `repeater_host` - The hostname or IP address of the VNC repeater.
    /// * `repeater_port` - The port of the VNC repeater.
    /// * `repeater_id` - The ID to use when connecting to the repeater.
    ///
    /// # Returns
    ///
    /// `Ok(client_id)` if the connection to the repeater is successfully established, where `client_id`
    /// is the unique identifier assigned to the new repeater client.
    ///
    /// # Errors
    ///
    /// Returns `Err(std::io::Error)` if a client ID counter overflow occurs, or if there is an issue
    /// connecting to the repeater or handling the client.
    #[allow(clippy::too_many_lines)] // VNC repeater protocol requires Mode-2 handshake and complete error handling
    #[allow(clippy::cast_possible_truncation)] // Client ID counter limited to u64::MAX, safe on 64-bit platforms
    pub async fn connect_repeater(
        &self,
        repeater_host: String,
        repeater_port: u16,
        repeater_id: String,
    ) -> Result<usize, std::io::Error> {
        // Safely increment client ID counter and check for overflow
        let client_id_raw = NEXT_CLIENT_ID.fetch_add(1, Ordering::SeqCst);
        if client_id_raw == 0 || client_id_raw >= u64::MAX - 1000 {
            return Err(std::io::Error::other("Client ID counter overflow"));
        }
        let client_id = client_id_raw as usize;

        let framebuffer = self.framebuffer.clone();
        let desktop_name = self.desktop_name.clone();
        let password = self.password.clone();
        let clients = self.clients.clone();
        let client_write_streams = self.client_write_streams.clone();
        let client_tasks = self.client_tasks.clone();
        let client_ids = self.client_ids.clone();
        let server_event_tx = self.event_tx.clone();

        // Use oneshot channel to wait for connection result before returning
        let (result_tx, result_rx) = tokio::sync::oneshot::channel();

        tokio::spawn(async move {
            let (client_event_tx, mut client_event_rx) = mpsc::unbounded_channel();

            let connection_result = repeater::connect_repeater(
                client_id,
                repeater_host,
                repeater_port,
                repeater_id,
                framebuffer.clone(),
                desktop_name,
                password,
                client_event_tx,
            )
            .await;

            // Send connection result back to caller
            let _ = result_tx.send(
                connection_result
                    .as_ref()
                    .map(|_| ())
                    .map_err(|e| std::io::Error::new(e.kind(), e.to_string())),
            );

            match connection_result {
                Ok(client) => {
                    log::info!("Repeater connection {client_id} established");

                    let client_arc = Arc::new(RwLock::new(client));

                    // Register client to receive dirty region notifications (standard VNC protocol style)
                    let regions_arc = client_arc.read().await.get_receiver_handle();
                    let receiver = DirtyRegionReceiver::new(Arc::downgrade(&regions_arc));
                    framebuffer.register_receiver(receiver).await;

                    // Store the write stream handle for direct socket shutdown
                    let write_stream_handle = {
                        let client = client_arc.read().await;
                        client.get_write_stream_handle()
                    };
                    client_write_streams.write().await.push(write_stream_handle);

                    clients.write().await.push(client_arc.clone());
                    client_ids.write().await.push(client_id);

                    let _ = server_event_tx.send(ServerEvent::ClientConnected { client_id });

                    // Spawn task to handle client messages
                    // Note: Same write lock behavior as regular clients (see handle_client)
                    let client_arc_clone = client_arc.clone();
                    let msg_handle = tokio::spawn(async move {
                        let result = {
                            let mut client: tokio::sync::RwLockWriteGuard<'_, VncClient> = client_arc_clone.write().await;
                            client.handle_messages().await
                        };
                        if let Err(e) = result {
                            error!("Repeater client {client_id} message handling error: {e}");
                        }
                    });

                    // Store the message handler task handle
                    client_tasks.write().await.push(msg_handle);

                    // Handle client events
                    while let Some(event) = client_event_rx.recv().await {
                        match event {
                            ClientEvent::KeyPress { down, key } => {
                                let _ = server_event_tx.send(ServerEvent::KeyPress {
                                    client_id,
                                    down,
                                    key,
                                });
                            }
                            ClientEvent::PointerMove { x, y, button_mask } => {
                                let _ = server_event_tx.send(ServerEvent::PointerMove {
                                    client_id,
                                    x,
                                    y,
                                    button_mask,
                                });
                            }
                            ClientEvent::CutText { text } => {
                                let _ =
                                    server_event_tx.send(ServerEvent::CutText { client_id, text });
                            }
                            ClientEvent::Disconnected => {
                                break;
                            }
                        }
                    }

                    // Remove client from list
                    let mut clients_guard = clients.write().await;
                    clients_guard.retain(|c| !Arc::ptr_eq(c, &client_arc));
                    drop(clients_guard);

                    let mut client_ids_guard = client_ids.write().await;
                    client_ids_guard.retain(|&id| id != client_id);
                    drop(client_ids_guard);

                    let _ = server_event_tx.send(ServerEvent::ClientDisconnected { client_id });

                    log::info!("Repeater client {client_id} disconnected");
                }
                Err(e) => {
                    error!("Failed to connect to repeater: {e}");
                }
            }
        });

        // Wait for connection to complete before returning to caller
        match result_rx.await {
            Ok(Ok(())) => Ok(client_id),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(std::io::Error::other(
                "Repeater connection task died unexpectedly",
            )),
        }
    }

    /// Finds a client by its ID.
    ///
    /// This method searches through all connected clients to find the one
    /// with the specified ID.
    ///
    /// # Arguments
    ///
    /// * `client_id` - The client ID to search for.
    ///
    /// # Returns
    ///
    /// `Some(Arc<RwLock<VncClient>>)` if the client is found, `None` otherwise.
    pub async fn find_client(&self, client_id: usize) -> Option<Arc<RwLock<VncClient>>> {
        let clients = self.clients.read().await;
        for client in clients.iter() {
            let client_guard = client.read().await;
            if client_guard.get_client_id() == client_id {
                drop(client_guard); // Release read lock before returning
                return Some(client.clone());
            }
        }
        None
    }

    /// Disconnects a specific client by its ID.
    ///
    /// This method forcibly closes the TCP connection for the specified client,
    /// which will cause the client's message handler to exit and trigger cleanup.
    ///
    /// # Arguments
    ///
    /// * `client_id` - The client ID to disconnect.
    ///
    /// # Returns
    ///
    /// `true` if the client was found and disconnected, `false` if not found.
    pub async fn disconnect_client(&self, client_id: usize) -> bool {
        // Find and remove the client from the list
        let mut clients = self.clients.write().await;
        let initial_len = clients.len();

        // Find the client with matching ID and remove it
        clients.retain(|client_arc| {
            // We can't use async in retain closure, so we use try_read() instead
            // This is safe because we hold the write lock on the clients list
            if let Ok(client_guard) = client_arc.try_read() {
                client_guard.get_client_id() != client_id
            } else {
                // If we can't acquire read lock, keep the client (don't remove it)
                true
            }
        });

        let removed = clients.len() < initial_len;
        drop(clients); // Explicitly release write lock

        if removed {
            #[cfg(feature = "debug-logging")]
            info!("Client {client_id} removed from server client list");
        }

        removed
    }

    /// Attempts to acquire a read lock on the clients list without blocking.
    ///
    /// This is used by JNI methods to avoid blocking the main thread and causing ANR.
    /// If the lock cannot be acquired immediately, returns an error.
    ///
    /// # Returns
    ///
    /// `Ok(RwLockReadGuard)` if the lock was acquired.
    ///
    /// # Errors
    ///
    /// Returns `Err(TryLockError)` if the lock could not be acquired immediately.
    pub fn clients_try_read(
        &self,
    ) -> Result<
        tokio::sync::RwLockReadGuard<'_, Vec<Arc<RwLock<VncClient>>>>,
        tokio::sync::TryLockError,
    > {
        self.clients.try_read()
    }

    /// Attempts to acquire a write lock on the clients list without blocking.
    ///
    /// This is used by JNI methods to avoid blocking the main thread and causing ANR.
    /// If the lock cannot be acquired immediately, returns an error.
    ///
    /// # Returns
    ///
    /// `Ok(RwLockWriteGuard)` if the lock was acquired.
    ///
    /// # Errors
    ///
    /// Returns `Err(TryLockError)` if the lock could not be acquired immediately.
    pub fn clients_try_write(
        &self,
    ) -> Result<
        tokio::sync::RwLockWriteGuard<'_, Vec<Arc<RwLock<VncClient>>>>,
        tokio::sync::TryLockError,
    > {
        self.clients.try_write()
    }

    /// Gets a snapshot of all connected client IDs without locking `VncClient` objects.
    ///
    /// This method is safe to call from JNI without risk of ANR because it only
    /// locks the lightweight `client_ids` list, not the heavy `VncClient` objects.
    ///
    /// # Returns
    ///
    /// `Ok(Vec<usize>)` containing all active client IDs.
    ///
    /// # Errors
    ///
    /// Returns `Err(TryLockError)` if the lock cannot be acquired.
    pub fn get_client_ids(&self) -> Result<Vec<usize>, tokio::sync::TryLockError> {
        match self.client_ids.try_read() {
            Ok(guard) => Ok(guard.clone()),
            Err(e) => Err(e),
        }
    }

    /// Disconnects all connected clients by cleanly shutting down their tasks and TCP connections.
    ///
    /// This method performs a coordinated shutdown sequence to ensure both halves of each client's
    /// TCP connection are properly closed. The order of operations is critical to avoid orphaned
    /// connections:
    ///
    /// 1. **Abort all client tasks** - Signals all message handler and event handler tasks to cancel
    /// 2. **Wait for tasks to exit** - Blocks until tasks complete, ensuring their `Arc<VncClient>`
    ///    references are dropped
    /// 3. **Clear client lists** - Removes the final `Arc<VncClient>` references from the server's
    ///    client list, causing `VncClient` to drop and automatically close the read half of the
    ///    TCP connection
    /// 4. **Close write halves** - Explicitly calls `shutdown()` on the write halves to close the
    ///    write side of the TCP connection
    ///
    /// After this sequence completes, both sides of the TCP connection are closed and the client
    /// will receive a disconnect notification.
    ///
    /// # Notes
    ///
    /// - The caller should wrap this in a timeout to prevent indefinite blocking
    /// - The caller is responsible for calling Java-side cleanup (e.g., removing cursors) before
    ///   invoking this method
    /// - All client IDs, task handles, and write streams are cleared from their respective lists
    pub async fn disconnect_all_clients(&self) {
        use tokio::io::AsyncWriteExt;

        // Get both tasks and write streams
        let (tasks_to_abort, write_streams_to_close) = {
            let mut tasks = self.client_tasks.write().await;
            let mut streams = self.client_write_streams.write().await;
            (std::mem::take(&mut *tasks), std::mem::take(&mut *streams))
        };

        let count = tasks_to_abort.len();
        if count > 0 {
            #[cfg(feature = "debug-logging")]
            info!("Disconnecting {count} client(s)");

            // Step 1: Abort all tasks
            #[cfg(feature = "debug-logging")]
            info!("Aborting {count} client task(s)");
            for task in &tasks_to_abort {
                task.abort();
            }

            // Step 2: Wait for tasks to exit (ensures task's Arc<VncClient> is dropped)
            #[cfg(feature = "debug-logging")]
            info!("Waiting for {count} client task(s) to exit");
            for task in tasks_to_abort {
                let _ = task.await;
            }
            #[cfg(feature = "debug-logging")]
            info!("All client tasks exited");

            // Step 3: Clear client lists (drops last Arc<VncClient>, VncClient drops, read half closes)
            #[cfg(feature = "debug-logging")]
            info!("Clearing client list to drop VncClient objects");
            {
                let mut clients = self.clients.write().await;
                clients.clear();
            }
            {
                let mut client_ids = self.client_ids.write().await;
                client_ids.clear();
            }

            // Step 4: Close all write halves (write half closes, TCP fully closed)
            #[cfg(feature = "debug-logging")]
            info!(
                "Closing {} client write stream(s)",
                write_streams_to_close.len()
            );
            for write_stream_arc in write_streams_to_close {
                let mut write_stream = write_stream_arc.lock().await;
                let _ = write_stream.shutdown().await;
            }
        } else {
            // No active tasks, but still clear lists
            let mut clients = self.clients.write().await;
            clients.clear();
            drop(clients);

            let mut client_ids = self.client_ids.write().await;
            client_ids.clear();
            drop(client_ids);
        }

        #[cfg(feature = "debug-logging")]
        info!("All clients disconnected");
    }

    /// Schedules a copy rectangle operation for all connected clients (standard VNC protocol style).
    ///
    /// This method iterates through all clients and schedules the specified region to be
    /// sent using `CopyRect` encoding. This is the equivalent of standard VNC protocol's
    /// `rfbScheduleCopyRect` function.
    ///
    /// # Arguments
    ///
    /// * `x` - The X coordinate of the destination rectangle.
    /// * `y` - The Y coordinate of the destination rectangle.
    /// * `width` - The width of the rectangle.
    /// * `height` - The height of the rectangle.
    /// * `dx` - The X offset from destination to source (`src_x` = `dest_x` + dx).
    /// * `dy` - The Y offset from destination to source (`src_y` = `dest_y` + dy).
    pub async fn schedule_copy_rect(
        &self,
        x: u16,
        y: u16,
        width: u16,
        height: u16,
        dx: i16,
        dy: i16,
    ) {
        use crate::framebuffer::DirtyRegion;

        let region = DirtyRegion::new(x, y, width, height);

        // Clone client list to avoid holding lock during iteration
        let clients_snapshot = {
            let clients = self.clients.read().await;
            clients.clone()
        };

        // Schedule copy for all clients (standard VNC protocol: rfbGetClientIterator pattern)
        for client_arc in &clients_snapshot {
            let client = client_arc.read().await;
            client.schedule_copy_region(region, dx, dy).await;
        }
    }

    /// Performs a copy rectangle operation in the framebuffer and schedules it for all clients.
    ///
    /// This method first copies the specified region within the framebuffer memory,
    /// then schedules the copy operation to be sent to all connected clients.
    /// This is the equivalent of standard VNC protocol's `rfbDoCopyRect` function.
    ///
    /// # Arguments
    ///
    /// * `x` - The X coordinate of the destination rectangle.
    /// * `y` - The Y coordinate of the destination rectangle.
    /// * `width` - The width of the rectangle.
    /// * `height` - The height of the rectangle.
    /// * `dx` - The X offset from destination to source (`src_x` = `dest_x` + dx).
    /// * `dy` - The Y offset from destination to source (`src_y` = `dest_y` + dy).
    ///
    /// # Returns
    ///
    /// `Ok(())` if the operation is successful.
    ///
    /// # Errors
    ///
    /// Returns `Err(String)` if the rectangle is out of bounds.
    pub async fn do_copy_rect(
        &self,
        x: u16,
        y: u16,
        width: u16,
        height: u16,
        dx: i16,
        dy: i16,
    ) -> Result<(), String> {
        // Perform actual framebuffer copy
        self.framebuffer
            .do_copy_region(x, y, width, height, dx, dy)
            .await?;

        // Schedule copy for all clients
        self.schedule_copy_rect(x, y, width, height, dx, dy).await;

        Ok(())
    }
}
