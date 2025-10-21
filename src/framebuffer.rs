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

//! VNC framebuffer management and dirty region tracking.
//!
//! This module provides the core framebuffer functionality for the VNC server, including:
//! - Pixel data storage and access
//! - Dirty region tracking for efficient updates
//! - Copy rectangle detection for bandwidth optimization
//! - Client notification system for framebuffer changes
//!
//! # Architecture
//!
//! The framebuffer uses a push-based update model similar to standard VNC protocol, where changes
//! are automatically propagated to all connected clients through `DirtyRegionReceiver` handles.
//! This eliminates the need for clients to poll for changes.
//!
//! # Dirty Region Management
//!
//! When the framebuffer is updated, the system:
//! 1. Identifies the minimal bounding box of changed pixels
//! 2. Creates a `DirtyRegion` representing this change
//! 3. Pushes this region to all registered client receivers
//! 4. Clients merge and batch these regions for efficient transmission

use std::sync::Arc;
use std::sync::Weak;
use tokio::sync::RwLock;

/// Represents a rectangular region of the framebuffer that has been modified.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DirtyRegion {
    /// The X coordinate of the top-left corner of the region.
    pub x: u16,
    /// The Y coordinate of the top-left corner of the region.
    pub y: u16,
    /// The width of the region.
    pub width: u16,
    /// The height of the region.
    pub height: u16,
}

impl DirtyRegion {
    /// Creates a new `DirtyRegion`.
    ///
    /// # Arguments
    ///
    /// * `x` - The X coordinate of the top-left corner.
    /// * `y` - The Y coordinate of the top-left corner.
    /// * `width` - The width of the region.
    /// * `height` - The height of the region.
    ///
    /// # Returns
    ///
    /// A new `DirtyRegion` instance.
    #[must_use]
    pub fn new(x: u16, y: u16, width: u16, height: u16) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// Merges this `DirtyRegion` with another, returning a new `DirtyRegion`
    /// that contains both.
    ///
    /// # Arguments
    ///
    /// * `other` - The other `DirtyRegion` to merge with.
    ///
    /// # Returns
    ///
    /// A new `DirtyRegion` representing the union of the two regions.
    #[must_use]
    pub fn merge(&self, other: &DirtyRegion) -> DirtyRegion {
        let x1 = self.x.min(other.x);
        let y1 = self.y.min(other.y);
        // Use saturating_add to prevent overflow
        let x2 = self
            .x
            .saturating_add(self.width)
            .max(other.x.saturating_add(other.width));
        let y2 = self
            .y
            .saturating_add(self.height)
            .max(other.y.saturating_add(other.height));

        DirtyRegion {
            x: x1,
            y: y1,
            width: x2.saturating_sub(x1),
            height: y2.saturating_sub(y1),
        }
    }

    /// Checks if this `DirtyRegion` intersects with another.
    ///
    /// # Arguments
    ///
    /// * `other` - The other `DirtyRegion` to check for intersection.
    ///
    /// # Returns
    ///
    /// `true` if the regions intersect, `false` otherwise.
    #[must_use]
    pub fn intersects(&self, other: &DirtyRegion) -> bool {
        let x1 = self.x.max(other.x);
        let y1 = self.y.max(other.y);
        // Use saturating_add to prevent overflow
        let x2 = self
            .x
            .saturating_add(self.width)
            .min(other.x.saturating_add(other.width));
        let y2 = self
            .y
            .saturating_add(self.height)
            .min(other.y.saturating_add(other.height));

        x1 < x2 && y1 < y2
    }

