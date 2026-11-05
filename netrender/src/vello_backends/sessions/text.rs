// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Validated outline text using the pinned Glifo renderer, with a preparation
//! cache owned here so lifetime limits can actually release retained outlines.

use super::{SparseSessionError, budget, validate};
use crate::scene::{FontBlob, NO_CLIP, Scene, SceneGlyphRun, SceneOp};
use skrifa::{
    FontRef, GlyphId, MetadataProvider,
    instance::Size,
    outline::{DrawSettings, OutlinePen},
    raw::TableProvider,
};
use std::{
    collections::{HashMap, HashSet},
    ops::RangeInclusive,
    sync::Arc,
};
use vello_sparse_common::{
    kurbo::{Affine, Point},
    peniko::FontData,
};
use vello_sparse_text::{
    AtlasCacher, GlyphPrepCache, GlyphRenderError, GlyphRenderer, GlyphRun, GlyphRunBackend,
    GlyphRunBuilder,
};

/// Limits for unhinted, solid-color outline text. Font bytes and outline
/// segments are conservative logical residency bounds, not allocator bytes.
/// Fonts with color/bitmap/SVG tables are outside this first text subset.
/// Font assets are trusted inputs. Validation catches malformed requested
/// outlines before upstream unchecked drawing, but is not a font sanitizer or
/// parser CPU-time sandbox; parser/scratch allocations are excluded.
#[derive(Debug, Clone, Copy)]
pub struct SparseTextLimits {
    pub max_fonts: usize,
    pub max_font_bytes: usize,
    /// Total positioned glyphs in one frame, including repeated glyphs.
    pub max_glyphs: usize,
    pub max_cached_glyphs: usize,
    /// Outline segments emitted by a frame, counting repeated positioned glyphs.
    pub max_outline_segments: usize,
    pub max_cached_outline_segments: usize,
    /// Bounds normalized-coordinate storage per cached glyph variant.
    pub max_variation_axes: usize,
    pub max_font_size: f32,
    /// End the cache epoch after this many admitted frames. Must be nonzero
    /// and smaller than u32::MAX, also bounding upstream serial counters.
    pub max_cache_frames: u32,
}
impl Default for SparseTextLimits {
    fn default() -> Self {
        Self {
            max_fonts: 32,
            max_font_bytes: 64 * 1024 * 1024,
            max_glyphs: 16_384,
            max_cached_glyphs: 4096,
            max_outline_segments: 1_000_000,
            max_cached_outline_segments: 2_000_000,
            max_variation_axes: 64,
            max_font_size: 1024.0,
            max_cache_frames: 4096,
        }
    }
}

/// Conservative epoch accounting. Upstream may evict outlines sooner; its
/// freed path buffers remain covered until the whole preparation cache clears.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SparseTextStats {
    pub font_count: usize,
    /// Shared font bytes are conservatively counted once per collection face.
    pub font_bytes: usize,
    pub cached_glyphs: usize,
    pub cached_outline_segments: usize,
    /// Cumulative newly validated outlines, not glyph-atlas uploads.
    pub prepared_glyphs: u64,
    /// Cumulative admitted positioned glyphs not drawn because their validated
    /// outline bounds cannot reach the target or run clip.
    pub culled_glyphs: u64,
    pub cache_resets: u64,
    pub cache_frames: u32,
}

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
struct FontKey {
    blob: u64,
    index: u32,
}
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
struct OutlineKey {
    font: FontKey,
    glyph: u32,
    coords: Vec<i16>,
}
#[derive(Debug, Clone)]
struct OutlineRecord {
    segments: usize,
    bounds: Option<[f32; 4]>,
}

struct AdmittedFace<'a> {
    font: FontRef<'a>,
    upem: u16,
    glyph_count: u32,
}
#[cfg(test)]
thread_local! { static ADMISSION_CALLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }
#[cfg(test)]
thread_local! { static CULL_DISABLED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) }; }
fn admit_face<'a>(
    blob: &'a FontBlob,
    limits: SparseTextLimits,
    op_index: usize,
    font_id: u32,
) -> Result<AdmittedFace<'a>, SparseSessionError> {
    #[cfg(test)]
    ADMISSION_CALLS.with(|calls| calls.set(calls.get() + 1));
    let font_error = |reason| SparseSessionError::InvalidFont {
        op_index,
        font_id,
        reason,
    };
    let font = FontRef::from_index(blob.data.as_ref(), blob.index)
        .map_err(|_| font_error("invalid font bytes or collection index"))?;
    let upem = font
        .head()
        .map_err(|_| font_error("missing or malformed head table"))?
        .units_per_em();
    if !(16..=16_384).contains(&upem) {
        return Err(font_error(
            "units per em must be within the OpenType range 16..16384",
        ));
    }
    let glyph_count = font
        .maxp()
        .map_err(|_| font_error("missing or malformed maxp table"))?
        .num_glyphs() as u32;
    for tag in [
        *b"COLR", *b"CPAL", *b"CBDT", *b"CBLC", *b"EBDT", *b"EBLC", *b"sbix", *b"SVG ", *b"bdat",
        *b"bloc",
    ] {
        if font.table_data(skrifa::Tag::new(&tag)).is_some() {
            return Err(font_error(
                "color, bitmap and SVG font tables are not admitted by outline text sessions",
            ));
        }
    }
    budget(
        "font variation axes",
        font.axes().len(),
        limits.max_variation_axes,
    )?;
    if font.outline_glyphs().format().is_none() {
        return Err(font_error("font has no supported outlines"));
    }
    Ok(AdmittedFace {
        font,
        upem,
        glyph_count,
    })
}

