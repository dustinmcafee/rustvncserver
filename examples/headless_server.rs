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

//! Headless VNC server example with animated content.
//!
//! This example creates a VNC server that continuously updates the framebuffer
//! with animated content, demonstrating how to use the server in a headless
//! environment without actual screen capture.
//!
//! Usage:
//!   cargo run --example headless_server

use rustvncserver::VncServer;
use std::error::Error;
use std::time::Duration;
use tokio::time;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    env_logger::init();

    println!("Starting headless VNC server on port 5900...");
    println!("Connect with: vncviewer localhost:5900");

    const WIDTH: u16 = 640;
    const HEIGHT: u16 = 480;

    let (server, mut events) = VncServer::new(
        WIDTH,
        HEIGHT,
        "RustVNC Animated".to_string(),
        None, // No password
    );

    // Handle events in background
    tokio::spawn(async move {
        while let Some(event) = events.recv().await {
            match event {
                rustvncserver::server::ServerEvent::ClientConnected { client_id } => {
                    println!("Client {} connected", client_id);
                }
                rustvncserver::server::ServerEvent::ClientDisconnected { client_id } => {
                    println!("Client {} disconnected", client_id);
                }
                _ => {}
            }
        }
    });

    // Get framebuffer reference before moving server
    let framebuffer = server.framebuffer().clone();

    // Start server in background
    tokio::spawn(async move {
        if let Err(e) = server.listen(5900).await {
            eprintln!("Server error: {}", e);
        }
    });

    println!("Server started, generating animated content...");
    println!("Press Ctrl+C to stop");

    // Animation loop
    let mut frame = 0u32;
    let mut pixels = vec![0u8; (WIDTH as usize) * (HEIGHT as usize) * 4];

    loop {
        // Generate animated pattern
        for y in 0..HEIGHT {
            for x in 0..WIDTH {
                let offset = ((y as usize) * (WIDTH as usize) + (x as usize)) * 4;

                // Animated gradient
                let r = ((x as u32 + frame) % 256) as u8;
                let g = ((y as u32 + frame) % 256) as u8;
                let b = ((frame / 2) % 256) as u8;

                pixels[offset] = r;
                pixels[offset + 1] = g;
                pixels[offset + 2] = b;
                pixels[offset + 3] = 255; // Alpha
            }
        }

        // Update framebuffer
        framebuffer
            .update_cropped(&pixels, 0, 0, WIDTH, HEIGHT)
            .await
            .expect("Failed to update framebuffer");

        // Next frame
        frame = frame.wrapping_add(1);

        // ~30 FPS
        time::sleep(Duration::from_millis(33)).await;

        if frame.is_multiple_of(300) {
            println!("Frame: {}", frame);
        }
    }
}