    /// Computes the intersection of two `DirtyRegion`s.
    ///
    /// This function returns a new `DirtyRegion` representing the overlapping area
    /// of the two regions. If the regions do not intersect, `None` is returned.
    /// This behavior matches standard VNC protocol's `sraRgnAnd` operation.
    ///
    /// # Arguments
    ///
    /// * `other` - The other `DirtyRegion` to compute the intersection with.
    ///
    /// # Returns
    ///
    /// An `Option<DirtyRegion>` which is `Some(DirtyRegion)` if they intersect,
    /// or `None` if they do not.
    #[must_use]
    pub fn intersect(&self, other: &DirtyRegion) -> Option<DirtyRegion> {
        let x1 = self.x.max(other.x);
        let y1 = self.y.max(other.y);
        // Use saturating_add to prevent overflow
        let x2 = self
            .x
            .saturating_add(self.width)
            .min(other.x.saturating_add(other.width));
        let y2 = self
            .y
            .saturating_add(self.height)
            .min(other.y.saturating_add(other.height));

        if x1 < x2 && y1 < y2 {
            Some(DirtyRegion {
                x: x1,
                y: y1,
                width: x2.saturating_sub(x1),
                height: y2.saturating_sub(y1),
            })
        } else {
            None
        }
    }
}

/// A struct for receiving notifications about dirty (modified) regions in the framebuffer.
///
/// This uses a `Weak` reference to the client's `modified_regions` to allow for a
/// push-based update model, similar to how standard VNC protocol handles dirty region updates.
#[derive(Clone)]
pub struct DirtyRegionReceiver {
    /// A `Weak` reference to a `RwLock`-protected vector of `DirtyRegion`s.
    regions: Weak<RwLock<Vec<DirtyRegion>>>,
}

impl DirtyRegionReceiver {
    /// Creates a new `DirtyRegionReceiver`.
    ///
    /// # Arguments
    ///
    /// * `regions` - A `Weak` reference to the vector of `DirtyRegion`s to be updated.
    ///
    /// # Returns
    ///
    /// A new `DirtyRegionReceiver` instance.
    #[must_use]
    pub fn new(regions: Weak<RwLock<Vec<DirtyRegion>>>) -> Self {
        Self { regions }
    }

    /// Adds a new dirty region to the receiver's list.
    ///
    /// This function handles merging the new region with any existing intersecting regions
    /// to maintain a minimal set of non-overlapping dirty regions. It also includes logic
    /// to prevent memory exhaustion by merging all regions if the list grows too large.
    /// This behavior is modeled after standard VNC protocol's region merging.
    ///
    /// # Arguments
    ///
    /// * `region` - The `DirtyRegion` to add.
    pub async fn add_dirty_region(&self, region: DirtyRegion) {
        // Limit number of regions and total pixel count to prevent memory exhaustion
        // These limits ensure bounded memory usage even with rapid screen changes
        const MAX_REGIONS: usize = 10;
        const MAX_TOTAL_PIXELS: usize = 1920 * 1080 * 2; // Approximately 2 Full HD screens

        if let Some(regions_arc) = self.regions.upgrade() {
            let mut regions = regions_arc.write().await;

            // Merge with ALL intersecting regions (not just first)
            // This matches standard VNC protocol's proper region merging behavior
            let mut merged_region = region;
            regions.retain(|existing| {
                if existing.intersects(&merged_region) {
                    merged_region = existing.merge(&merged_region);
                    false // Remove this region, we've merged it
                } else {
                    true // Keep this region
                }
            });

            // Add the final merged region
            regions.push(merged_region);

            let total_pixels: usize = regions
                .iter()
                .map(|r| (r.width as usize) * (r.height as usize))
                .sum();

            if regions.len() > MAX_REGIONS || total_pixels > MAX_TOTAL_PIXELS {
                // If limits exceeded, merge all regions into one to prevent unbounded growth
                // This trades granularity for memory safety
                if let Some(first) = regions.first().copied() {
                    let merged = regions.iter().skip(1).fold(first, |acc, r| acc.merge(r));
                    regions.clear();
                    regions.push(merged);
                }
            }
        }
    }
}

use std::sync::atomic::{AtomicU16, Ordering as AtomicOrdering};