pub(super) struct PreparedRun {
    pub font: FontData,
    pub coords: Vec<i16>,
    /// Indices of glyphs whose validated outline bounds may touch the target
    /// inside the run clip. `None` draws every glyph. Culling only narrows
    /// drawing; every glyph has already passed admission and budget checks.
    pub visible: Option<Vec<u32>>,
}

/// Device-space rectangle that can receive a run's pixels: the viewport,
/// narrowed by the run's own clip. Sparse sessions refuse filters, so no
/// admitted operation samples pixels outside the target.
fn cull_rect(scene: &Scene, run: &SceneGlyphRun) -> [f64; 4] {
    let mut rect = [
        0.0,
        0.0,
        scene.viewport_width as f64,
        scene.viewport_height as f64,
    ];
    if run.clip_rect != NO_CLIP {
        rect[0] = rect[0].max(run.clip_rect[0] as f64);
        rect[1] = rect[1].max(run.clip_rect[1] as f64);
        rect[2] = rect[2].min(run.clip_rect[2] as f64);
        rect[3] = rect[3].min(run.clip_rect[3] as f64);
    }
    rect
}

/// Whether control-point bounds (in font units) mapped by `transform` may
/// touch `rect`. Control points contain the filled outline. The margin keeps
/// antialiasing across the edge plus a relative allowance for the renderer
/// recomputing the transform in f32 at a scaled font size.
fn may_touch(transform: Affine, bounds: Option<[f32; 4]>, rect: [f64; 4]) -> bool {
    let Some(b) = bounds else {
        // No control points: the outline is empty and paints nothing.
        return false;
    };
    let mut extent = [
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ];
    for (x, y) in [(b[0], b[1]), (b[0], b[3]), (b[2], b[1]), (b[2], b[3])] {
        let p = transform * Point::new(x as f64, y as f64);
        extent[0] = extent[0].min(p.x);
        extent[1] = extent[1].min(p.y);
        extent[2] = extent[2].max(p.x);
        extent[3] = extent[3].max(p.y);
    }
    let margin = 2.0 + extent.iter().fold(0.0_f64, |m, v| m.max(v.abs())) * 1e-6;
    !(extent[2] + margin < rect[0]
        || extent[0] - margin > rect[2]
        || extent[3] + margin < rect[1]
        || extent[1] - margin > rect[3])
}
pub(super) struct TextPlan {
    pub runs: HashMap<usize, PreparedRun>,
    fonts: HashMap<FontKey, FontBlob>,
    outlines: HashMap<OutlineKey, OutlineRecord>,
    reset: bool,
    prepared: u64,
    culled: u64,
}

