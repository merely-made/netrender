// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Phases 2' / 5' / 8' — netrender Scene → vello::Scene translator.
//!
//! Phase 2': rects with per-primitive transform and axis-aligned
//! clip. Phase 5': image ingestion with per-image transform, clip,
//! UV sub-region, and alpha tint. Phase 8': linear / circular-radial
//! / conic gradients with N-stop ramps. Output is suitable for
//! `Renderer::render_to_texture`; receipts are at
//! `tests/p2prime_vello_rects.rs`, `tests/p5prime_vello_image.rs`,
//! and `tests/p8prime_vello_gradients.rs`.
//!
//! ## Image-tint encoding (Phase 5a + 5b)
//!
//! `SceneImage.color` is a premultiplied RGBA tint, decomposed into
//! `alpha_factor = a` and `chromatic_factor = (r/a, g/a, b/a)`:
//!
//! - **Phase 5a — alpha factor.** Applied via
//!   `ImageBrush::with_alpha(a)`. Sufficient for achromatic tints
//!   (white-with-alpha, the tile-cache composite case).
//! - **Phase 5b — chromatic factor.** When `chromatic_factor` is
//!   not `(1, 1, 1)`, paint the alpha-modulated image and then
//!   apply a `BlendMode::new(Mix::Multiply, Compose::SrcAtop)`
//!   layer that fills the image rect with the chromatic factor as
//!   a solid color (alpha 1.0). `SrcAtop` constrains the multiply
//!   to where the image already painted, so transparent regions of
//!   the image stay transparent. Used by 9A's mask-as-tinted-image
//!   case and any drop-shadow style with a colored shadow.
//!
//! ## Boundary conventions (verified Phase 1' p1prime_02 / p1prime_03)
//!
//! - `SceneRect.color` is **premultiplied** RGBA. `peniko::Color`
//!   expects **straight-alpha**. We unpremultiply at the boundary:
//!   `(r/a, g/a, b/a, a)` for `a > 0`, `(0, 0, 0, 0)` for `a == 0`.
//! - Vello stores straight-alpha sRGB-encoded values in its output
//!   target. The compositor (downstream sample stage) is responsible
//!   for premultiplying after the hardware sRGB→linear decode; that
//!   contract is unchanged from §6.1.
//! - `interpolation_cs` is not threaded through gradients (no-op on
//!   the GPU compute path; see §3.3 / p1prime_03).
//!
//! ## Coordinate conventions
//!
//! `Transform.m` is a column-major 4×4 with the 2D affine in
//! `(m[0], m[1], m[4], m[5], m[12], m[13])` = `(a, b, c, d, e, f)`,
//! matching `kurbo::Affine::new([a, b, c, d, e, f])`.

use std::{borrow::Cow, collections::HashMap};

use vello::kurbo::{Affine, Rect, RoundedRect, RoundedRectRadii};
use vello::peniko::{self, Color, Fill, ImageAlphaType, ImageData, ImageFormat};

use crate::scene::{ImageKey, Scene, SceneOp, Transform};

mod emit;
mod emit_paint;
pub(crate) use emit::{build_bez_path, emit_push_layer};
use emit::{emit_glyph_run, emit_rect, emit_shape, emit_stroke};
use emit_paint::{emit_gradient, emit_image, emit_pattern};
/// Phase 2' / 5' scope: rects + images, with per-primitive transform
/// and clip. Gradients in `scene` are silently ignored (Phase 8').
/// Painter order matches the parent scene: rects first, then images
/// painted over them — the same ordering the existing netrender
/// pipeline uses.
pub fn scene_to_vello(scene: &Scene) -> vello::Scene {
    scene_to_vello_with_overrides(scene, &HashMap::new())
}

/// Translate a netrender [`Scene`] into a [`vello::Scene`] with
/// caller-supplied [`peniko::ImageData`] overrides for selected
/// [`ImageKey`]s.
///
/// `image_overrides` lets callers pre-register GPU-resident textures
/// via [`vello::Renderer::register_texture`] (Path B from rasterizer
/// plan §3.5) and pass the resulting [`ImageData`] in. Keys absent
/// from the overrides map fall back to building from
/// `scene.image_sources` CPU bytes (Path A — the default).
///
/// Use this entry point when image data lives as a render-graph
/// output (already a `wgpu::Texture`, no CPU bytes), e.g., the blur
/// task's output texture feeding into a vello-rasterized scene.
pub fn scene_to_vello_with_overrides(
    scene: &Scene,
    image_overrides: &HashMap<ImageKey, ImageData>,
) -> vello::Scene {
    let empty = HashMap::new();
    let images = ImageLookup::new(&empty, &scene.image_sources, image_overrides);
    scene_to_vello_with_lookup(scene, &images)
}

