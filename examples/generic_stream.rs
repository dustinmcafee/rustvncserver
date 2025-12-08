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

//! Generic stream VNC server example.
//!
//! This example demonstrates how to use the VNC server with different types of streams
//! that implement `AsyncRead + AsyncWrite + Unpin + Send`, such as:
//! - TCP streams (standard VNC)
//! - UDP streams with reliability layer
//! - WebSocket connections
//! - Custom transport protocols
//!
//! Usage:
//!   cargo run --example generic_stream
//!
//! This example creates a simple TCP listener and accepts connections using `from_socket`.

use rustvncserver::VncServer;
use std::error::Error;
use std::sync::Arc;
use tokio::net::TcpListener;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    // Initialize logging
    env_logger::init();

    println!("Starting generic stream VNC server example...");
    println!("This example demonstrates using from_socket() with different stream types");

    // Create VNC server
    let (server, mut events) = VncServer::new(
        800,
        600,
        "Generic Stream VNC".to_string(),
        None, // No password
    );
    let server = Arc::new(server);

    // Handle server events in background
    tokio::spawn(async move {
        while let Some(event) = events.recv().await {
            match event {
                rustvncserver::server::ServerEvent::ClientConnected { client_id } => {
                    println!("Client {} connected via generic stream", client_id);
                }
                rustvncserver::server::ServerEvent::ClientDisconnected { client_id } => {
                    println!("Client {} disconnected", client_id);
                }
                rustvncserver::server::ServerEvent::KeyPress { client_id, down, key } => {
                    let action = if down { "pressed" } else { "released" };
                    println!("Client {} key {} {}", client_id, key, action);
                }
                rustvncserver::server::ServerEvent::PointerMove { client_id, x, y, button_mask } => {
                    println!("Client {} pointer moved to ({}, {}) buttons: {:08b}",
                             client_id, x, y, button_mask);
                }
                rustvncserver::server::ServerEvent::CutText { client_id, text } => {
                    println!("Client {} sent cut text: {}...",
                             client_id, text.chars().take(20).collect::<String>());
                }
            }
        }
    });

    // Create a test pattern
    let mut pixels = vec![0u8; 800 * 600 * 4];
    for y in 0..600 {
        for x in 0..800 {
            let offset = (y * 800 + x) * 4;
            pixels[offset] = (x * 255 / 800) as u8;     // R gradient
            pixels[offset + 1] = (y * 255 / 600) as u8; // G gradient
            pixels[offset + 2] = 128;                   // B constant
            pixels[offset + 3] = 255;                   // A opaque
        }
    }

    // Update framebuffer
    server
        .framebuffer()
        .update_cropped(&pixels, 0, 0, 800, 600)
        .await
        .expect("Failed to update framebuffer");

    println!("Framebuffer initialized with test pattern");

    // Example 1: Standard TCP listener using from_socket
    println!("\nExample 1: Standard TCP listener on port 5901");
    let tcp_listener = TcpListener::bind("127.0.0.1:5901").await?;
    println!("TCP listener ready on port 5901");

    let server_clone = Arc::clone(&server);
    tokio::spawn(async move {
        loop {
            match tcp_listener.accept().await {
                Ok((stream, addr)) => {
                    println!("Accepted TCP connection from {}", addr);

                    // Use from_socket to handle the TCP stream
                    if let Err(e) = server_clone.from_socket(stream, None).await {
                        eprintln!("Failed to handle TCP connection: {}", e);
                    }
                }
                Err(e) => {
                    eprintln!("Error accepting TCP connection: {}", e);
                }
            }
        }
    });

    // Example 2: Custom stream wrapper demonstration
    println!("\nExample 2: Custom stream wrapper");
    println!("This shows how you could wrap different transport protocols");

    // Create a simple TCP server on another port to demonstrate
    let server_clone2 = Arc::clone(&server);
    tokio::spawn(async move {
        let listener = match TcpListener::bind("127.0.0.1:5902").await {
            Ok(l) => l,
            Err(e) => {
                eprintln!("Failed to bind port 5902: {}", e);
                return;
            }
        };

        println!("Custom stream server ready on port 5902");

        while let Ok((stream, addr)) = listener.accept().await {
            println!("Custom stream connection from {}", addr);

            // Example: You could wrap the stream here with custom logic
            // For example, add compression, encryption, or protocol translation
            let wrapped_stream = ExampleStreamWrapper::new(stream);

            if let Err(e) = server_clone2.from_socket(wrapped_stream, None).await {
                eprintln!("Failed to handle wrapped stream: {}", e);
            }
        }
    });

    println!("\nServers are running:");
    println!("- Standard VNC on port 5900 (using server.listen())");
    println!("- Generic stream TCP on port 5901 (using from_socket())");
    println!("- Custom wrapped stream on port 5902");
    println!("\nConnect with:");
    println!("  vncviewer localhost:5900");
    println!("  vncviewer localhost:5901");
    println!("  vncviewer localhost:5902");
    println!("\nPress Ctrl+C to stop");

    // Also start the standard listen method for comparison
    let server_ref = Arc::clone(&server);
    tokio::spawn(async move {
        if let Err(e) = server_ref.listen(5900).await {
            eprintln!("Server error: {}", e);
        }
    });

    println!("Servers are running. Press Ctrl+C to stop.");
    println!("Waiting for connections...");

    // Keep main thread alive by waiting for a long time
    tokio::time::sleep(tokio::time::Duration::from_secs(3600)).await;

    Ok(())
}

/// Example stream wrapper that demonstrates how to implement custom transport layers.
///
/// This struct wraps any stream that implements `AsyncRead + AsyncWrite + Unpin`
/// and adds custom behavior (in this case, just logging).
struct ExampleStreamWrapper<S> {
    inner: S,
    bytes_transferred: usize,
}

impl<S> ExampleStreamWrapper<S> {
    fn new(stream: S) -> Self {
        Self {
            inner: stream,
            bytes_transferred: 0,
        }
    }
}

impl<S: tokio::io::AsyncRead + Unpin> tokio::io::AsyncRead for ExampleStreamWrapper<S> {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        let before = buf.filled().len();
        let result = std::pin::Pin::new(&mut self.inner).poll_read(cx, buf);
        let after = buf.filled().len();

        if after > before {
            self.bytes_transferred += after - before;
            println!("Read {} bytes (total: {})", after - before, self.bytes_transferred);
        }

        result
    }
}

impl<S: tokio::io::AsyncWrite + Unpin> tokio::io::AsyncWrite for ExampleStreamWrapper<S> {
    fn poll_write(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<Result<usize, std::io::Error>> {
        let result = std::pin::Pin::new(&mut self.inner).poll_write(cx, buf);

        if let std::task::Poll::Ready(Ok(bytes_written)) = &result {
            self.bytes_transferred += bytes_written;
            println!("Wrote {} bytes (total: {})", bytes_written, self.bytes_transferred);
        }

        result
    }

    fn poll_flush(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), std::io::Error>> {
        std::pin::Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), std::io::Error>> {
        std::pin::Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

// Implement Unpin since S is Unpin
impl<S: Unpin> Unpin for ExampleStreamWrapper<S> {}

// Implement Send since S is Send
unsafe impl<S: Send> Send for ExampleStreamWrapper<S> {}
