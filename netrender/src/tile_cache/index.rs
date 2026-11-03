// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Roadmap E3 — per-frame op-to-tile index.
//!
//! Dirty detection used to ask, for every tile, "which of these ops
//! touch me?", recomputing each op's world AABB and full field hash once
//! per tile. That is O(tiles × ops) with an expensive constant, and it
//! measured at ~87% of a 4096² frame — more than the rasterizer.
//!
//! This module inverts the loop. One pass over the ops computes each
//! op's world AABB and a single `u64` digest of its fields, then files
//! that digest into the bin of every tile the AABB covers. Each tile
//! then hashes only its own bin. O(ops + Σ covered tiles + tiles).
//!
//! Two properties the old code had, that this has to keep:
//!
//! - **Painter order.** Bins store `(op_index, digest)` and are built by
//!   ascending op index, so a bin is already ordered. Layer push/pop ops
//!   apply to every tile and live in a separate list, merged back by
//!   index for hashing and tile rebuilding. Merging rather than concatenating makes
//!   moving a primitive across a `PushLayer` change the hash, which it
//!   must, because it changes the render.
//! - **Conservative layers.** `PushLayer` / `PopLayer` still dirty every
//!   tile. That is the pre-existing (deliberately conservative) rule and
//!   E3 does not touch it; tightening it to the layer's clip AABB is a
//!   separate change with its own correctness argument.
//!
//! The residual cost is `tiles × layer_ops` from that merge. Layer ops
//! are structurally far rarer than drawing ops, and it is strictly
//! better than the `tiles × all_ops` it replaces.

use std::collections::hash_map::DefaultHasher;
use std::hash::Hasher;

use crate::scene::{Scene, SceneOp};

use super::hash::{
    hash_aabb, hash_glyph_run, hash_gradient, hash_image, hash_pattern, hash_push_layer, hash_rect,
    hash_shape, hash_stroke,
};
use super::{
    world_aabb, world_aabb_glyph_run, world_aabb_gradient, world_aabb_image, world_aabb_rect,
    world_aabb_shape, world_aabb_stroke,
};

/// The viewport's tile grid for one frame.
#[derive(Clone, Copy, Default)]
pub(super) struct TileGrid {
    pub tile_size: f32,
    pub n_cols: i32,
    pub n_rows: i32,
}

impl TileGrid {
    pub(super) fn new(tile_size: u32, n_cols: u32, n_rows: u32) -> Self {
        Self {
            tile_size: tile_size as f32,
            n_cols: n_cols as i32,
            n_rows: n_rows as i32,
        }
    }

    fn tile_count(&self) -> usize {
        self.n_cols.max(0) as usize * self.n_rows.max(0) as usize
    }

    fn bin_index(&self, col: i32, row: i32) -> Option<usize> {
        if col < 0 || row < 0 || col >= self.n_cols || row >= self.n_rows {
            return None;
        }
        Some((row * self.n_cols + col) as usize)
    }

    /// Inclusive `(col0, row0, col1, row1)` grid range this AABB touches,
    /// or `None` if it touches no tile in the grid.
    ///
    /// Derived to agree exactly with [`super::aabb_intersects`], which
    /// treats both rects as half-open: `a` meets tile `c` on the x axis
    /// iff `a.x1 > c*T && a.x0 < (c+1)*T`. Solving that pair for `c`
    /// gives `floor(a.x0/T) ..= ceil(a.x1/T) - 1`. Degenerate and inverted
    /// AABBs are left unnormalized; their membership follows this range
    /// calculation and the legacy `aabb_intersects` predicate.
    fn cover(&self, aabb: [f32; 4]) -> Option<(i32, i32, i32, i32)> {
        if self.n_cols <= 0 || self.n_rows <= 0 {
            return None;
        }
        // Non-finite bounds make the index arithmetic meaningless. Use
        // the whole grid as a candidate range; `FrameIndex::build` then
        // applies the legacy AABB predicate to each candidate.
        if !aabb.iter().all(|v| v.is_finite()) {
            return Some((0, 0, self.n_cols - 1, self.n_rows - 1));
        }
        let t = self.tile_size;
        // Clamp in float space before the cast so an extreme coordinate
        // saturates instead of wrapping.
        let col0 = (aabb[0] / t).floor().max(0.0) as i32;
        let row0 = (aabb[1] / t).floor().max(0.0) as i32;
        let col1 = ((aabb[2] / t).ceil() - 1.0).min((self.n_cols - 1) as f32) as i32;
        let row1 = ((aabb[3] / t).ceil() - 1.0).min((self.n_rows - 1) as f32) as i32;
        if col0 > col1 || row0 > row1 {
            return None;
        }
        Some((col0, row0, col1, row1))
    }
}

