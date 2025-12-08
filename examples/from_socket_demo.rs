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

//! Simple demonstration of using `from_socket` to accept VNC connections
//! from any stream that implements `AsyncRead + AsyncWrite + Unpin + Send + Sync`.
//!
//! This example shows how to:
//! 1. Create a VNC server
//! 2. Accept TCP connections using `from_socket`
//! 3. Handle different types of streams
//!
//! Usage:
//!   cargo run --example from_socket_demo
//!
//! Then connect with a VNC viewer to localhost:5900

use rustvncserver::VncServer;
use std::error::Error;
use std::sync::Arc;
use tokio::net::TcpListener;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    // Initialize logging
    env_logger::init();

    println!("VNC Server with from_socket() demonstration");
    println!("===========================================");

    // Create VNC server
    let (server, mut events) = VncServer::new(
        800,
        600,
        "from_socket Demo".to_string(),
        None, // No password
    );
    let server = Arc::new(server);

    // Handle server events in background
    let _server_for_events = Arc::clone(&server);
    tokio::spawn(async move {
        while let Some(event) = events.recv().await {
            match event {
                rustvncserver::server::ServerEvent::ClientConnected { client_id } => {
                    println!("[Event] Client {} connected", client_id);
                }
                rustvncserver::server::ServerEvent::ClientDisconnected { client_id } => {
                    println!("[Event] Client {} disconnected", client_id);
                }
                rustvncserver::server::ServerEvent::KeyPress { client_id, down, key } => {
                    let action = if down { "pressed" } else { "released" };
                    println!("[Event] Client {} key {} {}", client_id, key, action);
                }
                rustvncserver::server::ServerEvent::PointerMove { client_id, x, y, button_mask } => {
                    println!("[Event] Client {} pointer at ({}, {}) buttons: {:08b}",
                             client_id, x, y, button_mask);
                }
                rustvncserver::server::ServerEvent::CutText { client_id, text } => {
                    let preview = if text.len() > 20 {
                        format!("{}...", &text[..20])
                    } else {
                        text.clone()
                    };
                    println!("[Event] Client {} sent clipboard: {}", client_id, preview);
                }
            }
        }
    });

    // Create a simple test pattern
    let mut pixels = vec![0u8; 800 * 600 * 4];
    for y in 0..600 {
        for x in 0..800 {
            let offset = (y * 800 + x) * 4;
            pixels[offset] = (x * 255 / 800) as u8;     // Red gradient
            pixels[offset + 1] = (y * 255 / 600) as u8; // Green gradient
            pixels[offset + 2] = 128;                   // Blue constant
            pixels[offset + 3] = 255;                   // Alpha opaque
        }
    }

    // Update framebuffer
    server
        .framebuffer()
        .update_cropped(&pixels, 0, 0, 800, 600)
        .await
        .expect("Failed to update framebuffer");

    println!("Framebuffer initialized with test pattern");

    // Create TCP listener
    let listener = TcpListener::bind("127.0.0.1:5900").await?;
    println!("TCP listener ready on port 5900");
    println!("Connect with: vncviewer localhost:5900");
    println!("Waiting for connections...");

    // Accept connections and handle them using from_socket
    loop {
        match listener.accept().await {
            Ok((stream, addr)) => {
                println!("New connection from {}", addr);

                // Use from_socket to handle the connection
                let server_clone = Arc::clone(&server);
                tokio::spawn(async move {
                    match server_clone.from_socket(stream, None).await {
                        Ok(()) => {
                            println!("Connection from {} handled successfully", addr);
                        }
                        Err(e) => {
                            eprintln!("Failed to handle connection from {}: {}", addr, e);
                        }
                    }
                });
            }
            Err(e) => {
                eprintln!("Error accepting connection: {}", e);
            }
        }
    }
}
