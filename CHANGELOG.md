# Changelog

All notable changes to rustvncserver will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [1.1.0] - 2025-01-20

### Added

**Project Infrastructure:**
- CI/CD pipeline with GitHub Actions for automated testing
  - Multi-platform testing (Ubuntu, Windows, macOS)
  - Rust stable and beta channel support
  - Clippy linting, rustfmt checks, and documentation validation
- CONTRIBUTING.md with comprehensive contribution guidelines
- CODE_OF_CONDUCT.md (Contributor Covenant v2.1)
- SECURITY.md with vulnerability reporting process and security best practices

**Documentation:**
- Professional README badges for Crates.io, docs.rs, Build Status, and Downloads
- LinkedIn profile badge
- Multiple donation options with badges:
  - GitHub Sponsors
  - PayPal
  - Buy Me A Coffee
  - Cryptocurrency support (Bitcoin, Ethereum, Solana, Monero)
- docs.rs metadata configuration for multi-platform documentation
- Comprehensive doc comments for all TurboJPEG pixel format and subsampling constants

### Changed
- Upgraded documentation requirement from `warn(missing_docs)` to `deny(missing_docs)` for stricter enforcement
- Added `clippy::pedantic` lint warnings for higher code quality standards
- Improved rustdoc output quality with proper markdown formatting

### Fixed
- Added missing `#[link(name = "turbojpeg")]` attribute for proper linking when turbojpeg feature is enabled

## [1.0.2] - 2025-01-20

### Fixed
- Updated README.md and TECHNICAL.md with correct version numbers (1.0 instead of 0.1)
- Updated integration examples to use crates.io version instead of path-based dependency

## [1.0.1] - 2025-01-20 (unreleased)

### Fixed
- Updated README.md with correct RustVNC GitHub repository URL

## [1.0.0] - 2025-01-20

### Added

**Protocol Implementation:**
- Complete RFC 6143 (RFB 3.8) protocol compliance
- VNC authentication support (DES encryption)
- Reverse connection support (connect to listening viewers)
- UltraVNC repeater support (Mode-2)
- Bidirectional clipboard support via events

**Encoding Support (11 total):**
- Raw encoding (0) - Uncompressed pixels
- CopyRect encoding (1) - Efficient region copying for scrolling/dragging
- RRE encoding (2) - Rise-and-Run-length encoding
- CoRRE encoding (4) - Compact RRE with 8-bit coordinates
- Hextile encoding (5) - 16x16 tile-based encoding
- Zlib encoding (6) - Zlib-compressed raw pixels with persistent streams
- Tight encoding (7) - Multi-mode compression with all 5 modes:
  - Solid fill (1 color)
  - Mono rect (2 colors, 1-bit bitmap)
  - Indexed palette (3-16 colors)
  - Full-color zlib (lossless)
  - JPEG (lossy, hardware-accelerated)
- ZlibHex encoding (8) - Zlib-compressed Hextile with persistent streams
- ZRLE encoding (16) - Zlib Run-Length with persistent streams
- ZYWRLE encoding (17) - Wavelet-based lossy compression with persistent streams
- TightPng encoding (-260) - PNG-only compression mode

**Pixel Format Support:**
- Full pixel format translation for all color depths
- 8-bit color (RGB332, BGR233, indexed)
- 16-bit color (RGB565, RGB555, BGR565, BGR555)
- 24-bit color (RGB888, BGR888)
- 32-bit color (RGBA32, BGRA32, RGBX, BGRX)
- Big-endian and little-endian support

**Compression Features:**
- Persistent zlib compression streams for optimal performance
- 4 persistent streams for Tight encoding (per RFC 6143)
- Quality level pseudo-encodings (-32 to -23, levels 0-9)
- Compression level pseudo-encodings (-256 to -247, levels 0-9)
- JPEG quality mapping compatible with TigerVNC

**Performance Features:**
- Async/await architecture using Tokio runtime
- Zero-copy framebuffer updates via Arc-based sharing
- Concurrent multi-client support
- Efficient dirty region tracking
- CopyRect scheduling for scrolling/dragging operations

**Architecture:**
- Memory-safe Rust implementation
- No buffer overflows, use-after-free, or data races
- Thread-safe concurrent client handling
- Event-based architecture for client input (keyboard, pointer, clipboard)

**Documentation:**
- Comprehensive README with feature overview
- Complete technical documentation (TECHNICAL.md)
- Example implementations (simple_server, headless_server)

### Features

**Compatibility:**
- Works with all standard VNC viewers (TigerVNC, RealVNC, TightVNC)
- Works with web-based clients (noVNC)
- 100% wire format compatible with RFC 6143

**Optional Features:**
- `turbojpeg` - Hardware-accelerated JPEG compression via libjpeg-turbo (NEON on ARM, SSE2 on x86)

### Notes

**Tested Encodings:**
- Raw, CopyRect, RRE, Hextile, Zlib, Tight, ZRLE, TightPng - Fully tested with noVNC

**Untested Encodings:**
- CoRRE, ZlibHex, ZYWRLE - Fully implemented and RFC 6143 compliant but cannot be tested with noVNC due to lack of client support

**Not Implemented (Low Priority):**
- Cursor pseudo-encoding (-239)
- Desktop resize pseudo-encoding (-223)

---

## Release Information

**Initial Release:** v1.0.0 marks the first stable release of rustvncserver with complete RFC 6143 protocol compliance and all major VNC encodings operational.

**License:** Apache License 2.0

**Repository:** https://github.com/dustinmcafee/rustvncserver