/// Represents the VNC server's framebuffer.
///
/// This struct manages the pixel data of the remote screen, tracks dirty regions,
/// and notifies connected clients of updates. It also maintains a copy of the
/// previous framebuffer state to enable `CopyRect` encoding for performance optimization.
#[derive(Clone)]
pub struct Framebuffer {
    /// The width of the framebuffer in pixels (uses atomic for interior mutability).
    width: Arc<AtomicU16>,
    /// The height of the framebuffer in pixels (uses atomic for interior mutability).
    height: Arc<AtomicU16>,
    /// The raw pixel data of the framebuffer, stored as an `Arc<RwLock<Vec<u8>>>` for shared, mutable access.
    data: Arc<RwLock<Vec<u8>>>,
    /// A list of `DirtyRegionReceiver`s to be notified when parts of the framebuffer are modified.
    receivers: Arc<RwLock<Vec<DirtyRegionReceiver>>>,
    /// A copy of the previous framebuffer data, used for detecting `CopyRect` encoding opportunities.
    prev_data: Arc<RwLock<Vec<u8>>>,
}

impl Framebuffer {
    /// Creates a new `Framebuffer`.
    ///
    /// # Arguments
    ///
    /// * `width` - The width of the framebuffer in pixels.
    /// * `height` - The height of the framebuffer in pixels.
    ///
    /// # Returns
    ///
    /// A new `Framebuffer` instance.
    #[must_use]
    pub fn new(width: u16, height: u16) -> Self {
        let size = (width as usize) * (height as usize) * 4; // RGBA32
        Self {
            width: Arc::new(AtomicU16::new(width)),
            height: Arc::new(AtomicU16::new(height)),
            data: Arc::new(RwLock::new(vec![0; size])),
            receivers: Arc::new(RwLock::new(Vec::new())),
            prev_data: Arc::new(RwLock::new(vec![0; size])),
        }
    }

    /// Registers a `DirtyRegionReceiver` to be notified of framebuffer updates.
    ///
    /// This method allows clients to subscribe to dirty region notifications, similar to
    /// standard VNC protocol's `rfbGetClientIterator` pattern.
    ///
    /// # Arguments
    ///
    /// * `receiver` - The `DirtyRegionReceiver` to register.
    pub async fn register_receiver(&self, receiver: DirtyRegionReceiver) {
        let mut receivers = self.receivers.write().await;
        receivers.push(receiver);
    }

    /// Removes dead `Weak` references from the list of `DirtyRegionReceiver`s.
    ///
    /// This function is called periodically to clean up receivers for clients that have disconnected.
    async fn cleanup_receivers(&self) {
        let mut receivers = self.receivers.write().await;
        receivers.retain(|r| r.regions.strong_count() > 0);
    }

    /// Marks a rectangular region of the framebuffer as dirty and notifies all registered receivers.
    ///
    /// This behavior is analogous to standard VNC protocol's `rfbMarkRegionAsModified` function.
    ///
    /// # Arguments
    ///
    /// * `x` - The X coordinate of the top-left corner of the dirty region.
    /// * `y` - The Y coordinate of the top-left corner of the dirty region.
    /// * `width` - The width of the dirty region.
    /// * `height` - The height of the dirty region.
    pub async fn mark_dirty_region(&self, x: u16, y: u16, width: u16, height: u16) {
        let region = DirtyRegion::new(x, y, width, height);

        // Clone receivers while holding lock briefly to prevent deadlock
        // (standard VNC protocol uses client iterator for similar thread safety)
        let receivers_copy = {
            let receivers = self.receivers.read().await;
            receivers.clone()
        };

        // Notify all receivers without holding receivers lock
        // This prevents deadlock if add_dirty_region acquires other locks
        for receiver in &receivers_copy {
            receiver.add_dirty_region(region).await;
        }

        // Clean up dead receivers
        self.cleanup_receivers().await;
    }

    /// Returns the width of the framebuffer.
    #[must_use]
    pub fn width(&self) -> u16 {
        self.width.load(AtomicOrdering::Relaxed)
    }

    /// Returns the height of the framebuffer.
    #[must_use]
    pub fn height(&self) -> u16 {
        self.height.load(AtomicOrdering::Relaxed)
    }

