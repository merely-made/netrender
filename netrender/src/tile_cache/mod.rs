// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Phase 7A — picture-cache invalidation algorithm.
//!
//! This module implements the *algorithm* portion of Phase 7: hashing
//! per-tile primitive dependencies, frame-stamping seen tiles, and
//! reporting which tiles changed between two `invalidate(scene)` calls.
//! Tile texture allocation + per-tile rendering land in Phase 7B; the
//! `Tile` struct intentionally has no `texture` field yet.
//!
//! Algorithm:
//!   1. Tick `current_frame`.
//!   2. For each tile in the viewport grid: hash every primitive
//!      whose world AABB intersects the tile (in painter order),
//!      compare against the tile's `last_hash`, and report new or
//!      changed tiles as dirty (updating the cached hash). Mark
//!      every tile seen this frame.
//!
//!   3. Evict tiles whose `last_seen_frame` is more than `RETAIN_FRAMES`
//!      stale (Arc drop, wgpu reclaims memory in Phase 7B+).
//!
//! The hash uses `DefaultHasher` (SipHash-1-3) — fast, deterministic
//! within a process, and good enough to make collisions astronomically
//! unlikely for the kind of state that fits in a primitive struct.

use std::collections::HashMap;

use crate::scene::{
    Scene, SceneGlyphRun, SceneGradient, SceneImage, SceneRect, SceneShape, SceneStroke, Transform,
};

mod hash;
mod index;
pub(crate) mod op_hash;
use index::{FrameIndex, TileGrid};
/// Integer (col, row) coordinate of a tile within the cache grid.
/// Tile (cx, cy) covers world rect `(cx*T, cy*T, (cx+1)*T, (cy+1)*T)`.
pub type TileCoord = (i32, i32);

/// Number of frames a tile may go un-touched before eviction.
const RETAIN_FRAMES: u64 = 4;

/// Sentinel value for a freshly-inserted tile's `last_hash`. Collision
/// with a real hash is harmless because new tiles are also detected via
/// `last_seen_frame == 0`.
const FRESH_HASH_SENTINEL: u64 = 0xDEAD_BEEF_DEAD_BEEF;

/// One tile's bookkeeping: the world rect this tile covers, the last
/// computed dependency hash, the last-seen frame stamp, and the most
/// recent frame the tile was reported dirty by `invalidate`.
///
/// `last_dirty_frame` powers the A3 tile-dirty visualizer (see
/// [`TileCache::recent_dirty_tiles`]); it lags `last_seen_frame` by
/// any number of frames in which the tile was unchanged.
#[derive(Clone)]
pub(crate) struct Tile {
    pub world_rect: [f32; 4],
    pub last_hash: u64,
    pub last_seen_frame: u64,
    pub last_dirty_frame: u64,
}

/// Picture-cache state. One instance per cached picture (Phase 7A ships
/// with one per `Renderer`; multi-picture caching lands when a concrete
/// consumer surfaces the need).
pub struct TileCache {
    tile_size: u32,
    pub(crate) glyph_scale: Option<Option<u32>>,
    pub(crate) tiles: HashMap<TileCoord, Tile>,
    current_frame: u64,
    dirty_count_last_invalidate: usize,
    /// Roadmap E3 — per-frame op-to-tile index. Held here rather than
    /// built locally so its per-tile bins keep their allocations across
    /// frames.
    frame_index: FrameIndex,
}

impl TileCache {
    /// Construct a new tile cache with the given square tile size in
    /// device pixels. `tile_size` must be > 0.
    pub fn new(tile_size: u32) -> Self {
        assert!(tile_size > 0, "tile_size must be > 0");
        Self {
            tile_size,
            glyph_scale: None,
            tiles: HashMap::new(),
            current_frame: 0,
            dirty_count_last_invalidate: 0,
            frame_index: FrameIndex::default(),
        }
    }

    pub fn tile_size(&self) -> u32 {
        self.tile_size
    }

    pub fn current_frame(&self) -> u64 {
        self.current_frame
    }

    /// Number of tiles reported dirty by the most recent `invalidate`
    /// call, or 0 if `invalidate` has never been called.
    pub fn dirty_count_last_invalidate(&self) -> usize {
        self.dirty_count_last_invalidate
    }

    /// Number of tiles currently cached (visited within `RETAIN_FRAMES`).
    pub fn tile_count(&self) -> usize {
        self.tiles.len()
    }