pub(super) struct TextState {
    limits: SparseTextLimits,
    fonts: HashMap<FontKey, FontBlob>,
    outlines: HashMap<OutlineKey, OutlineRecord>,
    cache: GlyphPrepCache,
    frames: u32,
    prepared: u64,
    culled: u64,
    resets: u64,
}
impl TextState {
    pub fn new(limits: SparseTextLimits) -> Self {
        Self {
            limits,
            fonts: HashMap::new(),
            outlines: HashMap::new(),
            cache: GlyphPrepCache::default(),
            frames: 0,
            prepared: 0,
            culled: 0,
            resets: 0,
        }
    }
    pub fn stats(&self) -> SparseTextStats {
        SparseTextStats {
            font_count: self.fonts.len(),
            font_bytes: font_bytes(&self.fonts),
            cached_glyphs: self.outlines.len(),
            cached_outline_segments: segments(&self.outlines),
            prepared_glyphs: self.prepared,
            culled_glyphs: self.culled,
            cache_resets: self.resets,
            cache_frames: self.frames,
        }
    }
    pub fn clear(&mut self) {
        self.cache = GlyphPrepCache::default();
        self.fonts = HashMap::new();
        self.outlines = HashMap::new();
        self.frames = 0;
        self.resets = self.resets.saturating_add(1);
    }
    pub fn plan(&self, scene: &Scene) -> Result<TextPlan, SparseSessionError> {
        let limits = self.limits;
        if !limits.max_font_size.is_finite()
            || limits.max_font_size <= 0.0
            || limits.max_cache_frames == 0
            || limits.max_cache_frames == u32::MAX
        {
            return Err(SparseSessionError::InvalidScene {
                op_index: None,
                reason: "invalid text size/cache epoch limit",
            });
        }
        let mut fonts: HashMap<FontKey, FontBlob> = HashMap::new();
        // All entries live only for this plan. Pointer keys identify immutable
        // Arc allocations that remain owned by the scene or retained state.
        // Blob IDs alone never authorize reuse of face admission or bytes.
        let mut identities = HashMap::new();
        for prior in self.fonts.values() {
            identities
                .entry(prior.data.id())
                .or_insert_with(|| prior.data.clone().into_raw_parts().0);
        }
        let mut checked_backings = HashSet::new();
        let mut faces = HashMap::new();
        let mut outlines = HashMap::new();
        let mut runs = HashMap::new();
        let mut glyphs = 0usize;
        let mut frame_segments = 0usize;
        let mut prepared = 0u64;
        let mut culled = 0u64;
        for (op_index, op) in scene.ops.iter().enumerate() {
            let SceneOp::GlyphRun(run) = op else {
                continue;
            };
            let font_error = |reason| SparseSessionError::InvalidFont {
                op_index,
                font_id: run.font_id,
                reason,
            };
            let glyph_error = |glyph_id, reason| SparseSessionError::InvalidGlyph {
                op_index,
                glyph_id,
                reason,
            };
            if run.font_size > limits.max_font_size {
                return Err(SparseSessionError::InvalidScene {
                    op_index: Some(op_index),
                    reason: "font size exceeds text session limit",
                });
            }
            glyphs = glyphs
                .checked_add(run.glyphs.len())
                .ok_or_else(|| font_error("glyph count overflow"))?;
            budget("positioned glyphs", glyphs, limits.max_glyphs)?;
            let blob = scene
                .fonts
                .get(run.font_id as usize)
                .filter(|_| run.font_id != 0)
                .ok_or_else(|| font_error("missing font palette entry"))?;
            let key = FontKey {
                blob: blob.data.id(),
                index: blob.index,
            };
            let backing = blob.data.clone().into_raw_parts().0;
            let allocation = Arc::as_ptr(&backing) as *const () as usize;
            if checked_backings.insert((key.blob, allocation)) {
                if let Some(prior) = identities.get(&key.blob) {
                    if !Arc::ptr_eq(prior, &backing)
                        && prior.as_ref().as_ref() != backing.as_ref().as_ref()
                    {
                        return Err(font_error("font blob identity reused with different bytes"));
                    }
                } else {
                    identities.insert(key.blob, backing);
                }
            }
            if let std::collections::hash_map::Entry::Vacant(entry) = fonts.entry(key) {
                entry.insert(blob.clone());
                budget("text font count", fonts.len(), limits.max_fonts)?;
                budget("text font bytes", font_bytes(&fonts), limits.max_font_bytes)?;
            }
            let face_key = (allocation, blob.index);
            if let std::collections::hash_map::Entry::Vacant(entry) = faces.entry(face_key) {
                entry.insert(admit_face(blob, limits, op_index, run.font_id)?);
            }
            let face = &faces[&face_key];
            let font = &face.font;
            let upem = face.upem;
            let glyph_count = face.glyph_count;
            let settings: Vec<(&str, f32)> = run
                .font_axis_values
                .iter()
                .map(|(tag, value)| {
                    (
                        std::str::from_utf8(tag)
                            .expect("axis tags validated before resource preflight"),
                        *value,
                    )
                })
                .collect();
            let axes = font.axes();
            let location = axes.location(settings);
            let coords: Vec<i16> = location.coords().iter().map(|v| v.to_bits()).collect();
            let collection = font.outline_glyphs();
            let m = &scene.transforms[run.transform_id as usize].m;
            let world = Affine::new([
                m[0] as f64,
                m[1] as f64,
                m[4] as f64,
                m[5] as f64,
                m[12] as f64,
                m[13] as f64,
            ]);
            // Glifo absorbs positive uniform scale into an f32 font size.
            // Checking all linear components is conservative for other modes.
            if m[..2]
                .iter()
                .chain(&m[4..6])
                .any(|scale| !(run.font_size * *scale).is_finite())
            {
                return Err(font_error(
                    "transformed font size exceeds sparse precision/range",
                ));
            }
            let rect = cull_rect(scene, run);
            let mut visible: Option<Vec<u32>> = None;
            for (glyph_index, glyph) in run.glyphs.iter().enumerate() {
                if glyph.id >= glyph_count {
                    return Err(glyph_error(glyph.id, "glyph id is outside the font"));
                }
                let outline_key = OutlineKey {
                    font: key,
                    glyph: glyph.id,
                    coords: coords.clone(),
                };
                if !outlines.contains_key(&outline_key) {
                    let record = if let Some(record) = self.outlines.get(&outline_key) {
                        record.clone()
                    } else {
                        let outline = collection.get(GlyphId::new(glyph.id)).ok_or_else(|| {
                            glyph_error(glyph.id, "missing or malformed glyph outline")
                        })?;
                        let mut pen = CountingPen::default();
                        draw_outline(
                            &outline,
                            DrawSettings::unhinted(Size::new(upem as f32), location.coords()),
                            &mut pen,
                        )
                        .map_err(|_| glyph_error(glyph.id, "glyph outline failed validation"))?;
                        if !pen.finite {
                            return Err(glyph_error(glyph.id, "non-finite outline coordinates"));
                        }
                        prepared = prepared.saturating_add(1);
                        OutlineRecord {
                            segments: pen.segments,
                            bounds: pen.bounds,
                        }
                    };
                    outlines.insert(outline_key.clone(), record);
                    budget(
                        "text glyph variants",
                        outlines.len(),
                        limits.max_cached_glyphs,
                    )?;
                    budget(
                        "text cached outline segments",
                        segments(&outlines),
                        limits.max_cached_outline_segments,
                    )?;
                }
                frame_segments = frame_segments
                    .checked_add(outlines[&outline_key].segments)
                    .ok_or_else(|| glyph_error(glyph.id, "frame outline segment count overflow"))?;
                budget(
                    "text frame outline segments",
                    frame_segments,
                    limits.max_outline_segments,
                )?;
                let transform = world
                    * Affine::translate((glyph.x as f64, glyph.y as f64))
                    * Affine::scale_non_uniform(
                        run.font_size as f64 / upem as f64,
                        -(run.font_size as f64 / upem as f64),
                    );
                if !validate::safe_affine(transform)
                    || outlines[&outline_key].bounds.is_some_and(|bounds| {
                        [
                            (bounds[0], bounds[1]),
                            (bounds[0], bounds[3]),
                            (bounds[2], bounds[1]),
                            (bounds[2], bounds[3]),
                        ]
                        .into_iter()
                        .any(|(x, y)| {
                            let point = transform * Point::new(x as f64, y as f64);
                            !(point.x as f32).is_finite() || !(point.y as f32).is_finite()
                        })
                    })
                {
                    return Err(glyph_error(
                        glyph.id,
                        "derived glyph transform exceeds sparse precision/range",
                    ));
                }
                // Cull only after every admission, budget and range check
                // above has passed for this glyph.
                let touches = may_touch(transform, outlines[&outline_key].bounds, rect);
                #[cfg(test)]
                let touches = touches || CULL_DISABLED.with(|off| off.get());
                match (&mut visible, touches) {
                    (Some(kept), true) => kept.push(glyph_index as u32),
                    (None, false) => visible = Some((0..glyph_index as u32).collect()),
                    _ => {}
                }
            }
            culled = culled.saturating_add(
                visible
                    .as_ref()
                    .map_or(0, |kept| (run.glyphs.len() - kept.len()) as u64),
            );
            runs.insert(
                op_index,
                PreparedRun {
                    font: FontData {
                        data: blob.data.clone(),
                        index: blob.index,
                    },
                    coords,
                    visible,
                },
            );
        }
        let mut union_fonts = self.fonts.clone();
        union_fonts.extend(fonts.clone());
        let mut union_outlines = self.outlines.clone();
        union_outlines.extend(outlines.clone());
        let reset = self.frames >= limits.max_cache_frames
            || union_fonts.len() > limits.max_fonts
            || font_bytes(&union_fonts) > limits.max_font_bytes
            || union_outlines.len() > limits.max_cached_glyphs
            || segments(&union_outlines) > limits.max_cached_outline_segments;
        if !reset {
            fonts = union_fonts;
            outlines = union_outlines;
        }
        Ok(TextPlan {
            runs,
            fonts,
            outlines,
            reset,
            prepared,
            culled,
        })
    }
    pub fn commit(&mut self, plan: &TextPlan) {
        if plan.reset {
            self.clear();
        }
        self.fonts = plan.fonts.clone();
        self.outlines = plan.outlines.clone();
        self.prepared = self.prepared.saturating_add(plan.prepared);
        self.culled = self.culled.saturating_add(plan.culled);
        self.frames += 1;
    }
    pub fn maintain(&mut self) {
        self.cache.maintain();
    }
    pub fn draw<R: GlyphRenderer>(
        &mut self,
        renderer: &mut R,
        scene: &Scene,
        run: &SceneGlyphRun,
        prepared: &PreparedRun,
    ) -> Result<(), GlyphRenderError> {
        let m = &scene.transforms[run.transform_id as usize].m;
        let world = Affine::new([
            m[0] as f64,
            m[1] as f64,
            m[4] as f64,
            m[5] as f64,
            m[12] as f64,
            m[13] as f64,
        ]);
        let builder = GlyphRunBuilder::new(
            prepared.font.clone(),
            world,
            Affine::IDENTITY,
            OutlineBackend {
                renderer,
                cache: &mut self.cache,
            },
        )
        .font_size(run.font_size)
        .hint(false)
        .atlas_cache(false)
        .normalized_coords(&prepared.coords);
        let glyph = |g: &crate::scene::Glyph| vello_sparse_text::Glyph {
            id: g.id,
            x: g.x,
            y: g.y,
        };
        // A fully culled run still builds, so render-time font errors surface
        // exactly as before.
        match &prepared.visible {
            None => builder.fill_glyphs(run.glyphs.iter().map(glyph)),
            Some(kept) => {
                builder.fill_glyphs(kept.iter().map(|&index| glyph(&run.glyphs[index as usize])))
            }
        }
    }
}

