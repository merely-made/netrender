// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Master-scene composition for the vello tile rasterizer: per-frame master
//! texture pooling, dirty-tile rebuild, tile compose, the A3 dirty overlay, and
//! the per-tile scene filter. The public `compose_into` / `cached_image_blob_id`
//! / `vello_renderer_mut` entry points live here too (see [`super`]).

use vello::{
    Renderer,
    kurbo::{Affine, Rect},
    peniko::{BlendMode, Brush, Color, Compose, Fill, Mix},
};

use crate::scene::{ImageKey, Scene, SceneBlendMode, SceneOp};
use crate::tile_cache::TileCache;
use crate::vello_rasterizer::scene_to_vello_with_image_sources;

use super::{MasterEntry, VelloTileRasterizer};

pub(super) fn map_blend_mode(b: SceneBlendMode) -> BlendMode {
    let mix = match b {
        SceneBlendMode::Normal => Mix::Normal,
        SceneBlendMode::Multiply => Mix::Multiply,
        SceneBlendMode::Screen => Mix::Screen,
        SceneBlendMode::Overlay => Mix::Overlay,
        SceneBlendMode::Darken => Mix::Darken,
        SceneBlendMode::Lighten => Mix::Lighten,
    };
    BlendMode::new(mix, Compose::SrcOver)
}

