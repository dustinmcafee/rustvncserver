//! VNC Hextile encoding implementation.
//!
//! Hextile divides the rectangle into 16x16 tiles and encodes each independently.
//! Each tile can be: raw, solid, monochrome with subrects, or colored with subrects.

use bytes::{BufMut, BytesMut};
use super::Encoding;
use super::common::{rgba_to_rgb24_pixels, extract_tile, analyze_tile_colors, find_subrects, put_pixel32};
use crate::vnc::protocol::{HEXTILE_RAW, HEXTILE_BACKGROUND_SPECIFIED, HEXTILE_FOREGROUND_SPECIFIED, HEXTILE_ANY_SUBRECTS, HEXTILE_SUBRECTS_COLOURED};

/// Implements the VNC "Hextile" encoding.
///
/// Hextile divides the rectangle into 16x16 tiles and encodes each independently.
/// Each tile can be: raw, solid, monochrome with subrects, or colored with subrects.
pub struct HextileEncoding;

impl Encoding for HextileEncoding {
    fn encode(&self, data: &[u8], width: u16, height: u16, _quality: u8, _compression: u8) -> BytesMut {
        let mut buf = BytesMut::new();
        let pixels = rgba_to_rgb24_pixels(data);

        let mut last_bg: Option<u32> = None;
        let mut last_fg: Option<u32> = None;

        // Process tiles (16x16)
        for tile_y in (0..height).step_by(16) {
            for tile_x in (0..width).step_by(16) {
                let tile_w = std::cmp::min(16, width - tile_x);
                let tile_h = std::cmp::min(16, height - tile_y);

                // Extract tile data
                let tile_pixels = extract_tile(&pixels, width as usize, tile_x as usize, tile_y as usize, tile_w as usize, tile_h as usize);

                // Analyze tile colors
                let (is_solid, is_mono, bg, fg) = analyze_tile_colors(&tile_pixels);

                let mut subencoding: u8 = 0;
                let tile_start = buf.len();

                // Reserve space for subencoding byte
                buf.put_u8(0);

                if is_solid {
                    // Solid tile - just update background if needed
                    if Some(bg) != last_bg {
                        subencoding |= HEXTILE_BACKGROUND_SPECIFIED;
                        put_pixel32(&mut buf, bg);
                        last_bg = Some(bg);
                    }
                } else {
                    // Find subrectangles
                    let subrects = find_subrects(&tile_pixels, tile_w as usize, tile_h as usize, bg);

                    // Check if raw would be smaller OR if too many subrects (>255 max for u8)
                    let raw_size = tile_w as usize * tile_h as usize * 4; // 4 bytes per pixel for 32bpp
                    // Estimate overhead: bg (if different) + fg (if mono and different) + count byte
                    let bg_overhead = if Some(bg) != last_bg { 4 } else { 0 };
                    let fg_overhead = if is_mono && Some(fg) != last_fg { 4 } else { 0 };
                    let subrect_data = subrects.len() * if is_mono { 2 } else { 6 };
                    let encoded_size = bg_overhead + fg_overhead + 1 + subrect_data;

                    if subrects.is_empty() || subrects.len() > 255 || encoded_size > raw_size {
                        // Use raw encoding for this tile
                        subencoding = HEXTILE_RAW;
                        buf.truncate(tile_start);
                        buf.put_u8(subencoding);

                        for pixel in &tile_pixels {
                            put_pixel32(&mut buf, *pixel);
                        }

                        last_bg = None;
                        last_fg = None;
                        continue;
                    }

                    // Update background
                    if Some(bg) != last_bg {
                        subencoding |= HEXTILE_BACKGROUND_SPECIFIED;
                        put_pixel32(&mut buf, bg);
                        last_bg = Some(bg);
                    }

                    // We have subrectangles
                    subencoding |= HEXTILE_ANY_SUBRECTS;

                    if is_mono {
                        // Monochrome tile
                        if Some(fg) != last_fg {
                            subencoding |= HEXTILE_FOREGROUND_SPECIFIED;
                            put_pixel32(&mut buf, fg);
                            last_fg = Some(fg);
                        }

                        // Write number of subrects
                        buf.put_u8(subrects.len() as u8);

                        // Write subrects (without color)
                        for sr in subrects {
                            buf.put_u8(((sr.x as u8) << 4) | (sr.y as u8));
                            buf.put_u8((((sr.w - 1) as u8) << 4) | ((sr.h - 1) as u8));
                        }
                    } else {
                        // Colored subrects
                        subencoding |= HEXTILE_SUBRECTS_COLOURED;
                        last_fg = None;

                        buf.put_u8(subrects.len() as u8);

                        for sr in subrects {
                            put_pixel32(&mut buf, sr.color); // 4 bytes for 32bpp
                            buf.put_u8(((sr.x as u8) << 4) | (sr.y as u8));  // packed X,Y
                            buf.put_u8((((sr.w - 1) as u8) << 4) | ((sr.h - 1) as u8));  // packed W-1,H-1
                        }
                    }
                }

                // Write subencoding byte
                buf[tile_start] = subencoding;
            }
        }

        buf
    }
}
