//! Headless VNC server example with animated content.
//!
//! This example creates a VNC server that continuously updates the framebuffer
//! with animated content, demonstrating how to use the server in a headless
//! environment without actual screen capture.
//!
//! Usage:
//!   cargo run --example headless_server

use rustvncserver::{VncServer, ServerEvent};
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

    let server = VncServer::new(WIDTH, HEIGHT);

    // Start server in background
    let server_clone = server.clone();
    tokio::spawn(async move {
        if let Err(e) = server_clone.listen(5900).await {
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
        server.update_framebuffer(&pixels, 0, 0, WIDTH, HEIGHT);

        // Next frame
        frame = frame.wrapping_add(1);

        // ~30 FPS
        time::sleep(Duration::from_millis(33)).await;

        if frame % 300 == 0 {
            println!("Frame: {}", frame);
        }
    }
}
