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
    let server = VncServer::new(800, 600);

    // Set password
    server.set_password(Some("test123".to_string()));

    // Create a test pattern (gradient)
    let mut pixels = vec![0u8; 800 * 600 * 4]; // RGBA32
    for y in 0..600 {
        for x in 0..800 {
            let offset = (y * 800 + x) * 4;
            pixels[offset] = (x * 255 / 800) as u8;     // R: horizontal gradient
            pixels[offset + 1] = (y * 255 / 600) as u8; // G: vertical gradient
            pixels[offset + 2] = 128;                   // B: constant
            pixels[offset + 3] = 255;                   // A: opaque
        }
    }

    // Update framebuffer with test pattern
    server.update_framebuffer(&pixels, 0, 0, 800, 600);

    println!("Framebuffer updated with test pattern");
    println!("Server ready for connections");

    // Start server (blocks until Ctrl+C)
    server.listen(5900).await?;

    Ok(())
}