/// One op's contribution: its index in painter order and a digest of its
/// world AABB plus its fields.
type Entry = (u32, u64);

/// Per-frame index. Owned by the [`super::TileCache`] so its allocations
/// survive across frames; [`Self::build`] clears rather than reallocates.
#[derive(Default)]
pub(super) struct FrameIndex {
    grid: TileGrid,
    /// One bin per grid tile, row-major, each ascending by op index.
    bins: Vec<Vec<Entry>>,
    /// Ops that apply to every tile (layer push/pop), ascending by index.
    global: Vec<Entry>,
    /// Digest of the scene-level alpha + blend mode, which every tile
    /// depends on.
    prefix: u64,
}

/// Digest one drawing op: its world AABB, then its fields.
fn digest(aabb: [f32; 4], fields: impl FnOnce(&mut DefaultHasher)) -> u64 {
    let mut h = DefaultHasher::new();
    hash_aabb(&mut h, aabb);
    fields(&mut h);
    h.finish()
}

impl FrameIndex {
    /// Rebuild for `scene` against `grid`. O(ops + Σ covered tiles).
    pub(super) fn build(&mut self, scene: &Scene, grid: TileGrid) {
        self.grid = grid;
        self.bins.resize_with(grid.tile_count(), Vec::new);
        for bin in &mut self.bins {
            bin.clear();
        }
        self.global.clear();

        let mut prefix = DefaultHasher::new();
        prefix.write_u32(scene.root_alpha.to_bits());
        prefix.write_u8(scene.root_blend_mode as u8);
        self.prefix = prefix.finish();

        for (idx, op) in scene.ops.iter().enumerate() {
            let idx = idx as u32;
            // `None` means the op contributes to no tile at all, which
            // matches the reference's behaviour for an op with no
            // derivable AABB (an empty glyph run, an empty path).
            let placed: Option<([f32; 4], u64)> = match op {
                SceneOp::Rect(r) => {
                    let aabb = world_aabb_rect(r, scene);
                    Some((aabb, digest(aabb, |h| hash_rect(h, r))))
                }
                SceneOp::Image(i) => {
                    let aabb = world_aabb_image(i, scene);
                    Some((aabb, digest(aabb, |h| hash_image(h, i))))
                }
                SceneOp::Pattern(p) => {
                    let aabb = world_aabb(p.extent, p.transform_id, scene);
                    Some((aabb, digest(aabb, |h| hash_pattern(h, p))))
                }
                SceneOp::Gradient(g) => {
                    let aabb = world_aabb_gradient(g, scene);
                    Some((aabb, digest(aabb, |h| hash_gradient(h, g))))
                }
                SceneOp::Stroke(s) => {
                    let aabb = world_aabb_stroke(s, scene);
                    Some((aabb, digest(aabb, |h| hash_stroke(h, s))))
                }
                SceneOp::Shape(s) => world_aabb_shape(s, scene)
                    .map(|aabb| (aabb, digest(aabb, |h| hash_shape(h, s)))),
                SceneOp::GlyphRun(r) => world_aabb_glyph_run(r, scene)
                    .map(|aabb| (aabb, digest(aabb, |h| hash_glyph_run(h, r)))),

                // Layer scope ops apply to every tile — see the module
                // docs on why this stays conservative.
                SceneOp::PushLayer(layer) => {
                    let mut h = DefaultHasher::new();
                    hash_push_layer(&mut h, layer);
                    self.global.push((idx, h.finish()));
                    continue;
                }
                SceneOp::PopLayer => {
                    let mut h = DefaultHasher::new();
                    h.write_u8(0xFF);
                    self.global.push((idx, h.finish()));
                    continue;
                }

                // Roadmap E4 — scenes containing placed fragments take
                // the retained-fragment master path and never reach the
                // tile cache. If one arrives anyway, treat it like a
                // layer op: applies-to-every-tile, hashing id, placement
                // id, and the placement matrix so any change dirties
                // everything. Conservative and correct.
                SceneOp::Fragment(f) => {
                    let mut h = DefaultHasher::new();
                    h.write_u64(f.id);
                    h.write_u32(f.transform_id);
                    for v in scene.transforms[f.transform_id as usize].m {
                        h.write_u32(v.to_bits());
                    }
                    self.global.push((idx, h.finish()));
                    continue;
                }
            };

            let Some((aabb, entry)) = placed else {
                continue;
            };
            let Some((col0, row0, col1, row1)) = grid.cover(aabb) else {
                continue;
            };
            let non_finite_aabb = !aabb.iter().all(|value| value.is_finite());
            for row in row0..=row1 {
                let base = (row * grid.n_cols) as usize;
                for col in col0..=col1 {
                    if non_finite_aabb {
                        // `cover` returns the whole grid for nonfinite bounds.
                        // Filter those candidates with the old full-scan
                        // predicate so rebuild membership exactly matches it.
                        let tile_rect = [
                            (col * grid.tile_size as i32) as f32,
                            (row * grid.tile_size as i32) as f32,
                            ((col + 1) * grid.tile_size as i32) as f32,
                            ((row + 1) * grid.tile_size as i32) as f32,
                        ];
                        if !super::aabb_intersects(aabb, tile_rect) {
                            continue;
                        }
                    }
                    self.bins[base + col as usize].push((idx, entry));
                }
            }
        }
    }

