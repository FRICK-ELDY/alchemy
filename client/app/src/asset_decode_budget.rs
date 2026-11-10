//! GLB / glTF decode resource budgets (x0085 / improvement P1-2).
//!
//! Guarantees:
//! - input bytes checked **before** parse / buffer import
//! - declared + actual buffer bytes, decoded image bytes, and RGBA retain **only when
//!   conversion succeeds**, counted toward `expanded_bytes` before mesh `Vec` growth
//! - vertex / index caps use accessor **counts** before collecting into `Vec`
//! - image pixel caps applied **per decoded image** before keeping it
//! - textured mesh paths convert each image to RGBA **once** and share (`Arc`); no
//!   per-primitive RGBA `Vec` multiplication within budget
//!
//! Non-guarantees:
//! - peak inside a single `image` crate decode of one embedded texture (mitigated by
//!   input ≤32 MiB and fail-fast after each image)
//! - assets-server quarantine heuristics

use anyhow::{bail, Result};

/// Default max GLB / blob bytes accepted for decode (below assets upload 100 MiB).
pub const MAX_INPUT_BYTES: usize = 32 * 1024 * 1024;
/// Buffers + decoded image pixel bytes.
pub const MAX_EXPANDED_BYTES: usize = 128 * 1024 * 1024;
/// Total POSITION counts across collected primitives.
pub const MAX_VERTICES: usize = 500_000;
/// Total index counts (or sequential fill) across collected primitives.
pub const MAX_INDICES: usize = 1_500_000;
/// Sum of `width * height` across **all** decoded images in the asset.
pub const MAX_IMAGE_PIXELS: usize = 16_777_216; // 4096²

/// Running counters + hard caps for one GLB load.
#[derive(Clone, Debug)]
pub struct AssetDecodeBudget {
    pub max_input_bytes: usize,
    pub max_expanded_bytes: usize,
    pub max_vertices: usize,
    pub max_indices: usize,
    pub max_image_pixels: usize,
    expanded: usize,
    vertices: usize,
    indices: usize,
    image_pixels: usize,
}

impl AssetDecodeBudget {
    /// Production / Desktop+XR shared defaults.
    pub fn standard() -> Self {
        Self {
            max_input_bytes: MAX_INPUT_BYTES,
            max_expanded_bytes: MAX_EXPANDED_BYTES,
            max_vertices: MAX_VERTICES,
            max_indices: MAX_INDICES,
            max_image_pixels: MAX_IMAGE_PIXELS,
            expanded: 0,
            vertices: 0,
            indices: 0,
            image_pixels: 0,
        }
    }

    /// Test helper: custom caps, zeroed counters.
    #[cfg(test)]
    pub fn for_test(
        max_input_bytes: usize,
        max_expanded_bytes: usize,
        max_vertices: usize,
        max_indices: usize,
        max_image_pixels: usize,
    ) -> Self {
        Self {
            max_input_bytes,
            max_expanded_bytes,
            max_vertices,
            max_indices,
            max_image_pixels,
            expanded: 0,
            vertices: 0,
            indices: 0,
            image_pixels: 0,
        }
    }

    pub fn check_input_bytes(&self, len: usize) -> Result<()> {
        if len > self.max_input_bytes {
            bail!(
                "GLB decode budget: input {len} bytes exceeds max {}",
                self.max_input_bytes
            );
        }
        Ok(())
    }

    /// Pre-flight or post-load: add `n` toward the expanded budget.
    pub fn add_expanded_bytes(&mut self, n: usize) -> Result<()> {
        self.expanded = self.expanded.saturating_add(n);
        if self.expanded > self.max_expanded_bytes {
            bail!(
                "GLB decode budget: expanded {} bytes exceeds max {}",
                self.expanded,
                self.max_expanded_bytes
            );
        }
        Ok(())
    }

    pub fn add_vertices(&mut self, n: usize) -> Result<()> {
        self.vertices = self.vertices.saturating_add(n);
        if self.vertices > self.max_vertices {
            bail!(
                "GLB decode budget: vertices {} exceed max {}",
                self.vertices,
                self.max_vertices
            );
        }
        Ok(())
    }

    pub fn add_indices(&mut self, n: usize) -> Result<()> {
        self.indices = self.indices.saturating_add(n);
        if self.indices > self.max_indices {
            bail!(
                "GLB decode budget: indices {} exceed max {}",
                self.indices,
                self.max_indices
            );
        }
        Ok(())
    }

    pub fn add_image_pixels(&mut self, width: u32, height: u32) -> Result<()> {
        let n = (width as usize).saturating_mul(height as usize);
        self.image_pixels = self.image_pixels.saturating_add(n);
        if self.image_pixels > self.max_image_pixels {
            bail!(
                "GLB decode budget: image pixels {} exceed max {}",
                self.image_pixels,
                self.max_image_pixels
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_over_budget_errs() {
        let b = AssetDecodeBudget::for_test(8, 1024, 100, 100, 100);
        assert!(b.check_input_bytes(9).is_err());
        assert!(b.check_input_bytes(8).is_ok());
    }

    #[test]
    fn vertex_counter_errs_past_cap() {
        let mut b = AssetDecodeBudget::for_test(1024, 1024, 3, 100, 100);
        assert!(b.add_vertices(2).is_ok());
        assert!(b.add_vertices(2).is_err());
    }

    #[test]
    fn expanded_and_index_and_pixels_counters() {
        let mut b = AssetDecodeBudget::for_test(1024, 10, 100, 5, 4);
        assert!(b.add_expanded_bytes(6).is_ok());
        assert!(b.add_expanded_bytes(5).is_err());

        let mut b = AssetDecodeBudget::for_test(1024, 1024, 100, 5, 4);
        assert!(b.add_indices(5).is_ok());
        assert!(b.add_indices(1).is_err());

        let mut b = AssetDecodeBudget::for_test(1024, 1024, 100, 100, 4);
        assert!(b.add_image_pixels(2, 2).is_ok());
        assert!(b.add_image_pixels(1, 1).is_err());
    }
}
