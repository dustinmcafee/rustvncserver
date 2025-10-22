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

//! VNC Repeater support for establishing reverse connections.
//!
//! This module implements the UltraVNC-style repeater protocol, which enables VNC servers
//! to connect to clients through an intermediary repeater service. This is particularly
//! useful for scenarios where the VNC server is behind a NAT or firewall and cannot
//! accept direct incoming connections.
//!
//! # Protocol Overview
//!
//! The repeater protocol works as follows:
//! 1. Server connects to the repeater and sends an ID string formatted as "ID:xxxxx"
//! 2. The ID string is padded to exactly 250 bytes with null characters
//! 3. A VNC client connects to the same repeater using the same ID
//! 4. The repeater bridges the two connections
//! 5. Normal VNC protocol handshake proceeds between server and client
//!
//! # Usage
//!
//! This module is typically used through the VNC server's `connect_repeater` method,
//! which handles the repeater handshake and then establishes a normal VNC client session.

use log::error;
#[cfg(feature = "debug-logging")]
use log::info;
use std::io;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio::sync::mpsc;

use crate::client::{ClientEvent, VncClient};
use crate::framebuffer::Framebuffer;

/// Connects to a VNC repeater using the UltraVNC-style repeater protocol.
///
/// This function establishes a reverse VNC connection, which is useful for connecting
/// to clients behind NATs or firewalls. The protocol involves sending a specific
/// ID to the repeater, which then facilitates the connection to a waiting viewer.
///
/// # Arguments
///
/// * `client_id` - The unique client ID assigned by the server.
/// * `repeater_host` - The hostname or IP address of the VNC repeater.
/// * `repeater_port` - The port on which the VNC repeater is listening.
/// * `repeater_id` - The unique ID string to send to the repeater for session identification.
/// * `framebuffer` - The VNC framebuffer instance to be used for the session.
/// * `desktop_name` - The desktop name to be advertised to the connected viewer.
/// * `password` - An optional password for VNC authentication.
/// * `event_tx` - An `mpsc::UnboundedSender<ClientEvent>` to send client-related events.
///
/// # Returns
///
/// `Ok(VncClient)` if the connection to the repeater is successfully established and
/// the VNC handshake completes, returning the initialized `VncClient` instance.
/// Returns `Err(io::Error)` if a network error occurs, the repeater ID is too long,
/// or if the VNC handshake fails.
#[allow(clippy::too_many_arguments)] // VNC repeater connection requires all client configuration parameters
pub async fn connect_repeater(
    client_id: usize,
    repeater_host: String,
    repeater_port: u16,
    repeater_id: String,
    framebuffer: Framebuffer,
    desktop_name: String,
    password: Option<String>,
    event_tx: mpsc::UnboundedSender<ClientEvent>,
) -> Result<VncClient, io::Error> {
    #[cfg(feature = "debug-logging")]
    info!("Connecting to VNC repeater {repeater_host}:{repeater_port} with ID: {repeater_id}");

    // Connect to repeater
    #[cfg(feature = "debug-logging")]
    info!("Attempting TCP connection to {repeater_host}:{repeater_port}...");
    let mut stream = match TcpStream::connect(format!("{repeater_host}:{repeater_port}")).await {
        Ok(s) => {
            #[cfg(feature = "debug-logging")]
            info!("TCP connection established to {repeater_host}:{repeater_port}");
            s
        }
        Err(e) => {
            error!("Failed to establish TCP connection to {repeater_host}:{repeater_port}: {e}");
            return Err(e);
        }
    };

    // Format ID string: "ID:xxxxx" padded to 250 bytes with nulls
    // The repeater protocol expects exactly 250 bytes with the ID string
    // prefixed by "ID:" and the remainder filled with null bytes
    let mut id_buffer = [0u8; 250];
    let id_string = format!("ID:{repeater_id}");

    // Validate ID length - buffer is 250 bytes, so ID string can be up to 250 bytes
    if id_string.len() > 250 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Repeater ID too long (max 246 characters after 'ID:' prefix)",
        ));
    }

    // Copy ID string into buffer (rest remains null)
    id_buffer[..id_string.len()].copy_from_slice(id_string.as_bytes());

    // Send ID to repeater
    #[cfg(feature = "debug-logging")]
    info!("Sending repeater ID: {id_string}");
    if let Err(e) = stream.write_all(&id_buffer).await {
        error!("Failed to send repeater ID to {repeater_host}:{repeater_port}: {e}");
        return Err(e);
    }

    #[cfg(feature = "debug-logging")]
    info!("Repeater ID sent, proceeding with VNC handshake");

    // Now proceed with normal VNC client handshake
    let mut client = VncClient::new(
        client_id,
        stream,
        framebuffer,
        desktop_name,
        password,
        event_tx,
    )
    .await?;

    // Set repeater metadata for client management APIs
    client.set_repeater_metadata(repeater_id, Some(repeater_port));

    #[cfg(feature = "debug-logging")]
    info!("VNC repeater connection established successfully");
    Ok(client)
}