    /// Op indices affecting one tile, merged with global scope ops in
    /// painter order. Out-of-grid coordinates still receive global ops,
    /// matching the conservative dependency-hash behavior.
    pub(super) fn op_indices(&self, col: i32, row: i32) -> impl Iterator<Item = usize> + '_ {
        self.merged_entries(col, row)
            .map(|(index, _)| index as usize)
    }

    /// Merge the tile's spatial bin with always-applies ops. Both streams
    /// are ascending by scene index, so one shared merge keeps hashing and
    /// tile-scene construction aligned on membership and painter order.
    fn merged_entries(&self, col: i32, row: i32) -> impl Iterator<Item = Entry> + '_ {
        let bin: &[Entry] = self
            .grid
            .bin_index(col, row)
            .and_then(|i| self.bins.get(i))
            .map_or(&[], |v| v.as_slice());
        let mut i = 0usize;
        let mut j = 0usize;
        std::iter::from_fn(move || {
            if i >= bin.len() && j >= self.global.len() {
                return None;
            }
            let take_bin = match (bin.get(i), self.global.get(j)) {
                (Some(b), Some(g)) => b.0 < g.0,
                (Some(_), None) => true,
                _ => false,
            };
            if take_bin {
                let entry = bin[i];
                i += 1;
                Some(entry)
            } else {
                let entry = self.global[j];
                j += 1;
                Some(entry)
            }
        })
    }

    /// Dependency hash for one tile. Empty tiles all hash to the prefix,
    /// so they are never spuriously dirty.
    pub(super) fn tile_hash(&self, col: i32, row: i32) -> u64 {
        let mut h = DefaultHasher::new();
        h.write_u64(self.prefix);

        for (_, digest) in self.merged_entries(col, row) {
            h.write_u64(digest);
        }
        h.finish()
    }
}

// =============================================================================
// Differential tests against the pre-E3 reference implementation
// =============================================================================

#[cfg(test)]
mod tests;