/// Translate a [`Scene`] into a [`vello::Scene`] using a
/// caller-supplied pre-built image cache. Keys missing from this map
/// are treated as missing, matching the original cache-only contract.
pub fn scene_to_vello_with_cache(
    scene: &Scene,
    images: &HashMap<ImageKey, ImageData>,
) -> vello::Scene {
    let empty_sources = HashMap::new();
    let empty_overrides = HashMap::new();
    let images = ImageLookup::new(images, &empty_sources, &empty_overrides);
    scene_to_vello_with_lookup(scene, &images)
}

pub(crate) fn scene_to_vello_with_image_sources(
    scene: &Scene,
    cached: &HashMap<ImageKey, ImageData>,
    overrides: &HashMap<ImageKey, ImageData>,
) -> vello::Scene {
    let view = SceneView::from_scene(scene);
    scene_to_vello_with_view(&view, scene.ops.iter(), cached, overrides)
}

/// Borrowed lowering inputs. Tiled and retained paths can select an op
/// stream without copying the scene's palettes or the operations themselves.
pub(crate) struct SceneView<'a> {
    pub(crate) viewport_width: u32,
    pub(crate) viewport_height: u32,
    pub(crate) transforms: Cow<'a, [Transform]>,
    pub(crate) fonts: Cow<'a, [crate::scene::FontBlob]>,
    pub(crate) image_sources: &'a HashMap<ImageKey, crate::scene::ImageData>,
}

impl<'a> SceneView<'a> {
    pub(crate) fn from_scene(scene: &'a Scene) -> Self {
        Self {
            viewport_width: scene.viewport_width,
            viewport_height: scene.viewport_height,
            transforms: Cow::Borrowed(&scene.transforms),
            fonts: Cow::Borrowed(&scene.fonts),
            image_sources: &scene.image_sources,
        }
    }

    pub(crate) fn from_fragment(fragment: &'a crate::scene::SceneFragment) -> Self {
        // `append_fragment` has always discarded the fragment's local slot 0
        // and kept the destination Scene's identity transform and sentinel
        // font. Preserve that contract without copying valid palettes.
        let transforms = if fragment
            .transforms
            .first()
            .is_some_and(|t| t.m == Transform::IDENTITY.m)
        {
            Cow::Borrowed(fragment.transforms.as_slice())
        } else {
            let mut normalized = Vec::with_capacity(fragment.transforms.len().max(1));
            normalized.push(Transform::IDENTITY);
            normalized.extend(fragment.transforms.iter().skip(1).copied());
            Cow::Owned(normalized)
        };
        let fonts = if fragment
            .fonts
            .first()
            .is_some_and(crate::scene::is_sentinel_font)
        {
            Cow::Borrowed(fragment.fonts.as_slice())
        } else {
            let sentinel = Scene::new(1, 1).fonts[0].clone();
            let mut normalized = Vec::with_capacity(fragment.fonts.len().max(1));
            normalized.push(sentinel);
            normalized.extend(fragment.fonts.iter().skip(1).cloned());
            Cow::Owned(normalized)
        };
        Self {
            // Fragment lowering historically used a nominal 1×1 viewport.
            viewport_width: 1,
            viewport_height: 1,
            transforms,
            fonts,
            image_sources: &fragment.image_sources,
        }
    }
}

pub(crate) fn scene_to_vello_with_view<'a>(
    view: &SceneView<'a>,
    ops: impl IntoIterator<Item = &'a SceneOp>,
    cached: &HashMap<ImageKey, ImageData>,
    overrides: &HashMap<ImageKey, ImageData>,
) -> vello::Scene {
    scene_to_vello_with_inputs(view, ops, cached, view.image_sources, overrides)
}

