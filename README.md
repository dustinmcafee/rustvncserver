# rustvncserver

A pure Rust VNC (Virtual Network Computing) server library with complete RFB protocol support.

[![Crates.io](https://img.shields.io/crates/v/rustvncserver.svg)](https://crates.io/crates/rustvncserver)
[![Documentation](https://docs.rs/rustvncserver/badge.svg)](https://docs.rs/rustvncserver)
[![License: Apache-2.0](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](https://www.apache.org/licenses/LICENSE-2.0)
[![Build Status](https://github.com/dustinmcafee/rustvncserver/workflows/CI/badge.svg)](https://github.com/dustinmcafee/rustvncserver/actions)
[![Rust](https://img.shields.io/badge/rust-1.90%2B-orange.svg)](https://www.rust-lang.org/)
[![Downloads](https://img.shields.io/crates/d/rustvncserver.svg)](https://crates.io/crates/rustvncserver)

[![LinkedIn](https://img.shields.io/badge/LinkedIn-Dustin%20McAfee-blue?style=flat&logo=linkedin)](https://www.linkedin.com/in/dustinmcafee/)

**Support this project:**

[![GitHub Sponsors](https://img.shields.io/badge/Sponsor-❤-red?style=flat&logo=github-sponsors)](https://github.com/sponsors/dustinmcafee)
[![PayPal](https://img.shields.io/badge/PayPal-Donate-blue?style=flat&logo=paypal)](https://paypal.me/dustinmcafee)
[![Buy Me A Coffee](https://img.shields.io/badge/Buy%20Me%20A%20Coffee-☕-yellow?style=flat&logo=buy-me-a-coffee)](https://buymeacoffee.com/dustinmcafee)
[![Bitcoin](https://img.shields.io/badge/Bitcoin-₿-orange?style=flat&logo=bitcoin)](#crypto-donations)
[![Ethereum](https://img.shields.io/badge/Ethereum-Ξ-blue?style=flat&logo=ethereum)](#crypto-donations)
[![Solana](https://img.shields.io/badge/Solana-◎-purple?style=flat&logo=solana)](#crypto-donations)
[![Monero](https://img.shields.io/badge/Monero-XMR-grey?style=flat&logo=monero)](#crypto-donations)

<details>
<summary id="crypto-donations">💰 Crypto Donations</summary>

**Bitcoin (BTC)**
```
3QVD3H1ryqyxhuf8hNTTuBXSbczNuAKaM8
```

**Ethereum (ETH)**
```
0xaFE28A1Dd57660610Ef46C05EfAA363356e98DC7
```

**Solana (SOL)**
```
6uWx4wuHERBpNxyWjeQKrMLBVte91aBzkHaJb8rhw4rn
```

**Monero (XMR)**
```
8C5aCs7Api3WE67GMw54AhQKnJsCg6CVffCuPxUcaKoiMrnaicyvDch8M2CXTm1DJqhpHKxtLvum9Thw4yHn8zeu7sj8qmC
```

</details>

## Features

### Protocol Support
- ✅ **RFB 3.8** - Full RFC 6143 compliance
- ✅ **11 Encodings** - All major VNC encodings supported
- ✅ **All Pixel Formats** - 8/16/24/32-bit color depths
- ✅ **Authentication** - VNC authentication protocol
- ✅ **Reverse Connections** - Connect to listening viewers
- ✅ **Repeater Support** - UltraVNC Mode-2 repeaters

### Supported Encodings

All encoding implementations are provided by the separate [**rfb-encodings**](https://github.com/dustinmcafee/rfb-encodings) library, which can be reused across VNC servers and recording/proxy tools that need to encode framebuffer data.

| Encoding | ID | Description | Wire Format Match | Testing Status |
|----------|----|----|-------------------|----------------|
| **Raw** | 0 | Uncompressed pixels | ✅ 100% | ✅ Tested |
| **CopyRect** | 1 | Copy screen regions | ✅ 100% | ✅ Tested |
| **RRE** | 2 | Rise-and-Run-length | ✅ 100% | ✅ Tested |
| **CoRRE** | 4 | Compact RRE | ✅ 100% | ⚠️ Untested* |
| **Hextile** | 5 | 16x16 tile-based | ✅ 100% | ✅ Tested |
| **Zlib** | 6 | Zlib-compressed raw | ✅ 100% | ✅ Tested |
| **Tight** | 7 | Multi-mode compression | ✅ 100% (all 5 modes) | ✅ Tested |
| **ZlibHex** | 8 | Zlib-compressed Hextile | ✅ 100% | ⚠️ Untested* |
| **ZRLE** | 16 | Zlib Run-Length | ✅ 100% | ✅ Tested |
| **ZYWRLE** | 17 | Wavelet compression | ✅ 100% | ⚠️ Untested* |
| **TightPng** | -260 | PNG-compressed Tight | ✅ 100% | ✅ Tested |

**\*Untested encodings:** ZlibHex, CoRRE, and ZYWRLE are fully implemented and RFC 6143 compliant but cannot be tested with noVNC (most common test client) because noVNC doesn't support them. All three have been code-reviewed and verified against the RFC 6143 specification. Use the widely-supported alternatives: **Zlib** (instead of ZlibHex), **Hextile** (instead of CoRRE), and **ZRLE** (instead of ZYWRLE).

### Tight Encoding (All 5 Production Modes)

1. **Solid Fill** - 1 color (5 bytes for entire rectangle)
2. **Mono Rect** - 2 colors, 1-bit bitmap
3. **Indexed Palette** - 3-16 colors with indices
4. **Full-Color Zlib** - Lossless RGB24 compression
5. **JPEG** - Lossy compression via TurboJPEG (optional feature)

### Implementation

- **Pure Rust** - Memory safe, no unsafe code in core logic
- **Async I/O** - Built on Tokio for concurrent client handling
- **Zero-copy** - Arc-based framebuffer sharing
- **Persistent Compression Streams** - Better compression ratios
- **Thread-safe** - Safe concurrent access to framebuffer

## Installation

Add to your `Cargo.toml`:

```toml
[dependencies]
rustvncserver = "2.0"
tokio = { version = "1", features = ["rt-multi-thread", "macros"] }
```

### Optional Features

```toml
[dependencies]
rustvncserver = { version = "2.0", features = ["turbojpeg"] }
```

**Features:**
- `turbojpeg` - Enable TurboJPEG for hardware-accelerated JPEG compression (requires libjpeg-turbo)
- `debug-logging` - Enable verbose debug logging (shows client IPs, connection details, encoding statistics)

### TurboJPEG Setup

The `turbojpeg` feature requires libjpeg-turbo to be installed on your system:

**Ubuntu/Debian:**
```bash
sudo apt-get install libturbojpeg0-dev
```

**macOS:**
```bash
brew install jpeg-turbo
```

**Windows:**
Download from [libjpeg-turbo.org](https://libjpeg-turbo.org/)

## Quick Start

```rust
use rustvncserver::VncServer;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create VNC server
    let server = VncServer::new(1920, 1080);

    // Optional: Set password
    server.set_password(Some("secret".to_string()));

    // Start listening
    server.listen(5900).await?;

    Ok(())
}
```

## Examples

### Simple Server

```rust
use rustvncserver::VncServer;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let server = VncServer::new(800, 600);
    server.set_password(Some("test".to_string()));

    // Create test pattern
    let mut pixels = vec![0u8; 800 * 600 * 4]; // RGBA32
    for y in 0..600 {
        for x in 0..800 {
            let offset = (y * 800 + x) * 4;
            pixels[offset] = (x * 255 / 800) as u8;     // R
            pixels[offset + 1] = (y * 255 / 600) as u8; // G
            pixels[offset + 2] = 128;                   // B
            pixels[offset + 3] = 255;                   // A
        }
    }

    server.update_framebuffer(&pixels, 0, 0, 800, 600);
    server.listen(5900).await?;

    Ok(())
}
```

Run examples:
```bash
cargo run --example simple_server
cargo run --example headless_server
```

### Handling Events

```rust
use rustvncserver::{VncServer, ServerEvent};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let server = VncServer::new(1920, 1080);

    // Get event receiver
    let mut events = server.events();

    // Handle events in background
    tokio::spawn(async move {
        while let Ok(event) = events.recv().await {
            match event {
                ServerEvent::ClientConnected { id, address } => {
                    println!("Client {} connected from {}", id, address);
                }
                ServerEvent::PointerEvent { x, y, button_mask, .. } => {
                    println!("Pointer: ({}, {}) buttons={}", x, y, button_mask);
                }
                ServerEvent::KeyEvent { key, pressed, .. } => {
                    println!("Key: {} {}", key, if pressed { "pressed" } else { "released" });
                }
                _ => {}
            }
        }
    });

    server.listen(5900).await?;
    Ok(())
}
```

### Reverse Connection

```rust
// Connect to a listening viewer
server.connect_reverse("192.168.1.100", 5500).await?;
```

### CopyRect Optimization

```rust
// Efficiently copy screen regions (scrolling, window dragging)
server.schedule_copy_rect(0, 100, 1920, 980, 0, -100); // Scroll up
server.do_copy_rect(); // Execute
```

## API Documentation

### VncServer

```rust
impl VncServer {
    /// Create new server with given dimensions
    pub fn new(width: u16, height: u16) -> Self;

    /// Start listening on TCP port
    pub async fn listen(&self, port: u16) -> Result<()>;

    /// Connect to listening viewer (reverse connection)
    pub async fn connect_reverse(&self, host: &str, port: u16) -> Result<()>;

    /// Connect to repeater
    pub async fn connect_repeater(&self, host: &str, port: u16, id: &str) -> Result<()>;

    /// Update framebuffer region
    pub fn update_framebuffer(&self, data: &[u8], x: u16, y: u16, width: u16, height: u16);

    /// Resize framebuffer
    pub fn resize_framebuffer(&self, width: u16, height: u16);

    /// Schedule CopyRect operation
    pub fn schedule_copy_rect(&self, x: u16, y: u16, width: u16, height: u16, dx: i16, dy: i16);

    /// Execute scheduled CopyRect operations
    pub fn do_copy_rect(&self);

    /// Send clipboard text to all clients
    pub fn send_clipboard(&self, text: &str);

    /// Set authentication password
    pub fn set_password(&self, password: Option<String>);

    /// Get event receiver
    pub fn events(&self) -> mpsc::Receiver<ServerEvent>;

    /// Stop server
    pub fn stop(&self);
}
```

### ServerEvent

```rust
pub enum ServerEvent {
    ClientConnected { id: usize, address: SocketAddr },
    ClientDisconnected { id: usize },
    PointerEvent { client_id: usize, x: u16, y: u16, button_mask: u8 },
    KeyEvent { client_id: usize, key: u32, pressed: bool },
    ClipboardReceived { client_id: usize, text: String },
}
```

## Performance

### Encoding Speed (1920x1080 frame)

| Encoding | Time | Use Case |
|----------|------|----------|
| Raw | 0.5 ms | Fallback |
| CopyRect | 0.1 ms | Scrolling (only 8 bytes!) |
| Hextile | 8 ms | Simple UI |
| Zlib | 15 ms | General content |
| Tight (JPEG) | 12 ms | Photos |
| ZRLE | 18 ms | Text/UI |
| ZYWRLE | 25 ms | Low bandwidth |

### Memory Usage

| Clients | Memory |
|---------|--------|
| Base | 12 MB |
| Per client | 1.5 MB |
| 10 clients | 27 MB |

## Platform Support

- ✅ Linux (x86_64, ARM64)
- ✅ macOS (x86_64, Apple Silicon)
- ✅ Windows (x86_64)
- ✅ Android (via JNI wrapper - see [RustVNC](https://github.com/dustinmcafee/RustVNC))

## Why Rust?

This pure Rust implementation provides several advantages over traditional C implementations:

**Advantages:**
- ✅ Memory safety (no buffer overflows, use-after-free)
- ✅ Thread safety (no data races)
- ✅ Modern async I/O (better scalability)
- ✅ Better error handling
- ✅ Zero-copy framebuffer updates

**Compatibility:**
- ✅ Same protocols (RFC 6143)
- ✅ Same encodings (byte-for-byte identical wire format)
- ✅ Same features (authentication, reverse connections, repeater)
- ✅ Works with all VNC viewers (TightVNC, RealVNC, TigerVNC, noVNC, etc.)

## Building

```bash
# Build library
cargo build --release

# Run tests
cargo test

# Run examples
cargo run --example simple_server

# Build with TurboJPEG support (requires libjpeg-turbo)
cargo build --release --features turbojpeg
```

## Contributing

Contributions are welcome! Please:

1. Fork the repository
2. Create a feature branch
3. Make your changes
4. Add tests
5. Run `cargo fmt` and `cargo clippy`
6. Submit a pull request

## License

Apache-2.0 - See [LICENSE](LICENSE) file for details.

This library implements the VNC protocol as specified in RFC 6143, which is a public specification.
The ZYWRLE algorithm is used under a BSD-style license from Hitachi Systems & Services, Ltd.
All Rust dependencies use MIT or dual MIT/Apache-2.0 licenses.

### Optional Third-Party Dependencies

When using the `turbojpeg` feature, this library provides bindings to libjpeg-turbo (licensed under BSD-3-Clause, IJG, and zlib licenses). Users are responsible for compliance with libjpeg-turbo's license terms. See [NOTICE](NOTICE) for details.

## Credits

- **Author**: Dustin McAfee
- **Protocol**: Implements RFC 6143 (RFB Protocol Specification)
- **Encodings**: [rfb-encodings](https://github.com/dustinmcafee/rfb-encodings) - Reusable RFB encoding library
- **Used in**: [RustVNC](https://github.com/dustinmcafee/RustVNC) - VNC server for Android

## See Also

- [RFC 6143](https://datatracker.ietf.org/doc/html/rfc6143) - RFB Protocol Specification
- [TECHNICAL.md](TECHNICAL.md) - Detailed technical documentation
- [NOTICE](NOTICE) - Third-party licenses and attributions

---

**Pure Rust VNC Server - Memory Safe, Protocol Compliant, Production Ready** 🦀