    /// Forget all dependency hashes and cached tile coordinates.
    ///
    /// A GPU image override whose dimensions change receives a new Vello
    /// `ImageData` identity. The scene op still contains the same `ImageKey`,
    /// so its ordinary dependency hash cannot observe that replacement. The
    /// owner uses this narrow reset before the next frame to make every tile
    /// rebuild against the new image identity.
    pub(crate) fn clear(&mut self) {
        self.tiles.clear();
        self.frame_index = FrameIndex::default();
        self.dirty_count_last_invalidate = 0;
    }

    /// Roadmap A3 — return tiles that became dirty within the last
    /// `window_frames` invalidate calls, paired with an `age_frac` in
    /// `[0.0, 1.0]` where `0.0` means dirtied this frame and `1.0`
    /// means just exiting the window. Used by the tile-dirty
    /// visualizer to fade overlay opacity with age.
    ///
    /// Tiles never dirtied (or dirtied longer than `window_frames`
    /// ago) are excluded. `window_frames = 0` returns an empty Vec.
    pub fn recent_dirty_tiles(&self, window_frames: u64) -> Vec<([f32; 4], f32)> {
        if window_frames == 0 {
            return Vec::new();
        }
        let now = self.current_frame;
        let mut out = Vec::new();
        for tile in self.tiles.values() {
            if tile.last_dirty_frame == 0 {
                continue;
            }
            let age = now.saturating_sub(tile.last_dirty_frame);
            if age >= window_frames {
                continue;
            }
            let age_frac = age as f32 / window_frames as f32;
            out.push((tile.world_rect, age_frac));
        }
        out
    }

    /// World-space rect for the named tile, or `None` if the tile is
    /// not in the cache.
    pub fn tile_world_rect(&self, coord: TileCoord) -> Option<[f32; 4]> {
        self.tiles.get(&coord).map(|t| t.world_rect)
    }

    /// Tick the frame counter, recompute each tile's dependency hash
    /// against `scene`, and return the list of tile coords whose
    /// dependencies changed (= need re-rendering in Phase 7B).
    ///
    /// Tiles outside the new viewport grid are evicted after
    /// `RETAIN_FRAMES` frames of absence.
    pub fn invalidate(&mut self, scene: &Scene) -> Vec<TileCoord> {
        self.current_frame += 1;
        let frame = self.current_frame;
        let tile_size = self.tile_size;

        let n_cols = scene.viewport_width.div_ceil(tile_size);
        let n_rows = scene.viewport_height.div_ceil(tile_size);

        // Roadmap E3 — file every op into the bins of the tiles it
        // covers, once, so the per-tile pass below reads its own bin
        // instead of rescanning the whole op list. Was O(tiles × ops).
        self.frame_index.build(
            scene,
            TileGrid::new(tile_size, n_cols, n_rows),
            self.glyph_scale
                .flatten()
                .is_some_and(|bits| f32::from_bits(bits) < 1.0),
        );

        let mut dirty = Vec::new();

        for row in 0..n_rows as i32 {
            for col in 0..n_cols as i32 {
                let coord = (col, row);
                let world_rect = [
                    (col * tile_size as i32) as f32,
                    (row * tile_size as i32) as f32,
                    ((col + 1) * tile_size as i32) as f32,
                    ((row + 1) * tile_size as i32) as f32,
                ];

                let new_hash = self.frame_index.tile_hash(col, row);

                let tile = self.tiles.entry(coord).or_insert(Tile {
                    world_rect,
                    last_hash: FRESH_HASH_SENTINEL,
                    last_seen_frame: 0,
                    last_dirty_frame: 0,
                });

                let is_new = tile.last_seen_frame == 0;
                let changed = tile.last_hash != new_hash;

                if is_new || changed {
                    tile.last_hash = new_hash;
                    tile.last_dirty_frame = frame;
                    dirty.push(coord);
                }
                tile.last_seen_frame = frame;
            }
        }

        // Retain heuristic: evict tiles not seen recently.
        let cutoff = frame.saturating_sub(RETAIN_FRAMES);
        self.tiles.retain(|_, t| t.last_seen_frame > cutoff);

        self.dirty_count_last_invalidate = dirty.len();
        dirty
    }