fn font_bytes(fonts: &HashMap<FontKey, FontBlob>) -> usize {
    fonts.values().fold(0usize, |sum, font| {
        sum.saturating_add(font.data.as_ref().len())
    })
}
fn segments(outlines: &HashMap<OutlineKey, OutlineRecord>) -> usize {
    outlines
        .values()
        .fold(0usize, |sum, record| sum.saturating_add(record.segments))
}

struct CountingPen {
    segments: usize,
    bounds: Option<[f32; 4]>,
    finite: bool,
}
impl Default for CountingPen {
    fn default() -> Self {
        Self {
            segments: 0,
            bounds: None,
            finite: true,
        }
    }
}
impl CountingPen {
    fn point(&mut self, x: f32, y: f32) {
        self.finite &= x.is_finite() && y.is_finite();
        let b = self.bounds.get_or_insert([x, y, x, y]);
        b[0] = b[0].min(x);
        b[1] = b[1].min(y);
        b[2] = b[2].max(x);
        b[3] = b[3].max(y);
    }
    fn segment(&mut self) {
        self.segments = self.segments.saturating_add(1);
    }
}
/// Draw `outline` and replay it into `pen`.
///
/// Skrifa's glyf and CFF interpreters are generic over the pen. Drawing into
/// `Vec<PathElement>`, the pen Read Fonts provides and the one the Vello
/// renderers' glyph caches also use, keeps a single copy of them per binary.
fn draw_outline(
    outline: &skrifa::outline::OutlineGlyph<'_>,
    settings: DrawSettings<'_>,
    pen: &mut impl OutlinePen,
) -> Result<skrifa::outline::AdjustedMetrics, skrifa::outline::DrawError> {
    use skrifa::outline::pen::PathElement;
    let mut elements = Vec::<PathElement>::new();
    let metrics = outline.draw(settings, &mut elements)?;
    for element in elements {
        match element {
            PathElement::MoveTo { x, y } => pen.move_to(x, y),
            PathElement::LineTo { x, y } => pen.line_to(x, y),
            PathElement::QuadTo { cx0, cy0, x, y } => pen.quad_to(cx0, cy0, x, y),
            PathElement::CurveTo {
                cx0,
                cy0,
                cx1,
                cy1,
                x,
                y,
            } => pen.curve_to(cx0, cy0, cx1, cy1, x, y),
            PathElement::Close => pen.close(),
        }
    }
    Ok(metrics)
}
impl OutlinePen for CountingPen {
    fn move_to(&mut self, x: f32, y: f32) {
        self.segment();
        self.point(x, y);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.segment();
        self.point(x, y);
    }
    fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) {
        self.segment();
        self.point(cx, cy);
        self.point(x, y);
    }
    fn curve_to(&mut self, a: f32, b: f32, c: f32, d: f32, x: f32, y: f32) {
        self.segment();
        self.point(a, b);
        self.point(c, d);
        self.point(x, y);
    }
    fn close(&mut self) {
        self.segment();
    }
}