    /// Updates the entire framebuffer from a slice of data.
    ///
    /// This function compares the new data with the existing framebuffer content and
    /// identifies the bounding box of the changes, marking only the modified region as dirty.
    /// This approach implements efficient differential updates as specified in RFC 6143.
    ///
    /// # Arguments
    ///
    /// * `data` - A slice containing the new RGBA32 pixel data for the entire framebuffer.
    ///
    /// # Returns
    ///
    /// `Ok(())` if the update is successful.
    ///
    /// # Errors
    ///
    /// Returns `Err(String)` if the provided data slice has an incorrect size.
    ///
    /// # Panics
    ///
    /// May panic if the framebuffer dimensions are invalid or if internal state is corrupted.
    #[allow(clippy::cast_possible_truncation)] // Intentional: converting row indices to u16 coordinates
    pub async fn update_from_slice(&self, data: &[u8]) -> Result<(), String> {
        let expected_size = (self.width() as usize) * (self.height() as usize) * 4;
        if data.len() != expected_size {
            return Err(format!(
                "Invalid data size: expected {}, got {}",
                expected_size,
                data.len()
            ));
        }

        let mut fb = self.data.write().await;

        let width_usize = self.width() as usize;
        let row_bytes = width_usize * 4;
        let mut changed = false;
        let mut min_y = 0;
        let mut max_y = 0;
        let mut min_x = u16::MAX;
        let mut max_x = 0;

        // Find the first and last changed rows efficiently by comparing whole rows
        if let Some(first_row) = (0..self.height() as usize).find(|&y| {
            let offset = y * row_bytes;
            fb[offset..offset + row_bytes] != data[offset..offset + row_bytes]
        }) {
            changed = true;
            min_y = first_row as u16;

            // Since we know there's a change, the last changed row must exist
            max_y = (min_y as usize..self.height() as usize)
                .rev()
                .find(|&y| {
                    let offset = y * row_bytes;
                    fb[offset..offset + row_bytes] != data[offset..offset + row_bytes]
                })
                .unwrap() as u16;

            // Now find the exact horizontal bounds but only within the changed rows
            for y in min_y..=max_y {
                for x in 0..self.width() {
                    let offset = ((y as usize) * width_usize + (x as usize)) * 4;
                    if fb[offset..offset + 4] != data[offset..offset + 4] {
                        min_x = min_x.min(x);
                        max_x = max_x.max(x);
                    }
                }
            }
        }

        if changed {
            fb.copy_from_slice(data);
            drop(fb); // Release lock before other operations

            // Save state for CopyRect detection
            self.save_state().await;

            // Mark only the changed rectangle as dirty
            // Calculate proper width and height (inclusive to exclusive conversion)
            let width = max_x - min_x + 1;
            let height = max_y - min_y + 1;
            self.mark_dirty_region(min_x, min_y, width, height).await;
        }

        Ok(())
    }

    /// Retrieves the pixel data for a specific rectangular region of the framebuffer.
    ///
    /// # Arguments
    ///
    /// * `x` - The X coordinate of the top-left corner of the region.
    /// * `y` - The Y coordinate of the top-left corner of the region.
    /// * `width` - The width of the region to retrieve.
    /// * `height` - The height of the region to retrieve.
    ///
    /// # Returns
    ///
    /// `Ok(Vec<u8>)` containing the pixel data for the requested rectangle.
    ///
    /// # Errors
    ///
    /// Returns `Err(String)` if the requested rectangle is out of the framebuffer's bounds.
    pub async fn get_rect(
        &self,
        x: u16,
        y: u16,
        width: u16,
        height: u16,
    ) -> Result<Vec<u8>, String> {
        // Bounds checking with overflow protection - return error instead of panic
        if x.saturating_add(width) > self.width() || y.saturating_add(height) > self.height() {
            return Err(format!(
                "Rectangle out of bounds: ({}, {}, {}, {}) exceeds ({}, {})",
                x,
                y,
                width,
                height,
                self.width(),
                self.height()
            ));
        }

        let data = self.data.read().await;
        let mut result = Vec::with_capacity((width as usize) * (height as usize) * 4);

        for row in y..(y + height) {
            let start = ((row as usize) * (self.width() as usize) + (x as usize)) * 4;
            let end = start + (width as usize) * 4;
            result.extend_from_slice(&data[start..end]);
        }

        Ok(result)
    }

