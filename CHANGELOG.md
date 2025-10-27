# Changelog

All notable changes to rustvncserver will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [2.0.0] - 2025-10-27

**Stable Release** - This marks the official 2.0.0 release, graduating from beta status.

### Changed

- **Code Deduplication**: Removed duplicate code to improve maintainability
  - Updated `rfb-encodings` dependency from `0.1.3` to `0.1.5`
  - Removed duplicate encoding type constants from `protocol.rs` (now imported from `rfb-encodings`)
  - Removed duplicate Hextile and Tight subencoding constants (now imported from `rfb-encodings`)
  - Deleted duplicate `src/jpeg/` module (now using TurboJPEG from `rfb-encodings`)
  - Encoding constants now have single source of truth in `rfb-encodings` library
  - Server-specific constants (COPYRECT, pseudo-encodings, protocol messages) remain in `protocol.rs`
  - Code reduction: ~220 lines of duplicate code eliminated

- **Documentation**: Comprehensive TurboJPEG setup and licensing information
  - Added TurboJPEG installation instructions for Ubuntu/Debian, macOS, and Windows in README
  - Added "TurboJPEG Setup" section with platform-specific installation commands
  - Updated License section to document optional third-party dependencies
  - Updated NOTICE file with complete libjpeg-turbo attribution including:
    - BSD-3-Clause license for TurboJPEG API
    - IJG License for libjpeg code
    - zlib License for SIMD extensions
    - Copyright notices for all contributors
  - Clarified that libjpeg-turbo is NOT distributed and users are responsible for license compliance

### Improved

- Simplified API surface by consolidating constant definitions
- Better separation of concerns: encoding library handles encoding constants, server handles protocol constants
- Reduced maintenance burden by eliminating duplicate code across projects

## [2.0.0-beta.4] - 2025-10-25

### Changed

- Updated all documentation (README.md, TECHNICAL.md, CONTRIBUTING.md) to properly credit the `rfb-encodings` library
  - Added clear references to [rfb-encodings](https://github.com/dustinmcafee/rfb-encodings) throughout documentation
  - Updated architecture diagrams to show the separation between rustvncserver and rfb-encodings
  - Clarified that rfb-encodings provides encoding implementations (for servers), not decoding (for clients)
  - Updated version examples in documentation to use version 2.0

## [2.0.0-beta.3] - 2025-10-23

### Changed

- Updated `rfb-encodings` dependency from `0.1` to `0.1.3`
  - Fixes critical build failure when using `turbojpeg` feature without `debug-logging`
  - Resolves "use of unresolved module or unlinked crate log" compilation errors
  - All turbojpeg builds now work correctly

## [2.0.0-beta.2] - 2025-10-23

### Fixed

- Code formatting: Removed extra blank line in `protocol.rs`

## [2.0.0-beta.1] - 2025-10-23

### Changed

- **Major architectural refactoring:** Extracted all encoding implementations to separate `rfb-encodings` crate
  - All encoding modules (Raw, RRE, CoRRE, Hextile, Zlib, Tight, TightPng, ZlibHex, ZRLE, ZYWRLE) moved to `rfb-encodings`
  - Pixel format translation moved to `rfb-encodings`
  - `PixelFormat` struct now re-exported from `rfb-encodings`
  - Benefits:
    - Encoding implementations now reusable across VNC servers, clients, proxies, and recorders
    - Cleaner separation of concerns: protocol vs encoding
    - Independent versioning and publishing of encodings
    - Better visibility and discoverability on crates.io
  - **Fully backwards compatible:** All public APIs preserved through re-exports
  - Existing code using `rustvncserver::encoding::*` or `rustvncserver::PixelFormat` continues to work

### Added

- New dependency: `rfb-encodings` crate (0.1.2) for all encoding implementations
- Re-exported all encoding types from `rfb-encodings` for full backwards compatibility
- `pub use rfb_encodings as encoding;` allows seamless access to all encodings

### Fixed

- All fixes from rfb-encodings 0.1.1 and 0.1.2 inherited:
  - macOS CI Build: Fixed turbojpeg linking errors
  - Compiler warnings for conditional compilation suppressed

## [1.1.5] - 2025-10-23

### Fixed
- **Critical RRE encoding bug:** Fixed data loss causing severe visual corruption and flickering
  - Root cause: Encoder had an "efficiency check" that would return 0 subrectangles when RRE encoding was larger than raw
  - This told VNC clients to paint the entire rectangle with only the background color, discarding all other pixel data
  - For video content or complex images with many colors, this resulted in constant flickering as frames alternated between partial data (background color only) and complete data
  - **Solution:** Always encode all pixels as subrectangles, even if RRE is inefficient
  - Ensures correct visual output at all times; protocol layer can choose different encoding if efficiency is a concern
  - Eliminates flickering and visual artifacts when using RRE encoding

## [1.1.3] - 2025-01-22

### Fixed
- **Critical TIGHT encoding bug:** Fixed persistent zlib stream implementation causing "Incomplete zlib block" errors
  - Root cause: `compress_data()` was creating fresh `ZlibEncoder` instances with `finish()` instead of using persistent streams
  - The control bytes indicated persistent stream IDs (0, 1, 2), but fresh streams created self-contained zlib blocks
  - This caused client decompressors to fail with "Incomplete zlib block" when expecting continuation data
  - **Solution:** Removed `stream.reset()` call that was destroying dictionary state
  - Now properly maintains dictionary state across compressions using `FlushCompress::Sync` (Z_SYNC_FLUSH)
  - Threaded persistent `TightStreamCompressor` through all encoding functions
  - Stream IDs correctly implemented per RFC 6143: 0=full-color, 1=mono, 2=indexed
  - Eliminates decompression errors and connection instability with noVNC and other clients

## [1.1.2] - 2025-01-21

### Fixed
- **Critical encoding bug:** Fixed server incorrectly selecting COPYRECT as preferred encoding
  - Server was blindly using the first encoding from client's list, which could be COPYRECT
  - COPYRECT is only for copy rectangle operations, not general framebuffer encoding
  - Now properly skips COPYRECT and selects the first supported encoding (TIGHT, ZRLE, etc.)
  - Eliminates "Encoding 1 not available, falling back to RAW" error and poor performance

## [1.1.1] - 2025-01-21

### Added
- **Security feature:** `debug-logging` feature flag to control verbose logging
  - Hides sensitive information (client IPs, connection details, protocol versions) by default
  - Enable with `features = ["debug-logging"]` for troubleshooting
  - Operational logs (server startup, client connect/disconnect) remain visible

### Fixed
- Eliminated all unused variable and assignment warnings when `debug-logging` is disabled
- Fixed clippy `uninlined_format_args` warning in tight.rs
- Code formatting consistency with `cargo fmt`

### Changed
- Updated minimum Rust version requirement from 1.76 to 1.90 in CONTRIBUTING.md

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
