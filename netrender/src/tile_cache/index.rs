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
//!   index at hash time. Merging rather than concatenating is what makes
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
    /// gives `floor(a.x0/T) ..= ceil(a.x1/T) - 1`. Degenerate
    /// (zero-width) and inverted AABBs fall out of the same arithmetic:
    /// a zero-width box still covers the tile it sits in, and an
    /// inverted one yields an empty range.
    fn cover(&self, aabb: [f32; 4]) -> Option<(i32, i32, i32, i32)> {
        if self.n_cols <= 0 || self.n_rows <= 0 {
            return None;
        }
        // Non-finite bounds make the index arithmetic meaningless. Take
        // the whole grid instead: over-marking costs one redundant
        // re-lower, under-marking leaves a stale tile on screen.
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
            for row in row0..=row1 {
                let base = (row * grid.n_cols) as usize;
                for col in col0..=col1 {
                    self.bins[base + col as usize].push((idx, entry));
                }
            }
        }
    }

    /// Dependency hash for one tile. Empty tiles all hash to the prefix,
    /// so they are never spuriously dirty.
    pub(super) fn tile_hash(&self, col: i32, row: i32) -> u64 {
        let bin: &[Entry] = self
            .grid
            .bin_index(col, row)
            .and_then(|i| self.bins.get(i))
            .map_or(&[], |v| v.as_slice());

        let mut h = DefaultHasher::new();
        h.write_u64(self.prefix);

        // Merge the tile's own ops with the always-applies layer ops,
        // ascending by op index, so painter order survives.
        let (mut i, mut j) = (0usize, 0usize);
        while i < bin.len() || j < self.global.len() {
            let take_bin = match (bin.get(i), self.global.get(j)) {
                (Some(b), Some(g)) => b.0 < g.0,
                (Some(_), None) => true,
                _ => false,
            };
            if take_bin {
                h.write_u64(bin[i].1);
                i += 1;
            } else {
                h.write_u64(self.global[j].1);
                j += 1;
            }
        }
        h.finish()
    }
}