    /// Returns a copy of the entire framebuffer's pixel data.
    ///
    /// # Returns
    ///
    /// A `Vec<u8>` containing the full framebuffer data.
    #[allow(dead_code)]
    pub async fn get_full_data(&self) -> Vec<u8> {
        self.data.read().await.clone()
    }

    /// Updates a specified cropped region of the framebuffer with new data.
    ///
    /// This function performs validation to ensure the crop region is within the framebuffer bounds
    /// and that the provided data size matches the crop dimensions. It then identifies the precise
    /// dirty region within the crop area by comparing pixel data, and marks this smaller region
    /// as dirty.
    ///
    /// # Arguments
    ///
    /// * `data` - A slice containing the new RGBA32 pixel data for the cropped region.
    /// * `crop_x` - The X coordinate of the top-left corner of the crop region.
    /// * `crop_y` - The Y coordinate of the top-left corner of the crop region.
    /// * `crop_width` - The width of the crop region.
    /// * `crop_height` - The height of the crop region.
    ///
    /// # Returns
    ///
    /// `Ok(())` if the update is successful.
    ///
    /// # Errors
    ///
    /// Returns `Err(String)` if the crop region is out of bounds or the data size is incorrect.
    pub async fn update_cropped(
        &self,
        data: &[u8],
        crop_x: u16,
        crop_y: u16,
        crop_width: u16,
        crop_height: u16,
    ) -> Result<(), String> {
        // Validate crop region with overflow protection
        if crop_x.saturating_add(crop_width) > self.width() {
            return Err(format!(
                "Crop region exceeds framebuffer width: {}+{} > {}",
                crop_x,
                crop_width,
                self.width()
            ));
        }
        if crop_y.saturating_add(crop_height) > self.height() {
            return Err(format!(
                "Crop region exceeds framebuffer height: {}+{} > {}",
                crop_y,
                crop_height,
                self.height()
            ));
        }

        let expected_size = (crop_width as usize) * (crop_height as usize) * 4;
        if data.len() != expected_size {
            return Err(format!(
                "Invalid crop data size: expected {}, got {}",
                expected_size,
                data.len()
            ));
        }

        let mut fb = self.data.write().await;

        let mut changed = false;
        let mut min_x = u16::MAX;
        let mut min_y = u16::MAX;
        let mut max_x = 0u16;
        let mut max_y = 0u16;
        let crop_width_usize = crop_width as usize;
        let frame_width_usize = self.width() as usize;

        for y in 0..crop_height {
            let src_offset = (y as usize) * crop_width_usize * 4;
            let dst_offset = ((crop_y + y) as usize * frame_width_usize + crop_x as usize) * 4;
            let src_row = &data[src_offset..src_offset + crop_width_usize * 4];
            let dst_row = &fb[dst_offset..dst_offset + crop_width_usize * 4];

            if src_row != dst_row {
                let abs_y = crop_y + y;
                min_y = min_y.min(abs_y);
                max_y = max_y.max(abs_y);

                for x in 0..crop_width {
                    let px_offset = x as usize * 4;
                    if src_row[px_offset..px_offset + 4] != dst_row[px_offset..px_offset + 4] {
                        let abs_x = crop_x + x;
                        min_x = min_x.min(abs_x);
                        max_x = max_x.max(abs_x);
                    }
                }

                // Update the framebuffer row
                fb[dst_offset..dst_offset + crop_width_usize * 4].copy_from_slice(src_row);
                changed = true;
            }
        }

        if changed {
            let width = (max_x - min_x + 1).min(self.width() - min_x);
            let height = (max_y - min_y + 1).min(self.height() - min_y);
            drop(fb); // Release lock before marking dirty

            // Save state for CopyRect detection
            self.save_state().await;

            self.mark_dirty_region(min_x, min_y, width, height).await;
        }

        Ok(())
    }