fn scene_to_vello_with_inputs<'a>(
    view: &SceneView<'a>,
    ops: impl IntoIterator<Item = &'a SceneOp>,
    cached: &HashMap<ImageKey, ImageData>,
    sources: &'a HashMap<ImageKey, crate::scene::ImageData>,
    overrides: &HashMap<ImageKey, ImageData>,
) -> vello::Scene {
    let images = ImageLookup::new(cached, sources, overrides);
    let mut vscene = vello::Scene::new();

    // Single pass over the unified op list — painter order = consumer
    // push order. (Pre-2026-05-04 op-list refactor this dispatched
    // through six per-type Vec passes with a fixed cross-type order;
    // see plan §11.11 for context.)
    // Layer-balance counter so debug builds catch unbalanced
    // PushLayer/PopLayer pairs at scene-translation time. In release
    // an unbalanced PopLayer with no live layer is silently skipped
    // (vello would panic on underflow).
    let mut layer_depth: u32 = 0;
    // Missing-source image/pattern keys, aggregated so a systemic drop (e.g. unbuilt
    // box-shadow masks) reports one warn per rasterize instead of a per-op flood —
    // the difference between a signal and thousands of noise lines. (Diagnostics.)
    let mut missing_images: Vec<ImageKey> = Vec::new();
    for op in ops {
        match op {
            SceneOp::Rect(rect) => emit_rect(&mut vscene, rect, view.transforms.as_ref()),
            SceneOp::Stroke(stroke) => emit_stroke(&mut vscene, stroke, view.transforms.as_ref()),
            SceneOp::Gradient(gradient) => {
                emit_gradient(&mut vscene, gradient, view.transforms.as_ref())
            }
            SceneOp::Image(image) => {
                if let Some(key) = emit_image(&mut vscene, image, view.transforms.as_ref(), &images)
                {
                    missing_images.push(key);
                }
            }
            SceneOp::Pattern(pattern) => {
                if let Some(key) =
                    emit_pattern(&mut vscene, pattern, view.transforms.as_ref(), &images)
                {
                    missing_images.push(key);
                }
            }
            SceneOp::Shape(shape) => emit_shape(&mut vscene, shape, view.transforms.as_ref()),
            SceneOp::GlyphRun(run) => emit_glyph_run(
                &mut vscene,
                run,
                view.fonts.as_ref(),
                view.transforms.as_ref(),
            ),
            SceneOp::PushLayer(layer) => {
                emit_push_layer(
                    &mut vscene,
                    layer,
                    view.transforms.as_ref(),
                    view.viewport_width,
                    view.viewport_height,
                );
                layer_depth += 1;
            }
            SceneOp::PopLayer => {
                debug_assert!(
                    layer_depth > 0,
                    "SceneOp::PopLayer with no matching PushLayer"
                );
                if layer_depth > 0 {
                    vscene.pop_layer();
                    layer_depth -= 1;
                }
            }
            // Roadmap E4 — placed fragments resolve through the
            // renderer's registry, which this free translator cannot
            // see. The retained-fragment master path expands them
            // before lowering ever runs; reaching here means a
            // consumer handed a fragment-bearing scene to the simple
            // path, where the op paints nothing.
            SceneOp::Fragment(f) => {
                log::warn!(
                    "scene_to_vello: SceneOp::Fragment(id={}) has no registry here; skipped \
                     (use the tile rasterizer path for retained fragments)",
                    f.id
                );
            }
        }
    }
    if !missing_images.is_empty() {
        // One report per rasterize, deduped. Keys >= 0xFFFF_0000_0000_0000
        // (`BOX_SHADOW_MASK_KEY_BASE`) are box-shadow masks the host must build via
        // `build_box_shadow_mask` before rasterizing; lower keys are content images
        // the paint translator emitted without registering a source. Either way the
        // op is skipped (paints nothing / black), so a divergence is a real bug.
        let unique: std::collections::HashSet<_> = missing_images.iter().copied().collect();
        log::warn!(
            "scene_to_vello: {} image/pattern op(s) referenced {} unregistered image key(s) \
             and were skipped (up to {} image lookup entries were available). Keys >= {:#x} are unbuilt box-shadow \
             masks; lower keys are content images with no registered source. Missing: {:?}",
            missing_images.len(),
            unique.len(),
            images.source_count_upper_bound(),
            0xFFFF_0000_0000_0000u64,
            unique,
        );
    }
    debug_assert_eq!(
        layer_depth, 0,
        "Scene ended with {} unclosed PushLayer(s)",
        layer_depth,
    );

    vscene
}