/// Native CPU/Hybrid renderers implement GlyphRenderer. The small adapter keeps
/// preparation-cache ownership here, rather than inside their opaque Resources.
struct OutlineBackend<'a, R> {
    renderer: &'a mut R,
    cache: &'a mut GlyphPrepCache,
}
impl<'a, R: GlyphRenderer> GlyphRunBackend<'a> for OutlineBackend<'a, R> {
    fn atlas_cache(self, _enabled: bool) -> Self {
        self
    }
    fn fill_glyphs<G: Iterator<Item = vello_sparse_text::Glyph> + Clone>(
        self,
        run: GlyphRun<'a>,
        glyphs: G,
    ) -> Result<(), GlyphRenderError> {
        run.build(glyphs, self.cache.as_mut(), AtlasCacher::Disabled)
            .fill_glyphs(self.renderer)
    }
    fn stroke_glyphs<G: Iterator<Item = vello_sparse_text::Glyph> + Clone>(
        self,
        run: GlyphRun<'a>,
        glyphs: G,
    ) -> Result<(), GlyphRenderError> {
        run.build(glyphs, self.cache.as_mut(), AtlasCacher::Disabled)
            .stroke_glyphs(self.renderer)
    }
    fn render_decoration<G: Iterator<Item = vello_sparse_text::Glyph> + Clone>(
        self,
        run: GlyphRun<'a>,
        glyphs: G,
        x_range: RangeInclusive<f32>,
        baseline_y: f32,
        offset: f32,
        size: f32,
        buffer: f32,
    ) {
        run.build(glyphs, self.cache.as_mut(), AtlasCacher::Disabled)
            .render_decoration(x_range, baseline_y, offset, size, buffer, self.renderer);
    }
}