    /// Detects copy operations by comparing current framebuffer with previous state.
    ///
    /// This method identifies if a dirty region's content matches a region from the previous
    /// framebuffer state at a different location. This enables the use of `CopyRect` encoding,
    /// which dramatically reduces bandwidth for scrolling and window dragging operations.
    ///
    /// NOTE: This auto-detection method is no longer used. `CopyRect` now uses explicit
    /// tracking via `schedule_copy_region()` and `do_copy_region()`, matching standard VNC protocol's approach.
    ///
    /// # Arguments
    ///
    /// * `region` - The dirty region to check for copy detection.
    ///
    /// # Returns
    ///
    /// `Some((src_x, src_y))` if the region matches content at a different location in the
    /// previous framebuffer, where `(src_x, src_y)` are the source coordinates.
    /// Returns `None` if no match is found or the region is too small for copy detection.
    #[allow(dead_code)]
    #[allow(clippy::cast_possible_truncation)] // Intentional: i32 to u16 coordinate conversion with bounds checks
    #[allow(clippy::cast_sign_loss)] // Intentional: i32 to usize for array indexing after bounds checks
    pub async fn detect_copy_rect(&self, region: &DirtyRegion) -> Option<(u16, u16)> {
        // Don't detect copy for very small regions (not worth the CPU cost)
        const MIN_COPY_SIZE: u16 = 64;
        if region.width < MIN_COPY_SIZE || region.height < MIN_COPY_SIZE {
            return None;
        }

        // Extract only the region data we need, then release locks early
        // This prevents holding locks during the expensive comparison operation
        let (current_region_data, prev_full_data) = {
            let data = self.data.read().await;
            let prev = self.prev_data.read().await;

            // Quick check: if previous framebuffer is empty, can't detect copies
            if prev.len() != data.len() {
                return None;
            }

            // Extract current region data (what we're comparing)
            let mut current = Vec::new();
            for row in region.y..(region.y + region.height) {
                let start = ((row as usize) * (self.width() as usize) + (region.x as usize)) * 4;
                let end = start + (region.width as usize) * 4;
                current.extend_from_slice(&data[start..end]);
            }

            (current, prev.clone())
        };
        // Locks released here

        // Search for matching region in previous framebuffer
        // We'll check common scroll offsets (vertical and horizontal)
        let search_offsets: Vec<(i32, i32)> = vec![
            // Vertical scrolling (common in text editors, browsers)
            (0, -1),
            (0, -2),
            (0, -3),
            (0, -5),
            (0, -10),
            (0, -20),
            (0, 1),
            (0, 2),
            (0, 3),
            (0, 5),
            (0, 10),
            (0, 20),
            // Horizontal scrolling
            (-1, 0),
            (-2, 0),
            (-3, 0),
            (-5, 0),
            (-10, 0),
            (-20, 0),
            (1, 0),
            (2, 0),
            (3, 0),
            (5, 0),
            (10, 0),
            (20, 0),
            // Diagonal (window dragging)
            (-1, -1),
            (1, 1),
            (-1, 1),
            (1, -1),
        ];

        for (dx, dy) in search_offsets {
            let src_x = i32::from(region.x) + dx;
            let src_y = i32::from(region.y) + dy;

            // Check if source is within bounds
            if src_x < 0
                || src_y < 0
                || (src_x as u16).saturating_add(region.width) > self.width()
                || (src_y as u16).saturating_add(region.height) > self.height()
            {
                continue;
            }

            // Compare current region with previous framebuffer at offset
            let mut matches = true;
            let sample_rows = (region.height / 4).max(1); // Sample 25% of rows for performance
            let step_size =
                ((region.height as usize).max(1) / (sample_rows as usize).max(1)).max(1);

            for row_idx in (0..region.height).step_by(step_size) {
                let src_row = (src_y as u16) + row_idx;

                // Calculate offset in current_region_data
                let current_row_start = (row_idx as usize) * (region.width as usize) * 4;
                let current_row_end = current_row_start + (region.width as usize) * 4;

                // Calculate offset in prev_full_data
                let prev_row_start =
                    ((src_row as usize) * (self.width() as usize) + (src_x as usize)) * 4;
                let prev_row_end = prev_row_start + (region.width as usize) * 4;

                // Bounds check for prev_full_data
                if prev_row_end > prev_full_data.len() {
                    matches = false;
                    break;
                }

                // Compare current region data with previous framebuffer at offset
                if current_region_data[current_row_start..current_row_end]
                    != prev_full_data[prev_row_start..prev_row_end]
                {
                    matches = false;
                    break;
                }
            }

            if matches {
                // Found a match! Return source coordinates
                return Some((src_x as u16, src_y as u16));
            }
        }

        None
    }