fn scene_to_vello_with_lookup(scene: &Scene, images: &ImageLookup<'_>) -> vello::Scene {
    let view = SceneView::from_scene(scene);
    scene_to_vello_with_inputs(
        &view,
        scene.ops.iter(),
        images.cached,
        images.sources,
        images.overrides,
    )
}

/// Stateful simple-path translator. Shares CPU-source identity retention with
/// the tiled renderer, keeping Blob identities stable when producers rebuild
/// their scenes. Source retirement happens on refresh; overrides have their
/// own explicit registration lifetime. Lowering borrows both maps directly.
pub struct VelloRasterizer {
    image_data: CpuImageCache,
    image_overrides: HashMap<ImageKey, ImageData>,
}

impl VelloRasterizer {
    /// Construct an empty rasterizer. Cache fills on the first
    /// `scene_to_vello` call.
    pub fn new() -> Self {
        Self {
            image_data: CpuImageCache::default(),
            image_overrides: HashMap::new(),
        }
    }

    /// Number of CPU-side `ImageData` entries currently held in the
    /// cache (one per `ImageKey` present in the most recent
    /// scene's `image_sources`). Stable across consecutive
    /// `scene_to_vello` calls on the same scene; updates as the
    /// consumer adds or removes image sources.
    pub fn cached_image_count(&self) -> usize {
        self.image_data.len()
    }

    /// Register a Path B [`peniko::ImageData`] (typically the
    /// result of `vello::Renderer::register_texture`) under the
    /// given key. Path B overrides win over `scene.image_sources`
    /// on key collision.
    pub fn register_texture(&mut self, key: ImageKey, image: ImageData) {
        self.image_overrides.insert(key, image);
    }

    /// Drop a previously-registered Path B entry. Returns the
    /// dropped value if present, `None` otherwise.
    pub fn unregister_texture(&mut self, key: ImageKey) -> Option<ImageData> {
        self.image_overrides.remove(&key)
    }

    /// Translate `scene` into a fresh [`vello::Scene`], using and
    /// updating the cached image map.
    ///
    /// Cache invariant: after this call, `self.image_data` contains
    /// exactly the keys present in `scene.image_sources` (unchanged
    /// keys keep their existing `ImageData`; new keys are inserted;
    /// keys absent from the scene are evicted).
    pub fn scene_to_vello(&mut self, scene: &Scene) -> vello::Scene {
        self.image_data.refresh(scene);
        scene_to_vello_with_image_sources(scene, self.image_data.entries(), &self.image_overrides)
    }
}

impl Default for VelloRasterizer {
    fn default() -> Self {
        Self::new()
    }
}

/// Owns the retained CPU-source identities for both Classic rasterizers.
/// Keys identify immutable pixels until absent from a refreshed scene.
#[derive(Default)]
pub(crate) struct CpuImageCache {
    entries: HashMap<ImageKey, ImageData>,
}

impl CpuImageCache {
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }
    pub(crate) fn entries(&self) -> &HashMap<ImageKey, ImageData> {
        &self.entries
    }
    pub(crate) fn refresh(&mut self, scene: &Scene) {
        for (key, data) in &scene.image_sources {
            self.entries
                .entry(*key)
                .or_insert_with(|| convert_image_source(data));
            debug_assert_eq!(
                (
                    self.entries[key].width,
                    self.entries[key].height,
                    self.entries[key].data.len()
                ),
                (data.width, data.height, data.data.len()),
                "ImageKey {key:#x} reused with different dimensions or byte length"
            );
        }
        self.entries
            .retain(|key, _| scene.image_sources.contains_key(key));
    }
}

pub(crate) struct ImageLookup<'a> {
    cached: &'a HashMap<ImageKey, ImageData>,
    sources: &'a HashMap<ImageKey, crate::scene::ImageData>,
    overrides: &'a HashMap<ImageKey, ImageData>,
}
impl<'a> ImageLookup<'a> {
    fn new(
        cached: &'a HashMap<ImageKey, ImageData>,
        sources: &'a HashMap<ImageKey, crate::scene::ImageData>,
        overrides: &'a HashMap<ImageKey, ImageData>,
    ) -> Self {
        Self {
            cached,
            sources,
            overrides,
        }
    }
    pub(crate) fn get(&self, key: &ImageKey) -> Option<Cow<'_, ImageData>> {
        if let Some(image) = self.overrides.get(key).or_else(|| self.cached.get(key)) {
            return Some(Cow::Borrowed(image));
        }
        self.sources
            .get(key)
            .map(|data| Cow::Owned(convert_image_source(data)))
    }
    fn source_count_upper_bound(&self) -> usize {
        self.sources.len() + self.cached.len() + self.overrides.len()
    }
}

