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

//! Simple VNC server example.
//!
//! This example creates a VNC server with a static test pattern.
//!
//! Usage:
//!   cargo run --example simple_server
//!
//! Then connect with a VNC viewer to localhost:5900

use rustvncserver::VncServer;
use std::error::Error;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    // Initialize logging
    env_logger::init();

    println!("Starting VNC server on port 5900...");
    println!("Connect with: vncviewer localhost:5900");
    println!("Password: test123");

    // Create server with 800x600 resolution
    let (server, _events) = VncServer::new(
        800,
        600,
        "RustVNC Example".to_string(),
        Some("test123".to_string()),
    );

    // Create a test pattern (gradient)
    let mut pixels = vec![0u8; 800 * 600 * 4]; // RGBA32
    for y in 0..600 {
        for x in 0..800 {
            let offset = (y * 800 + x) * 4;
            pixels[offset] = (x * 255 / 800) as u8; // R: horizontal gradient
            pixels[offset + 1] = (y * 255 / 600) as u8; // G: vertical gradient
            pixels[offset + 2] = 128; // B: constant
            pixels[offset + 3] = 255; // A: opaque
        }
    }

    // Update framebuffer with test pattern
    server
        .framebuffer()
        .update_cropped(&pixels, 0, 0, 800, 600)
        .await
        .expect("Failed to update framebuffer");

    println!("Framebuffer updated with test pattern");
    println!("Server ready for connections");

    // Start server (blocks until Ctrl+C)
    server.listen(5900).await?;

    Ok(())
}