impl VelloTileRasterizer {
    pub(super) fn ensure_master_texture(
        &mut self,
        width: u32,
        height: u32,
        format: wgpu::TextureFormat,
    ) {
        let needs_realloc = match &self.master_pool {
            Some(e) => e.width != width || e.height != height || e.format != format,
            None => true,
        };
        if !needs_realloc {
            return;
        }

        let texture = self
            .handles
            .device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("netrender path-b' master"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::STORAGE_BINDING
                    | wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
        self.master_pool = Some(MasterEntry {
            width,
            height,
            format,
            texture,
        });
        self.master_allocations += 1;
    }

    /// Run the same tile-cache update and master-scene composition
    /// as [`Self::render`], but append the result into a caller-
    /// provided `vello::Scene` with the given transform — instead
    /// of rendering to a texture.
    ///
    /// This is the C-architecture entry point: a caller (graphshell
    /// workbench, app-level compositor) holds a master `vello::Scene`
    /// for the whole frame and asks each consumer to compose its
    /// content into it. The caller does the single
    /// `vello::Renderer::render_to_texture` at end-of-frame; vello
    /// dedups font / image atlas slots across the appended sub-
    /// scenes via `Blob::id()`.
    ///
    /// Per `vello::Scene::append`: the operation is bytewise-cheap
    /// (per-encoding-element O(N), no GPU work), and `transform` is
    /// applied to every transform inside this rasterizer's master.
    /// Pass `Affine::IDENTITY` to compose at scene-space origin.
    ///
    /// `last_dirty_count` and `cached_tile_count` reflect the work
    /// done by this call exactly as they would for `render`.
    pub fn compose_into(
        &mut self,
        scene: &Scene,
        tile_cache: &mut TileCache,
        master: &mut vello::Scene,
        transform: Affine,
    ) {
        use crate::profiling::{FrameTimings, Span};
        let total_span = Span::start("total");
        let mut timings = FrameTimings::empty();

        let mut tile_scenes = std::mem::take(&mut self.tile_scenes);
        let local_master =
            self.build_master_scene_timed(scene, tile_cache, &mut tile_scenes, &mut timings);
        self.tile_scenes = tile_scenes;

        let append_span = Span::start("master_append");
        let xform = if transform == Affine::IDENTITY {
            None
        } else {
            Some(transform)
        };
        master.append(&local_master, xform);
        append_span.stop_recording(&mut timings);

        timings.total = total_span.stop();
        self.last_timings = Some(timings);
    }

    /// Internal: tile-cache update + master-scene composition with
    /// A4 timing instrumentation. Shared by [`Self::render`],
    /// [`Self::render_to_internal_master`], and
    /// [`Self::compose_into`]; each caller wraps this in its own
    /// outer total + per-format spans (vello_render, master_append,
    /// etc.) and finalises `self.last_timings`.
    pub(super) fn build_master_scene_timed(
        &mut self,
        scene: &Scene,
        tile_cache: &mut TileCache,
        tile_scenes: &mut std::collections::HashMap<crate::tile_cache::TileCoord, vello::Scene>,
        timings: &mut crate::profiling::FrameTimings,
    ) -> vello::Scene {
        use crate::profiling::Span;

        // Roadmap E4 — scenes placing retained fragments take the
        // retained master path (registry lookups + cached lowerings +
        // whole-master signature short-circuit) and skip the tile
        // cache. Fragment-free scenes keep the exact pre-E4 path below.
        if super::retained::has_fragments(scene) {
            return self.build_master_scene_fragments(scene, timings);
        }

        let refresh_span = Span::start("refresh_image_data");
        self.refresh_image_data(scene);
        refresh_span.stop_recording(timings);

        // Build the merged Path A + Path B image map once per frame
        // (Path B overrides win on key collision). Previously this
        // ran inside build_tile_scene and re-merged for every dirty
        // tile — O(N_images × N_dirty_tiles) instead of O(N_images).
        let invalidate_span = Span::start("tile_invalidate");
        let dirty = tile_cache.invalidate(scene);
        invalidate_span.stop_recording(timings);
        self.last_dirty_count = dirty.len();
        self.last_dirty_tiles = dirty.clone();

        let rebuild_span = Span::start("dirty_tile_rebuild");
        for &coord in &dirty {
            let world_rect = tile_cache
                .tile_world_rect(coord)
                .expect("dirty tile must be in tile_cache");
            let filtered = filter_scene_to_tile(scene, world_rect);
            let tile_scene = scene_to_vello_with_image_sources(
                &filtered,
                self.image_data.entries(),
                &self.image_overrides,
            );
            tile_scenes.insert(coord, tile_scene);
        }

        // Drop tile-Scenes whose coords were evicted from the tile
        // cache (e.g., scrolled out of viewport for RETAIN_FRAMES
        // frames).
        tile_scenes.retain(|coord, _| tile_cache.tile_world_rect(*coord).is_some());
        rebuild_span.stop_recording(timings);

        let compose_span = Span::start("master_compose");
        let master = self.compose_master(tile_cache, scene, tile_scenes);
        compose_span.stop_recording(timings);
        master
    }

    pub(super) fn refresh_image_data(&mut self, scene: &Scene) {
        self.image_data.refresh(scene);
    }

    /// Return the `peniko::Blob` id for the cached Path A image
    /// data under `key`, if any. Stable across frames as long as
    /// the key remains in `scene.image_sources` — used by tests
    /// to verify the cross-frame cache invariant.
    pub fn cached_image_blob_id(&self, key: ImageKey) -> Option<u64> {
        self.image_data.entries().get(&key).map(|img| img.data.id())
    }

    fn compose_master(
        &self,
        tile_cache: &TileCache,
        scene: &Scene,
        tile_scenes: &std::collections::HashMap<crate::tile_cache::TileCoord, vello::Scene>,
    ) -> vello::Scene {
        let mut master = vello::Scene::new();

        // Phase 12a' scene-level alpha + blend mode wrap. Skip the
        // outer layer when settings are at their defaults
        // (alpha = 1.0 and blend = Normal) so simple scenes don't
        // pay an extra layer.
        let scene_alpha = scene.root_alpha.clamp(0.0, 1.0);
        let scene_blend = map_blend_mode(scene.root_blend_mode);
        let needs_root_layer = scene_alpha < 1.0 || scene_blend.mix != Mix::Normal;
        if needs_root_layer {
            let viewport = Rect::new(
                0.0,
                0.0,
                scene.viewport_width as f64,
                scene.viewport_height as f64,
            );
            master.push_layer(
                Fill::NonZero,
                scene_blend,
                scene_alpha,
                Affine::IDENTITY,
                &viewport,
            );
        }

        // Vello scene append order is painter order. Keep it stable even though
        // retained tile storage is hash-backed: streamed replacements can
        // rebuild every tile with fresh font resources, and an arbitrary
        // append order can make equivalent frames produce different glyph
        // coverage in the composed scene.
        let mut ordered_tiles: Vec<_> = tile_scenes.iter().collect();
        ordered_tiles.sort_unstable_by_key(|(coord, _)| (coord.1, coord.0));
        for (coord, tile_scene) in ordered_tiles {
            // Get the world rect from the tile cache. If it's not
            // present (race with eviction), skip — the retain pass
            // above should have already pruned, so this is purely
            // defensive.
            let Some(world_rect) = tile_cache.tile_world_rect(*coord) else {
                continue;
            };
            let clip = Rect::new(
                world_rect[0] as f64,
                world_rect[1] as f64,
                world_rect[2] as f64,
                world_rect[3] as f64,
            );
            master.push_layer(Fill::NonZero, Mix::Normal, 1.0, Affine::IDENTITY, &clip);
            master.append(tile_scene, None);
            master.pop_layer();
        }

        if needs_root_layer {
            master.pop_layer();
        }

        // Roadmap A3 — translucent red wash on tiles dirtied within
        // the configured fade window. Painted *after* the root layer
        // pop so the overlay is not subject to scene-level alpha or
        // blend-mode wraps.
        if self.dirty_overlay_enabled {
            self.paint_dirty_overlay(&mut master, tile_cache);
        }

        log::debug!(target: "netrender::tile_scopes",
            "master_clip_records={};cached_tiles={}",
            master.encoding().n_clips, tile_scenes.len());
        master
    }

    /// Roadmap A3 — append a translucent red wash on top of every
    /// tile dirtied within `dirty_overlay_window_frames`. Opacity
    /// fades linearly with age. Caller decides whether to call this
    /// (gated on `dirty_overlay_enabled`).
    fn paint_dirty_overlay(&self, master: &mut vello::Scene, tile_cache: &TileCache) {
        // Peak alpha at age 0; decays to 0 at age = window. 0.4 is a
        // bright-enough wash to be visible over typical content
        // without obscuring it. Tune via fork if profiles surface a
        // legibility concern.
        const OVERLAY_PEAK_ALPHA: f32 = 0.4;

        let recent = tile_cache.recent_dirty_tiles(self.dirty_overlay_window_frames as u64);
        if recent.is_empty() {
            return;
        }
        for (rect, age_frac) in recent {
            let alpha = OVERLAY_PEAK_ALPHA * (1.0 - age_frac);
            let alpha_u8 = (alpha.clamp(0.0, 1.0) * 255.0).round() as u8;
            if alpha_u8 == 0 {
                continue;
            }
            let color = Color::from_rgba8(255, 0, 0, alpha_u8);
            let shape = Rect::new(
                rect[0] as f64,
                rect[1] as f64,
                rect[2] as f64,
                rect[3] as f64,
            );
            master.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                &Brush::Solid(color),
                None,
                &shape,
            );
        }
    }