    /// Saves the current framebuffer state for future copy detection.
    ///
    /// This method creates a snapshot of the current framebuffer, which is used by
    /// `detect_copy_rect` to identify scrolling and copying operations. Should be called
    /// after each framebuffer update to maintain accurate copy detection.
    pub async fn save_state(&self) {
        let data = self.data.read().await;
        let mut prev = self.prev_data.write().await;
        prev.copy_from_slice(&data);
    }

    /// Resizes the framebuffer to new dimensions.
    ///
    /// This method creates a new framebuffer with the specified dimensions, preserving
    /// as much of the existing content as possible. Any content that fits within the new
    /// dimensions is copied over; areas outside are clipped, and new areas are filled with black.
    ///
    /// This is equivalent to standard VNC protocol's `rfbNewFramebuffer` function.
    ///
    /// Note: This method uses interior mutability through `RwLock`, so it doesn't require `&mut self`.
    /// The width and height fields themselves cannot be updated atomically with the data,
    /// so there's a brief window where they may be inconsistent. However, since this is called
    /// infrequently (only on screen rotation/resize), this is acceptable.
    ///
    /// # Arguments
    ///
    /// * `new_width` - The new width of the framebuffer in pixels.
    /// * `new_height` - The new height of the framebuffer in pixels.
    ///
    /// # Returns
    ///
    /// `Ok(())` if the resize is successful.
    ///
    /// # Errors
    ///
    /// Returns `Err(String)` if the new dimensions are invalid (zero or too large).
    pub async fn resize(&self, new_width: u16, new_height: u16) -> Result<(), String> {
        const MAX_DIMENSION: u16 = 8192;

        // Validate dimensions
        if new_width == 0 || new_height == 0 {
            return Err("Framebuffer dimensions must be greater than zero".to_string());
        }

        if new_width > MAX_DIMENSION || new_height > MAX_DIMENSION {
            return Err(format!(
                "Framebuffer dimensions too large: {new_width}x{new_height} (max: {MAX_DIMENSION})"
            ));
        }

        // Calculate new size
        let new_size = (new_width as usize) * (new_height as usize) * 4;

        let old_width = self.width();
        let old_height = self.height();

        // If dimensions haven't changed, do nothing
        if new_width == old_width && new_height == old_height {
            return Ok(());
        }

        // Create new framebuffer data
        let mut new_data = vec![0u8; new_size];

        // Copy as much of the old data as possible
        {
            let old_data = self.data.read().await;
            let copy_width = old_width.min(new_width) as usize;
            let copy_height = old_height.min(new_height) as usize;

            for y in 0..copy_height {
                let old_offset = y * (old_width as usize) * 4;
                let new_offset = y * (new_width as usize) * 4;
                let len = copy_width * 4;

                new_data[new_offset..new_offset + len]
                    .copy_from_slice(&old_data[old_offset..old_offset + len]);
            }
        }

        // Replace the old data with new data
        {
            let mut data = self.data.write().await;
            *data = new_data;
        }

        // Update dimensions atomically
        self.width.store(new_width, AtomicOrdering::Release);
        self.height.store(new_height, AtomicOrdering::Release);

        // Reset prev_data to match new size
        {
            let mut prev = self.prev_data.write().await;
            *prev = vec![0u8; new_size];
        }

        // Mark entire framebuffer as dirty after resize
        self.mark_dirty_region(0, 0, new_width, new_height).await;

        Ok(())
    }