#[cfg(test)]
mod admission_tests {
    use super::*;
    use std::sync::Arc;
    use crate::{Glyph, peniko::Blob};
    const BOX_FONT: &[u8] =
        include_bytes!("../../../tests/fixtures/vb3_fonts/variabletest_box.ttf");
    fn fixture() -> Scene {
        let mut scene = Scene::new(128, 128);
        let id = scene.push_font(FontBlob {
            data: Blob::new(Arc::new(BOX_FONT.to_vec())),
            index: 0,
        });
        let glyph = FontRef::new(BOX_FONT)
            .unwrap()
            .charmap()
            .map('A')
            .unwrap()
            .to_u32();
        for _ in 0..64 {
            scene.push_glyph_run(
                id,
                32.0,
                vec![Glyph {
                    id: glyph,
                    x: 8.0,
                    y: 40.0,
                }],
                [1.0; 4],
            );
        }
        scene
    }
    #[test]
    fn each_distinct_backing_and_face_is_admitted_once_per_frame() {
        let mut state = TextState::new(SparseTextLimits::default());
        let scene = fixture();
        ADMISSION_CALLS.with(|calls| calls.set(0));
        let plan = state.plan(&scene).unwrap();
        assert_eq!(plan.runs.len(), 64);
        assert_eq!(ADMISSION_CALLS.with(|calls| calls.get()), 1);
        state.commit(&plan);
        ADMISSION_CALLS.with(|calls| calls.set(0));
        let plan = state.plan(&scene).unwrap();
        assert_eq!(plan.prepared, 0);
        assert_eq!(
            ADMISSION_CALLS.with(|calls| calls.get()),
            1,
            "admission remains per frame rather than trusting a retained ID"
        );
    }
    fn set_font(scene: &mut Scene, data: Blob<u8>, index: u32) {
        scene.fonts[1] = FontBlob { data, index };
    }
    #[test]
    fn shared_backing_with_different_ids_keeps_font_budget_accounting() {
        let mut scene = fixture();
        let backing = scene.fonts[1].data.clone().into_raw_parts().0;
        let other = scene.push_font(FontBlob {
            data: Blob::new(backing),
            index: 0,
        });
        if let SceneOp::GlyphRun(run) = &mut scene.ops[1] {
            run.font_id = other;
        }
        ADMISSION_CALLS.with(|n| n.set(0));
        let state = TextState::new(SparseTextLimits::default());
        let plan = state.plan(&scene).unwrap();
        assert_eq!(ADMISSION_CALLS.with(|n| n.get()), 1);
        assert_eq!(
            plan.fonts.len(),
            2,
            "shared backing must not collapse identity/face accounting"
        );
        let state = TextState::new(SparseTextLimits {
            max_fonts: 1,
            ..Default::default()
        });
        assert!(matches!(
            state.plan(&scene),
            Err(SparseSessionError::ResourceBudgetExceeded {
                resource: "text font count",
                ..
            })
        ));
    }
    #[test]
    fn equal_bytes_in_different_backings_are_admitted_independently() {
        let mut scene = fixture();
        let id = scene.fonts[1].data.id();
        let alias = scene.push_font(FontBlob {
            data: Blob::from_raw_parts(Arc::new(BOX_FONT.to_vec()), id),
            index: 0,
        });
        if let SceneOp::GlyphRun(run) = &mut scene.ops[1] {
            run.font_id = alias;
        }
        ADMISSION_CALLS.with(|n| n.set(0));
        let plan = TextState::new(SparseTextLimits::default())
            .plan(&scene)
            .unwrap();
        assert_eq!(ADMISSION_CALLS.with(|n| n.get()), 2);
        assert_eq!(plan.fonts.len(), 1);
    }
    #[test]
    fn collision_in_later_palette_alias_refuses_atomically_even_after_face_admission() {
        let mut state = TextState::new(SparseTextLimits::default());
        let mut scene = fixture();
        let plan = state.plan(&scene).unwrap();
        state.commit(&plan);
        let before = state.stats();
        let id = scene.fonts[1].data.id();
        let mut bytes = BOX_FONT.to_vec();
        bytes.push(0);
        let alias = scene.push_font(FontBlob {
            data: Blob::from_raw_parts(Arc::new(bytes), id),
            index: 0,
        });
        if let SceneOp::GlyphRun(run) = &mut scene.ops[63] {
            run.font_id = alias;
        }
        assert!(matches!(
            state.plan(&scene),
            Err(SparseSessionError::InvalidFont {
                op_index: 63,
                reason: "font blob identity reused with different bytes",
                ..
            })
        ));
        assert_eq!(state.stats(), before);
        // Also exercise collision with only the retained allocation present.
        let bad = scene.fonts[alias as usize].data.clone();
        set_font(&mut scene, bad, 0);
        assert!(matches!(
            state.plan(&scene),
            Err(SparseSessionError::InvalidFont {
                op_index: 0,
                reason: "font blob identity reused with different bytes",
                ..
            })
        ));
        assert_eq!(state.stats(), before);
    }
    #[test]
    fn another_face_on_the_same_backing_is_not_a_cache_hit() {
        let mut scene = fixture();
        let alias = scene.push_font(FontBlob {
            data: scene.fonts[1].data.clone(),
            index: 1,
        });
        if let SceneOp::GlyphRun(run) = &mut scene.ops[1] {
            run.font_id = alias;
        }
        assert!(matches!(
            TextState::new(SparseTextLimits::default()).plan(&scene),
            Err(SparseSessionError::InvalidFont { op_index: 1, .. })
        ));
    }
    #[test]
    fn cached_face_does_not_skip_later_glyph_and_transform_checks() {
        let mut scene = fixture();
        if let SceneOp::GlyphRun(run) = &mut scene.ops[63] {
            run.glyphs[0].id = u32::MAX;
        }
        assert!(matches!(
            TextState::new(SparseTextLimits::default()).plan(&scene),
            Err(SparseSessionError::InvalidGlyph { op_index: 63, .. })
        ));
        let mut scene = fixture();
        if let SceneOp::GlyphRun(run) = &mut scene.ops[63] {
            run.glyphs[0].x = f32::INFINITY;
        }
        assert!(matches!(
            TextState::new(SparseTextLimits::default()).plan(&scene),
            Err(SparseSessionError::InvalidGlyph { op_index: 63, .. })
        ));
    }
    #[test]
    fn equal_identity_alias_is_compared_once_per_backing_with_retained_fonts() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        struct Counted(Arc<AtomicUsize>);
        impl AsRef<[u8]> for Counted {
            fn as_ref(&self) -> &[u8] {
                self.0.fetch_add(1, Ordering::Relaxed);
                BOX_FONT
            }
        }
        let mut scene = fixture();
        let calls = Arc::new(AtomicUsize::new(0));
        let alias = scene.push_font(FontBlob {
            data: Blob::from_raw_parts(Arc::new(Counted(calls.clone())), scene.fonts[1].data.id()),
            index: 0,
        });
        for (i, op) in scene.ops.iter_mut().enumerate() {
            if i % 2 == 1 {
                if let SceneOp::GlyphRun(run) = op {
                    run.font_id = alias;
                }
            }
        }
        let mut state = TextState::new(SparseTextLimits::default());
        for _ in 0..2 {
            calls.store(0, Ordering::Relaxed);
            let plan = state.plan(&scene).unwrap();
            assert_eq!(
                calls.load(Ordering::Relaxed),
                2,
                "one byte comparison and one face parse, independent of run count"
            );
            state.commit(&plan);
        }
    }
    #[test]
    fn collection_faces_on_one_backing_are_each_admitted_once() {
        let inter = include_bytes!("../../../tests/fixtures/vb3_fonts/Inter.var.subset.ttf");
        let mut bytes = Vec::from(&b"ttcf\0\x01\0\0\0\0\0\x02\0\0\0\0\0\0\0\0"[..]);
        for (face, data) in [inter.as_slice(), BOX_FONT].into_iter().enumerate() {
            let start = bytes.len().next_multiple_of(4);
            bytes.resize(start, 0);
            bytes[12 + face * 4..16 + face * 4].copy_from_slice(&(start as u32).to_be_bytes());
            bytes.extend_from_slice(data);
            let count = u16::from_be_bytes(data[4..6].try_into().unwrap()) as usize;
            for table in 0..count {
                let record = 12 + table * 16;
                let offset =
                    u32::from_be_bytes(data[record + 8..record + 12].try_into().unwrap()) as usize;
                bytes[start + record + 8..start + record + 12]
                    .copy_from_slice(&((start + offset) as u32).to_be_bytes());
                if &data[record..record + 4] == b"head" {
                    bytes[start + offset + 8..start + offset + 12].fill(0);
                }
            }
        }
        let mut scene = Scene::new(128, 128);
        let blob = Blob::new(Arc::new(bytes));
        for index in 0..2 {
            let font = FontRef::from_index(blob.as_ref(), index).unwrap();
            let glyph = font
                .charmap()
                .map(if index == 0 { 'a' } else { 'A' })
                .unwrap()
                .to_u32();
            let id = scene.push_font(FontBlob {
                data: blob.clone(),
                index,
            });
            for _ in 0..32 {
                scene.push_glyph_run(
                    id,
                    32.,
                    vec![Glyph {
                        id: glyph,
                        x: 8.,
                        y: 40.,
                    }],
                    [1.; 4],
                );
            }
        }
        ADMISSION_CALLS.with(|n| n.set(0));
        let plan = TextState::new(SparseTextLimits::default())
            .plan(&scene)
            .unwrap();
        assert_eq!(ADMISSION_CALLS.with(|n| n.get()), 2);
        assert_eq!(plan.fonts.len(), 2);
    }

    const INTER: &[u8] = include_bytes!("../../../tests/fixtures/vb3_fonts/Inter.var.subset.ttf");

    fn glyph_of(font: &[u8], ch: char) -> u32 {
        FontRef::new(font)
            .unwrap()
            .charmap()
            .map(ch)
            .unwrap()
            .to_u32()
    }

    /// Glyphs fully inside, straddling every edge at subpixel offsets, and far
    /// outside; under rotation, a run clip, tiny and large sizes, and a
    /// variation location whose outlines differ from the default.
    fn edge_scene() -> Scene {
        let mut scene = Scene::new(96, 72);
        let font = scene.push_font(FontBlob {
            data: Blob::new(Arc::new(INTER.to_vec())),
            index: 0,
        });
        // The subset's mapped glyphs, so the fixture does not depend on which
        // characters survived subsetting.
        let ids: Vec<u32> = FontRef::new(INTER)
            .unwrap()
            .charmap()
            .mappings()
            .map(|(_, glyph)| glyph.to_u32())
            .filter(|&glyph| glyph != 0)
            .take(6)
            .collect();
        assert!(ids.len() >= 2);
        let grid = |size: f32, step: f32, phase: f32| {
            let mut glyphs = Vec::new();
            let mut y = -2.0 * size + phase;
            let mut n = 0;
            while y < 72.0 + 2.0 * size {
                let mut x = -2.0 * size + phase * 0.37;
                while x < 96.0 + 2.0 * size {
                    glyphs.push(Glyph {
                        id: ids[n % ids.len()],
                        x,
                        y,
                    });
                    n += 1;
                    x += step;
                }
                y += step * 0.83;
            }
            glyphs
        };
        scene.push_glyph_run(font, 23.0, grid(23.0, 13.37, 0.31), [0.9, 0.1, 0.2, 1.0]);
        scene.push_glyph_run(font, 3.5, grid(3.5, 2.71, 0.13), [0.2, 0.6, 0.3, 0.8]);
        scene.push_glyph_run_variable(
            font,
            41.0,
            grid(41.0, 19.9, 0.77),
            [0.1, 0.2, 0.6, 0.7],
            vec![(*b"wght", 900.0)],
        );
        let mut rotated = crate::scene::Transform::rotate_2d(0.41);
        rotated.m[12] = 30.25;
        rotated.m[13] = -12.5;
        let rotated = scene.push_transform(rotated);
        scene.push_glyph_run_full(
            font,
            17.0,
            grid(17.0, 11.1, 0.5),
            [0.5, 0.4, 0.1, 0.9],
            rotated,
            NO_CLIP,
            [0.0; 4],
        );
        scene.push_glyph_run_full(
            font,
            19.0,
            grid(19.0, 9.7, 0.21),
            [0.3, 0.3, 0.3, 0.6],
            0,
            [20.5, 30.25, 61.75, 50.5],
            [4.0, 0.0, 6.0, 2.0],
        );
        scene
    }

    #[test]
    fn culling_runs_after_every_glyph_check() {
        let scene = edge_scene();
        let plan = TextState::new(SparseTextLimits::default())
            .plan(&scene)
            .unwrap();
        assert!(plan.culled > 0, "the fixture must exercise culling");
        let total: usize = scene
            .ops
            .iter()
            .filter_map(|op| match op {
                SceneOp::GlyphRun(run) => Some(run.glyphs.len()),
                _ => None,
            })
            .sum();
        assert!((plan.culled as usize) < total, "some glyphs remain visible");

        // An invalid glyph far outside the target still refuses the frame.
        let mut bad = edge_scene();
        if let SceneOp::GlyphRun(run) = &mut bad.ops[0] {
            run.glyphs.push(Glyph {
                id: u32::MAX,
                x: -10_000.0,
                y: -10_000.0,
            });
        }
        assert!(matches!(
            TextState::new(SparseTextLimits::default()).plan(&bad),
            Err(SparseSessionError::InvalidGlyph {
                op_index: 0,
                glyph_id: u32::MAX,
                ..
            })
        ));

        // Culled glyphs still count against positioned-glyph and segment budgets.
        for limits in [
            SparseTextLimits {
                max_glyphs: total - 1,
                ..Default::default()
            },
            SparseTextLimits {
                max_outline_segments: 1,
                ..Default::default()
            },
        ] {
            assert!(matches!(
                TextState::new(limits).plan(&scene),
                Err(SparseSessionError::ResourceBudgetExceeded { .. })
            ));
        }

        // A run entirely outside the target keeps its admission and prepares
        // its outlines, but draws nothing.
        let mut away = Scene::new(32, 32);
        let font = away.push_font(FontBlob {
            data: Blob::new(Arc::new(INTER.to_vec())),
            index: 0,
        });
        away.push_glyph_run(
            font,
            20.0,
            vec![Glyph {
                id: glyph_of(INTER, 'a'),
                x: 500.0,
                y: 500.0,
            }],
            [1.0; 4],
        );
        let mut state = TextState::new(SparseTextLimits::default());
        let plan = state.plan(&away).unwrap();
        assert_eq!(plan.culled, 1);
        assert_eq!(plan.prepared, 1);
        assert_eq!(plan.runs[&0].visible.as_deref(), Some(&[][..]));
        state.commit(&plan);
        assert_eq!(state.stats().cached_glyphs, 1);
        assert_eq!(state.stats().culled_glyphs, 1);
    }

    #[cfg(feature = "vello-cpu")]
    #[test]
    fn culled_cpu_pixels_match_unculled_pixels_exactly() {
        use super::super::{CpuSession, SparseResourceLimits};
        let render = |disabled: bool| {
            CULL_DISABLED.with(|off| off.set(disabled));
            let pixels = CpuSession::new(SparseResourceLimits::default())
                .render(&edge_scene())
                .unwrap();
            CULL_DISABLED.with(|off| off.set(false));
            pixels.data_as_u8_slice().to_vec()
        };
        let reference = render(true);
        assert!(reference.iter().any(|&byte| byte != 0));
        assert!(reference == render(false), "culling changed pixels");
    }
}