    /// Borrow the underlying vello::Renderer for advanced uses
    /// (e.g., `register_texture` to convert a wgpu::Texture into a
    /// peniko::ImageData usable as a scene image source). The
    /// resulting ImageData lives until `unregister_texture` is
    /// called or the rasterizer is dropped.
    pub fn vello_renderer_mut(&mut self) -> &mut Renderer {
        &mut self.vello_renderer
    }
}

/// Filter `scene`'s primitives by AABB intersection with `tile_rect`,
/// returning a new `Scene` with only the intersecting ops in their
/// original painter order. Transforms and image_sources are
/// shallow-cloned (cheap for transforms; for large image-source
/// HashMaps this is a known inefficiency, see module docs).
fn filter_scene_to_tile(scene: &Scene, tile_rect: [f32; 4]) -> Scene {
    use crate::tile_cache::{aabb_intersects, world_aabb};

    let mut filtered = Scene::new(scene.viewport_width, scene.viewport_height);
    filtered.transforms = scene.transforms.clone();
    // Fonts are cloned (Arc-shared payload — clone is cheap).
    // Resolved by font_id in emit_glyph_run; the filtered Scene
    // needs the same palette as the source.
    filtered.fonts = scene.fonts.clone();
    // Image cache is supplied by the rasterizer's image_data via
    // overrides at scene_to_vello time, so we can leave
    // image_sources empty here — saves a HashMap clone.
    debug_assert!(filtered.image_sources.is_empty());

    for op in &scene.ops {
        let intersects = match op {
            SceneOp::Rect(rect) => aabb_intersects(
                world_aabb(
                    [rect.x0, rect.y0, rect.x1, rect.y1],
                    rect.transform_id,
                    scene,
                ),
                tile_rect,
            ),
            SceneOp::Gradient(grad) => aabb_intersects(
                world_aabb(
                    [grad.x0, grad.y0, grad.x1, grad.y1],
                    grad.transform_id,
                    scene,
                ),
                tile_rect,
            ),
            SceneOp::Image(image) => aabb_intersects(
                world_aabb(
                    [image.x0, image.y0, image.x1, image.y1],
                    image.transform_id,
                    scene,
                ),
                tile_rect,
            ),
            SceneOp::Pattern(pattern) => aabb_intersects(
                world_aabb(pattern.extent, pattern.transform_id, scene),
                tile_rect,
            ),
            SceneOp::Stroke(stroke) => {
                // Inflate by half stroke width so strokes whose pen
                // reaches a tile aren't filtered out when their path
                // bounds don't.
                let half = stroke.stroke_width * 0.5;
                aabb_intersects(
                    world_aabb(
                        [
                            stroke.x0 - half,
                            stroke.y0 - half,
                            stroke.x1 + half,
                            stroke.y1 + half,
                        ],
                        stroke.transform_id,
                        scene,
                    ),
                    tile_rect,
                )
            }
            SceneOp::Shape(shape) => crate::tile_cache::world_aabb_shape(shape, scene)
                .is_some_and(|aabb| aabb_intersects(aabb, tile_rect)),
            SceneOp::GlyphRun(run) => crate::tile_cache::world_aabb_glyph_run(run, scene)
                .is_some_and(|aabb| aabb_intersects(aabb, tile_rect)),
            // Keep scopes balanced during spatial filtering, then remove
            // only harmless groups with no retained drawing children.
            SceneOp::PushLayer(_) | SceneOp::PopLayer => true,
            // Roadmap E4 — fragment-bearing scenes take the retained
            // master path and never reach the per-tile filter. If one
            // arrives, keep the op (harmless: lowering warns and skips
            // it) so the painter-order stream stays intact.
            SceneOp::Fragment(_) => true,
        };
        if intersects {
            filtered.ops.push(op.clone());
        }
    }

    prune_empty_tile_layers(&mut filtered.ops);
    filtered
}