    /// Performs a copy rectangle operation within the framebuffer (standard VNC protocol style).
    ///
    /// This method copies pixel data from one region of the framebuffer to another,
    /// handling overlapping regions correctly by choosing the appropriate iteration direction.
    /// This is the equivalent of standard VNC protocol's `rfbDoCopyRegion` function.
    ///
    /// # Arguments
    ///
    /// * `dest_x` - The X coordinate of the destination rectangle.
    /// * `dest_y` - The Y coordinate of the destination rectangle.
    /// * `width` - The width of the rectangle to copy.
    /// * `height` - The height of the rectangle to copy.
    /// * `dx` - The X offset from destination to source (`src_x` = `dest_x` + dx).
    /// * `dy` - The Y offset from destination to source (`src_y` = `dest_y` + dy).
    ///
    /// # Returns
    ///
    /// `Ok(())` if the copy is successful.
    ///
    /// # Errors
    ///
    /// Returns `Err(String)` if the source or destination rectangle is out of bounds.
    #[allow(clippy::cast_possible_truncation)] // Region dimensions are u16, indexing math uses i32 for signed offsets
    #[allow(clippy::cast_sign_loss)] // Bounds-checked conversion from i32 to usize
    pub async fn do_copy_region(
        &self,
        dest_x: u16,
        dest_y: u16,
        width: u16,
        height: u16,
        dx: i16,
        dy: i16,
    ) -> Result<(), String> {
        // Calculate source coordinates
        let src_x = (i32::from(dest_x) + i32::from(dx)) as u16;
        let src_y = (i32::from(dest_y) + i32::from(dy)) as u16;

        // Validate bounds
        if dest_x.saturating_add(width) > self.width()
            || dest_y.saturating_add(height) > self.height()
        {
            return Err(format!(
                "Destination rectangle out of bounds: ({}, {}, {}, {}) exceeds ({}, {})",
                dest_x,
                dest_y,
                width,
                height,
                self.width(),
                self.height()
            ));
        }

        if src_x.saturating_add(width) > self.width()
            || src_y.saturating_add(height) > self.height()
        {
            return Err(format!(
                "Source rectangle out of bounds: ({}, {}, {}, {}) exceeds ({}, {})",
                src_x,
                src_y,
                width,
                height,
                self.width(),
                self.height()
            ));
        }

        let mut data = self.data.write().await;
        let fb_width = self.width() as usize;
        let row_bytes = width as usize * 4;

        // Copy rectangle within framebuffer
        // Choose iteration direction based on dx/dy to handle overlapping regions correctly
        // (standard VNC protocol uses sraRgnGetReverseIterator for this)
        if dy < 0 {
            // Copy top to bottom (forward)
            for row in 0..height {
                let src_offset = ((src_y + row) as usize * fb_width + src_x as usize) * 4;
                let dest_offset = ((dest_y + row) as usize * fb_width + dest_x as usize) * 4;

                // Use copy_within for safe overlapping copies
                data.copy_within(src_offset..src_offset + row_bytes, dest_offset);
            }
        } else {
            // Copy bottom to top (reverse)
            for row in (0..height).rev() {
                let src_offset = ((src_y + row) as usize * fb_width + src_x as usize) * 4;
                let dest_offset = ((dest_y + row) as usize * fb_width + dest_x as usize) * 4;

                // Use copy_within for safe overlapping copies
                data.copy_within(src_offset..src_offset + row_bytes, dest_offset);
            }
        }

        drop(data); // Release lock before save_state

        // Update prev_data for future copy detection
        self.save_state().await;

        Ok(())
    }
}