// =============================================================================
// Differential tests against the pre-E3 reference implementation
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::{
        FontBlob, Glyph, Scene, SceneClip, SceneLayer, SceneStroke, SceneStrokeCap,
        SceneStrokeJoin, Transform, NO_CLIP, SHARP_CLIP,
    };
    use std::sync::Arc;
    use vello::peniko::Blob;

    const TILE: u32 = 64;
    const DIM: u32 = 256;

    fn grid() -> TileGrid {
        TileGrid::new(TILE, DIM.div_ceil(TILE), DIM.div_ceil(TILE))
    }

    /// Tiles whose hash differs between `a` and `b`, via the pre-E3
    /// per-tile rescan. This is the oracle.
    fn dirty_reference(a: &Scene, b: &Scene) -> Vec<(i32, i32)> {
        let g = grid();
        let mut out = Vec::new();
        for row in 0..g.n_rows {
            for col in 0..g.n_cols {
                let rect = [
                    (col * TILE as i32) as f32,
                    (row * TILE as i32) as f32,
                    ((col + 1) * TILE as i32) as f32,
                    ((row + 1) * TILE as i32) as f32,
                ];
                let ha = super::super::hash::hash_tile_deps_reference(a, rect);
                let hb = super::super::hash::hash_tile_deps_reference(b, rect);
                if ha != hb {
                    out.push((col, row));
                }
            }
        }
        out
    }

    /// Same question, via the E3 index.
    fn dirty_fast(a: &Scene, b: &Scene) -> Vec<(i32, i32)> {
        let g = grid();
        let mut ia = FrameIndex::default();
        let mut ib = FrameIndex::default();
        ia.build(a, g);
        ib.build(b, g);
        let mut out = Vec::new();
        for row in 0..g.n_rows {
            for col in 0..g.n_cols {
                if ia.tile_hash(col, row) != ib.tile_hash(col, row) {
                    out.push((col, row));
                }
            }
        }
        out
    }

    #[track_caller]
    fn assert_same_dirty_set(label: &str, a: &Scene, b: &Scene) {
        let expected = dirty_reference(a, b);
        let actual = dirty_fast(a, b);
        assert_eq!(
            actual, expected,
            "{label}: E3 index disagreed with the pre-E3 reference on which tiles are dirty"
        );
    }

    fn font(scene: &mut Scene) -> u32 {
        scene.push_font(FontBlob {
            data: Blob::new(Arc::new(vec![0xABu8; 64])),
            index: 0,
        })
    }

    fn stroke_at(x0: f32, y0: f32, x1: f32, y1: f32, color: [f32; 4]) -> SceneStroke {
        SceneStroke {
            x0,
            y0,
            x1,
            y1,
            color,
            stroke_width: 4.0,
            stroke_corner_radii: [0.0; 4],
            transform_id: 0,
            clip_rect: NO_CLIP,
            clip_corner_radii: SHARP_CLIP,
            cap: SceneStrokeCap::Butt,
            join: SceneStrokeJoin::Miter,
            dash_pattern: Vec::new(),
            dash_offset: 0.0,
        }
    }

    /// A scene touching every op kind the index bins, plus transforms
    /// and a nested layer scope.
    fn rich(nudge: f32, color: [f32; 4]) -> Scene {
        let mut s = Scene::new(DIM, DIM);
        let xf = s.push_transform(Transform::translate_2d(12.0 + nudge, 8.0));

        s.push_rect(0.0, 0.0, DIM as f32, DIM as f32, [1.0, 1.0, 1.0, 1.0]);
        s.push_rect(10.0, 10.0, 70.0, 70.0, color);
        s.push_rect_transformed(90.0, 20.0, 150.0, 60.0, [0.2, 0.4, 0.8, 1.0], xf);

        s.push_stroke_op(stroke_at(20.0 + nudge, 120.0, 200.0, 124.0, color));

        let f = font(&mut s);
        let glyphs: Vec<Glyph> = (0..12)
            .map(|i| Glyph {
                id: i + 4,
                x: 16.0 + i as f32 * 9.0 + nudge,
                y: 180.0,
            })
            .collect();
        s.push_glyph_run(f, 16.0, glyphs, [0.1, 0.1, 0.1, 1.0]);

        s.push_layer(SceneLayer::clip(SceneClip::Rect {
            rect: [40.0, 40.0, 220.0, 220.0],
            radii: [0.0; 4],
        }));
        s.push_rect(50.0, 50.0, 120.0, 120.0, [0.9, 0.2, 0.2, 0.5]);
        s.ops.push(SceneOp::PopLayer);

        s
    }

    #[test]
    fn agrees_when_nothing_changed() {
        let a = rich(0.0, [1.0, 0.0, 0.0, 1.0]);
        let b = rich(0.0, [1.0, 0.0, 0.0, 1.0]);
        assert!(dirty_reference(&a, &b).is_empty(), "oracle sanity");
        assert_same_dirty_set("identical scenes", &a, &b);
    }

    #[test]
    fn agrees_on_a_color_change() {
        let a = rich(0.0, [1.0, 0.0, 0.0, 1.0]);
        let b = rich(0.0, [0.0, 1.0, 0.0, 1.0]);
        assert!(
            !dirty_reference(&a, &b).is_empty(),
            "a color change must dirty something"
        );
        assert_same_dirty_set("color change", &a, &b);
    }

    #[test]
    fn agrees_on_a_positional_nudge() {
        let a = rich(0.0, [1.0, 0.0, 0.0, 1.0]);
        let b = rich(7.0, [1.0, 0.0, 0.0, 1.0]);
        assert_same_dirty_set("positional nudge", &a, &b);
    }

    #[test]
    fn agrees_on_scene_level_alpha_and_blend() {
        let a = rich(0.0, [1.0, 0.0, 0.0, 1.0]);
        let mut b = rich(0.0, [1.0, 0.0, 0.0, 1.0]);
        b.root_alpha = 0.5;
        assert_eq!(
            dirty_fast(&a, &b).len(),
            (grid().n_cols * grid().n_rows) as usize,
            "scene alpha is global; every tile must go dirty"
        );
        assert_same_dirty_set("root alpha", &a, &b);
    }

    /// Painter order is consumer push order, so reordering two
    /// primitives that share a tile changes the render and must change
    /// the hash. This is the property the bin/global merge exists for.
    #[test]
    fn agrees_when_two_overlapping_primitives_swap_order() {
        let mut a = Scene::new(DIM, DIM);
        a.push_rect(10.0, 10.0, 60.0, 60.0, [1.0, 0.0, 0.0, 1.0]);
        a.push_rect(20.0, 20.0, 70.0, 70.0, [0.0, 0.0, 1.0, 1.0]);

        let mut b = Scene::new(DIM, DIM);
        b.push_rect(20.0, 20.0, 70.0, 70.0, [0.0, 0.0, 1.0, 1.0]);
        b.push_rect(10.0, 10.0, 60.0, 60.0, [1.0, 0.0, 0.0, 1.0]);

        assert!(
            !dirty_reference(&a, &b).is_empty(),
            "a reorder must dirty something"
        );
        assert_same_dirty_set("reordered overlapping rects", &a, &b);
    }

    /// Moving a primitive across a layer boundary changes the render.
    /// Concatenating bins after globals instead of merging would miss
    /// this.
    #[test]
    fn agrees_when_a_primitive_moves_across_a_layer_boundary() {
        let layer = || {
            SceneLayer::clip(SceneClip::Rect {
                rect: [0.0, 0.0, DIM as f32, DIM as f32],
                radii: [0.0; 4],
            })
        };

        let mut a = Scene::new(DIM, DIM);
        a.push_rect(10.0, 10.0, 60.0, 60.0, [1.0, 0.0, 0.0, 1.0]);
        a.push_layer(layer());
        a.ops.push(SceneOp::PopLayer);

        let mut b = Scene::new(DIM, DIM);
        b.push_layer(layer());
        b.push_rect(10.0, 10.0, 60.0, 60.0, [1.0, 0.0, 0.0, 1.0]);
        b.ops.push(SceneOp::PopLayer);

        assert!(
            !dirty_reference(&a, &b).is_empty(),
            "crossing a layer boundary must dirty something"
        );
        assert_same_dirty_set("primitive crosses a layer boundary", &a, &b);
    }

    /// Primitives entirely outside the grid contribute to no tile, and
    /// one spanning the whole viewport contributes to all of them.
    #[test]
    fn agrees_on_out_of_bounds_and_full_bleed_primitives() {
        let mut a = Scene::new(DIM, DIM);
        a.push_rect(-500.0, -500.0, -400.0, -400.0, [1.0, 0.0, 0.0, 1.0]);
        a.push_rect(900.0, 900.0, 1000.0, 1000.0, [1.0, 0.0, 0.0, 1.0]);

        let mut b = Scene::new(DIM, DIM);
        b.push_rect(-500.0, -500.0, -400.0, -400.0, [0.0, 1.0, 0.0, 1.0]);
        b.push_rect(900.0, 900.0, 1000.0, 1000.0, [1.0, 0.0, 0.0, 1.0]);
        assert!(
            dirty_reference(&a, &b).is_empty(),
            "off-grid changes touch no tile"
        );
        assert_same_dirty_set("off-grid change", &a, &b);

        let mut c = Scene::new(DIM, DIM);
        c.push_rect(0.0, 0.0, DIM as f32, DIM as f32, [1.0, 1.0, 1.0, 1.0]);
        let mut d = Scene::new(DIM, DIM);
        d.push_rect(0.0, 0.0, DIM as f32, DIM as f32, [0.5, 0.5, 0.5, 1.0]);
        assert_eq!(
            dirty_fast(&c, &d).len(),
            (grid().n_cols * grid().n_rows) as usize,
            "a full-bleed rect covers every tile"
        );
        assert_same_dirty_set("full-bleed change", &c, &d);
    }

    /// A zero-area primitive still sits in exactly one tile under the
    /// half-open intersection rule, and the cover range has to agree.
    #[test]
    fn agrees_on_degenerate_and_tile_aligned_primitives() {
        let mut a = Scene::new(DIM, DIM);
        a.push_rect(64.0, 64.0, 64.0, 96.0, [1.0, 0.0, 0.0, 1.0]);
        a.push_rect(128.0, 0.0, 192.0, 64.0, [0.0, 0.0, 1.0, 1.0]);

        let mut b = Scene::new(DIM, DIM);
        b.push_rect(64.0, 64.0, 64.0, 96.0, [0.0, 1.0, 0.0, 1.0]);
        b.push_rect(128.0, 0.0, 192.0, 64.0, [0.0, 0.0, 1.0, 1.0]);

        assert_same_dirty_set("degenerate + tile-aligned", &a, &b);
    }
}