/// Remove empty normal source-over groups after tile filtering. An empty
/// mask or backdrop/filter group may still modify the parent, so those
/// groups and their enclosing scopes remain intact. Malformed streams are
/// left unchanged for the normal owner validation/lowering path.
fn prune_empty_tile_layers(ops: &mut Vec<SceneOp>) {
    use crate::scene::SceneCompose;

    let mut keep = vec![true; ops.len()];
    let mut stack: Vec<(usize, bool)> = Vec::new();
    for (index, op) in ops.iter().enumerate() {
        match op {
            SceneOp::PushLayer(_) => stack.push((index, false)),
            SceneOp::PopLayer => {
                let Some((start, has_content)) = stack.pop() else {
                    return;
                };
                let SceneOp::PushLayer(layer) = &ops[start] else {
                    unreachable!();
                };
                let harmless = layer.compose == SceneCompose::SrcOver
                    && layer.blend_mode == SceneBlendMode::Normal
                    && layer.backdrop_filter.is_none()
                    && layer.filters.is_empty()
                    && layer.alpha.is_finite()
                    && (0.0..=1.0).contains(&layer.alpha);
                if !has_content && harmless {
                    keep[start] = false;
                    keep[index] = false;
                } else if let Some((_, parent_content)) = stack.last_mut() {
                    *parent_content = true;
                }
            }
            _ => {
                if let Some((_, has_content)) = stack.last_mut() {
                    *has_content = true;
                }
            }
        }
    }
    if !stack.is_empty() {
        return;
    }
    let mut index = 0;
    ops.retain(|_| {
        let retain = keep[index];
        index += 1;
        retain
    });
}