    /// Tiles of the frame most recently passed to [`Self::invalidate`] whose
    /// ops draw any of `keys` (image or pattern-tile ops). For callers that
    /// swap the pixels behind a key without changing the ops naming it.
    pub(crate) fn tiles_drawing_images(
        &self,
        scene: &Scene,
        keys: &[crate::scene::ImageKey],
    ) -> Vec<TileCoord> {
        use crate::scene::SceneOp;
        self.tiles
            .iter()
            .filter(|(_, tile)| tile.last_seen_frame == self.current_frame)
            .map(|(coord, _)| *coord)
            .filter(|&coord| {
                self.op_indices(coord).any(|i| match &scene.ops[i] {
                    SceneOp::Image(image) => keys.contains(&image.key),
                    SceneOp::Pattern(pattern) => keys.contains(&pattern.tile),
                    _ => false,
                })
            })
            .collect()
    }

    /// Ordered scene-op indices affecting `coord` in the frame most recently
    /// passed to [`Self::invalidate`]. The returned stream is the same one
    /// used to calculate the tile dependency hash.
    pub(crate) fn op_indices(&self, coord: TileCoord) -> impl Iterator<Item = usize> + '_ {
        self.frame_index.op_indices(coord.0, coord.1)
    }
}

// ── Geometry ───────────────────────────────────────────────────────────────

/// World-space AABB of a primitive's `[x0, y0, x1, y1]` local rect
/// after applying the 2-D affine portion of `transforms[transform_id]`.
/// Identity transform (id 0) is a fast path that returns `local`
/// unchanged.
///
/// Used once per operation by the tile index, which supplies dependency
/// hashing and tile rebuilding with the same membership. Conservative on
/// rotated rects (the AABB is larger than the rotated rect's true
/// bounds) — correct in both directions: over-marking dirty is safe,
/// over-including in a tile is safe (NDC clipping crops the extras).
pub(crate) fn world_aabb(local: [f32; 4], transform_id: u32, scene: &Scene) -> [f32; 4] {
    if transform_id == 0 {
        local
    } else {
        let t = &scene.transforms[transform_id as usize];
        transformed_aabb(local, t)
    }
}

fn world_aabb_rect(rect: &SceneRect, scene: &Scene) -> [f32; 4] {
    world_aabb(
        [rect.x0, rect.y0, rect.x1, rect.y1],
        rect.transform_id,
        scene,
    )
}

fn world_aabb_image(image: &SceneImage, scene: &Scene) -> [f32; 4] {
    world_aabb(
        [image.x0, image.y0, image.x1, image.y1],
        image.transform_id,
        scene,
    )
}

fn world_aabb_gradient(g: &SceneGradient, scene: &Scene) -> [f32; 4] {
    world_aabb([g.x0, g.y0, g.x1, g.y1], g.transform_id, scene)
}

pub(crate) fn world_aabb_glyph_run(r: &SceneGlyphRun, scene: &Scene) -> Option<[f32; 4]> {
    use skrifa::MetadataProvider;
    if r.glyphs.is_empty() {
        return None;
    }
    let unbounded = [
        f32::NEG_INFINITY,
        f32::NEG_INFINITY,
        f32::INFINITY,
        f32::INFINITY,
    ];
    let Some(blob) = scene
        .fonts
        .get(r.font_id as usize)
        .filter(|_| r.font_id != 0)
    else {
        return Some(unbounded);
    };
    let Ok(font) = skrifa::FontRef::from_index(blob.data.data(), blob.index) else {
        return Some(unbounded);
    };
    if !r.font_size.is_finite()
        || r.font_size <= 0.0
        || r.font_axis_values.iter().any(|(_, v)| !v.is_finite())
        || scene
            .transforms
            .get(r.transform_id as usize)
            .is_none_or(|t| t.m.iter().any(|v| !v.is_finite()))
        || [
            *b"COLR", *b"CPAL", *b"CBDT", *b"CBLC", *b"EBDT", *b"EBLC", *b"sbix", *b"SVG ",
            *b"bdat", *b"bloc",
        ]
        .iter()
        .any(|tag| font.table_data(skrifa::Tag::new(tag)).is_some())
    {
        return Some(unbounded);
    }
    let settings = r
        .font_axis_values
        .iter()
        .filter_map(|(tag, value)| std::str::from_utf8(tag).ok().map(|tag| (tag, *value)));
    let location = font.axes().location(settings);
    let outlines = font.outline_glyphs();
    let mut glyph_bounds = HashMap::<u32, Option<[f32; 4]>>::new();
    let mut local = [
        f32::INFINITY,
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::NEG_INFINITY,
    ];
    for g in &r.glyphs {
        if !g.x.is_finite() || !g.y.is_finite() {
            return Some(unbounded);
        }
        let b = glyph_bounds.entry(g.id).or_insert_with(|| {
            let glyph = outlines.get(skrifa::GlyphId::new(g.id))?;
            let mut pen = skrifa::outline::pen::ControlBoundsPen::default();
            glyph
                .draw(
                    skrifa::outline::DrawSettings::unhinted(
                        skrifa::instance::Size::new(r.font_size),
                        &location,
                    ),
                    &mut pen,
                )
                .ok()?;
            // Successful empty outlines have no ink; a zero box still keeps
            // their origin in the conservative aggregate.
            Some(
                pen.bounding_box()
                    .map_or([0.0; 4], |b| [b.x_min, b.y_min, b.x_max, b.y_max]),
            )
        });
        let Some(b) = *b else {
            return Some(unbounded);
        };
        let points = [g.x + b[0], g.y - b[3], g.x + b[2], g.y - b[1]];
        if points.iter().any(|v| !v.is_finite()) {
            return Some(unbounded);
        }
        local[0] = local[0].min(points[0]).min(g.x - r.font_size);
        local[1] = local[1].min(points[1]).min(g.y - r.font_size);
        local[2] = local[2].max(points[2]).max(g.x + r.font_size);
        local[3] = local[3].max(points[3]).max(g.y + r.font_size);
    }
    let mut world = world_aabb(local, r.transform_id, scene);
    if world.iter().any(|v| !v.is_finite()) {
        return Some(unbounded);
    }
    let margin = 2.0 + world.iter().fold(0.0_f32, |m, v| m.max(v.abs())) * 1e-6;
    world[0] -= margin;
    world[1] -= margin;
    world[2] += margin;
    world[3] += margin;
    Some(world)
}