// Keep the scene-to-peniko format policy identical for retained and local sources.
fn convert_image_source(data: &crate::scene::ImageData) -> ImageData {
    ImageData {
        data: data.data.clone(),
        format: ImageFormat::Rgba8,
        alpha_type: ImageAlphaType::Alpha,
        width: data.width,
        height: data.height,
    }
}

/// Push a clip layer for the given clip rect + per-corner radii.
/// Zero radii produce a sharp axis-aligned rect clip (legacy behavior);
/// non-zero radii produce a `kurbo::RoundedRect` clip (Phase 9'). The
/// caller is responsible for matching this with `vscene.pop_layer()`.
fn push_clip_layer(vscene: &mut vello::Scene, clip_rect: [f32; 4], radii: [f32; 4]) {
    let rect = Rect::new(
        clip_rect[0] as f64,
        clip_rect[1] as f64,
        clip_rect[2] as f64,
        clip_rect[3] as f64,
    );
    let any_radius = radii.iter().any(|&r| r > 0.0);
    if any_radius {
        // RoundedRectRadii::new takes (top_leading, top_trailing,
        // bottom_trailing, bottom_leading) which under our Y-down screen
        // coordinates maps to (top_left, top_right, bottom_right,
        // bottom_left) — the same order our SceneRect.clip_corner_radii
        // documents.
        let rrect = RoundedRect::from_rect(
            rect,
            RoundedRectRadii::new(
                radii[0] as f64,
                radii[1] as f64,
                radii[2] as f64,
                radii[3] as f64,
            ),
        );
        vscene.push_layer(
            Fill::NonZero,
            peniko::Mix::Normal,
            1.0,
            Affine::IDENTITY,
            &rrect,
        );
    } else {
        vscene.push_layer(
            Fill::NonZero,
            peniko::Mix::Normal,
            1.0,
            Affine::IDENTITY,
            &rect,
        );
    }
}

pub(crate) fn transform_to_affine(t: &Transform) -> Affine {
    Affine::new([
        t.m[0] as f64,
        t.m[1] as f64,
        t.m[4] as f64,
        t.m[5] as f64,
        t.m[12] as f64,
        t.m[13] as f64,
    ])
}

fn unpremultiply_color(c: [f32; 4]) -> Color {
    let a = c[3];
    if a > 0.0 {
        Color::from_rgba8(
            (c[0] / a * 255.0).round().clamp(0.0, 255.0) as u8,
            (c[1] / a * 255.0).round().clamp(0.0, 255.0) as u8,
            (c[2] / a * 255.0).round().clamp(0.0, 255.0) as u8,
            (a * 255.0).round().clamp(0.0, 255.0) as u8,
        )
    } else {
        Color::from_rgba8(0, 0, 0, 0)
    }
}

#[cfg(test)]
mod borrowed_view_tests {
    use super::*;
    use crate::scene::SceneFragment;

    #[test]
    fn valid_fragment_palettes_are_borrowed_and_local_ids_stay_local() {
        let mut fragment = SceneFragment::new();
        fragment.push_transform(Transform::translate_2d(7.0, 11.0));
        let view = SceneView::from_fragment(&fragment);

        assert!(std::ptr::eq(
            view.transforms.as_ptr(),
            fragment.transforms.as_ptr()
        ));
        assert!(std::ptr::eq(view.fonts.as_ptr(), fragment.fonts.as_ptr()));
        assert_eq!(view.transforms[1].m, Transform::translate_2d(7.0, 11.0).m);
        assert_eq!((view.viewport_width, view.viewport_height), (1, 1));
    }

    #[test]
    fn malformed_fragment_slot_zero_keeps_append_fragment_defaults() {
        let mut fragment = SceneFragment::new();
        fragment.transforms[0] = Transform::translate_2d(70.0, 90.0);
        fragment.fonts.clear();

        let view = SceneView::from_fragment(&fragment);
        assert_eq!(view.transforms[0].m, Transform::IDENTITY.m);
        assert_eq!(view.fonts[0].index, 0);
        assert!(crate::scene::is_sentinel_font(&view.fonts[0]));
    }
}