#[cfg(test)]
mod empty_tile_layers_tests {
    use super::*;
    use crate::scene::{SceneClip, SceneFilter, SceneLayer};

    #[test]
    fn spatially_filtered_empty_nested_scopes_are_removed() {
        let mut scene = Scene::new(128, 128);
        scene.push_layer(SceneLayer::alpha(0.5));
        scene.push_layer(SceneLayer::clip(SceneClip::Rect {
            rect: [64.0, 64.0, 128.0, 128.0],
            radii: [4.0; 4],
        }));
        scene.push_rect(70.0, 70.0, 90.0, 90.0, [1.0; 4]);
        scene.pop_layer();
        scene.pop_layer();
        let filtered = filter_scene_to_tile(&scene, [0.0, 0.0, 64.0, 64.0]);
        assert!(filtered.ops.is_empty());
        assert_eq!(scene.ops.len(), 5);
    }

    #[test]
    fn retained_draw_preserves_nested_clip_alpha_and_painter_order() {
        let mut scene = Scene::new(128, 128);
        scene.push_layer(SceneLayer::alpha(0.5));
        scene.push_layer(SceneLayer::clip(SceneClip::Rect {
            rect: [0.0, 0.0, 128.0, 128.0],
            radii: [4.0; 4],
        }));
        scene.push_rect(1.0, 1.0, 9.0, 9.0, [1.0; 4]);
        scene.pop_layer();
        scene.pop_layer();
        let filtered = filter_scene_to_tile(&scene, [0.0, 0.0, 64.0, 64.0]);
        assert_eq!(filtered.ops.len(), 5);
        let SceneOp::PushLayer(layer) = &filtered.ops[0] else {
            panic!()
        };
        assert_eq!(layer.alpha, 0.5);
        assert!(
            matches!(&filtered.ops[1], SceneOp::PushLayer(SceneLayer { clip: SceneClip::Rect { radii, .. }, .. }) if *radii == [4.0; 4])
        );
        assert!(matches!(filtered.ops[2], SceneOp::Rect(_)));
    }

    #[test]
    fn empty_masks_filters_backdrops_and_blends_keep_parent_scopes() {
        let mut effects = vec![SceneLayer::alpha_mask()];
        let mut backdrop = SceneLayer::alpha(1.0);
        backdrop.backdrop_filter = Some(SceneFilter::Blur(2.0));
        effects.push(backdrop);
        let mut filtered = SceneLayer::alpha(1.0);
        filtered.filters.push(SceneFilter::Invert(1.0));
        effects.push(filtered);
        let mut blend = SceneLayer::alpha(1.0);
        blend.blend_mode = SceneBlendMode::Multiply;
        effects.push(blend);
        for effect in effects {
            let mut ops = vec![
                SceneOp::PushLayer(SceneLayer::alpha(0.5)),
                SceneOp::PushLayer(effect),
                SceneOp::PopLayer,
                SceneOp::PopLayer,
            ];
            prune_empty_tile_layers(&mut ops);
            assert_eq!(ops.len(), 4);
        }
    }

    #[test]
    fn malformed_scopes_are_not_rewritten() {
        for mut ops in [
            vec![SceneOp::PushLayer(SceneLayer::alpha(1.0))],
            vec![
                SceneOp::PushLayer(SceneLayer::alpha(1.0)),
                SceneOp::PopLayer,
                SceneOp::PopLayer,
            ],
        ] {
            let original_len = ops.len();
            prune_empty_tile_layers(&mut ops);
            assert_eq!(ops.len(), original_len);
        }
    }
}