pub(crate) fn world_aabb_shape(s: &SceneShape, scene: &Scene) -> Option<[f32; 4]> {
    let local = s.path.local_aabb()?;
    // Inflate by half stroke width if a stroke is present, same
    // reasoning as world_aabb_stroke.
    let half = s.stroke.as_ref().map_or(0.0, |st| st.width * 0.5);
    let inflated = [
        local[0] - half,
        local[1] - half,
        local[2] + half,
        local[3] + half,
    ];
    Some(world_aabb(inflated, s.transform_id, scene))
}

fn world_aabb_stroke(s: &SceneStroke, scene: &Scene) -> [f32; 4] {
    // Stroke extends `width / 2` outward from the path bounds.
    // Inflate the AABB by half the stroke width before transforming
    // so the tile filter doesn't miss tiles the stroke pen reaches
    // into.
    let half = s.stroke_width * 0.5;
    let inflated = [s.x0 - half, s.y0 - half, s.x1 + half, s.y1 + half];
    world_aabb(inflated, s.transform_id, scene)
}

/// AABB of `[x0, y0, x1, y1]` after applying the 2-D affine portion of
/// `t` to each corner. Conservative: rotated rects produce a larger AABB
/// than the rotated rect's true bounds.
fn transformed_aabb(rect: [f32; 4], t: &Transform) -> [f32; 4] {
    let corners = [
        (rect[0], rect[1]),
        (rect[2], rect[1]),
        (rect[0], rect[3]),
        (rect[2], rect[3]),
    ];
    let mut min_x = f32::INFINITY;
    let mut min_y = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut max_y = f32::NEG_INFINITY;
    for (x, y) in corners {
        // Column-major mat4 × (x, y, 0, 1):
        //   x' = m[0]*x + m[4]*y + m[12]
        //   y' = m[1]*x + m[5]*y + m[13]
        let tx = t.m[0] * x + t.m[4] * y + t.m[12];
        let ty = t.m[1] * x + t.m[5] * y + t.m[13];
        min_x = min_x.min(tx);
        min_y = min_y.min(ty);
        max_x = max_x.max(tx);
        max_y = max_y.max(ty);
    }
    [min_x, min_y, max_x, max_y]
}

/// Half-open AABB intersection: `a` and `b` overlap iff their
/// `[x0, x1) × [y0, y1)` regions share at least one pixel. Touching
/// edges (a.x1 == b.x0) do NOT count as intersection — matches the
/// half-open rasterization semantics of the brush pipelines.
pub(crate) fn aabb_intersects(a: [f32; 4], b: [f32; 4]) -> bool {
    !(a[2] <= b[0] || a[0] >= b[2] || a[3] <= b[1] || a[1] >= b[3])
}
