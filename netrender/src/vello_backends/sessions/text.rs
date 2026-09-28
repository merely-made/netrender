// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Validated outline text using the pinned Glifo renderer, with a preparation
//! cache owned here so lifetime limits can actually release retained outlines.

use super::{SparseSessionError, budget, validate};
use crate::scene::{FontBlob, Scene, SceneGlyphRun, SceneOp};
use skrifa::{
    FontRef, GlyphId, MetadataProvider,
    instance::Size,
    outline::{DrawSettings, OutlinePen},
    raw::TableProvider,
};
use std::{collections::HashMap, ops::RangeInclusive};
use vello_sparse_common::{
    kurbo::{Affine, Point},
    peniko::FontData,
};
use vello_sparse_text::{
    AtlasCacher, GlyphPrepCache, GlyphRenderer, GlyphRun, GlyphRunBackend, GlyphRunBuilder,
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

pub(super) struct PreparedRun {
    pub font: FontData,
    pub coords: Vec<i16>,
}
pub(super) struct TextPlan {
    pub runs: HashMap<usize, PreparedRun>,
    fonts: HashMap<FontKey, FontBlob>,
    outlines: HashMap<OutlineKey, OutlineRecord>,
    reset: bool,
    prepared: u64,
}

pub(super) struct TextState {
    limits: SparseTextLimits,
    fonts: HashMap<FontKey, FontBlob>,
    outlines: HashMap<OutlineKey, OutlineRecord>,
    cache: GlyphPrepCache,
    frames: u32,
    prepared: u64,
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
        let mut outlines = HashMap::new();
        let mut runs = HashMap::new();
        let mut glyphs = 0usize;
        let mut frame_segments = 0usize;
        let mut prepared = 0u64;
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
            for prior in self.fonts.values().chain(fonts.values()) {
                if prior.data.id() == blob.data.id() && prior.data.as_ref() != blob.data.as_ref() {
                    return Err(font_error("font blob identity reused with different bytes"));
                }
            }
            fonts.entry(key).or_insert_with(|| blob.clone());
            budget("text font count", fonts.len(), limits.max_fonts)?;
            budget("text font bytes", font_bytes(&fonts), limits.max_font_bytes)?;
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
                *b"COLR", *b"CPAL", *b"CBDT", *b"CBLC", *b"EBDT", *b"EBLC", *b"sbix", *b"SVG ",
                *b"bdat", *b"bloc",
            ] {
                if font.table_data(skrifa::Tag::new(&tag)).is_some() {
                    return Err(font_error(
                        "color, bitmap and SVG font tables are not admitted by outline text sessions",
                    ));
                }
            }
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
            budget("font variation axes", axes.len(), limits.max_variation_axes)?;
            let location = axes.location(settings);
            let coords: Vec<i16> = location.coords().iter().map(|v| v.to_bits()).collect();
            let collection = font.outline_glyphs();
            if collection.format().is_none() {
                return Err(font_error("font has no supported outlines"));
            }
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
            for glyph in &run.glyphs {
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
                        outline
                            .draw(
                                DrawSettings::unhinted(Size::new(upem as f32), location.coords()),
                                &mut pen,
                            )
                            .map_err(|_| {
                                glyph_error(glyph.id, "glyph outline failed validation")
                            })?;
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
            }
            runs.insert(
                op_index,
                PreparedRun {
                    font: FontData {
                        data: blob.data.clone(),
                        index: blob.index,
                    },
                    coords,
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
        })
    }
    pub fn commit(&mut self, plan: &TextPlan) {
        if plan.reset {
            self.clear();
        }
        self.fonts = plan.fonts.clone();
        self.outlines = plan.outlines.clone();
        self.prepared = self.prepared.saturating_add(plan.prepared);
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
    ) {
        let m = &scene.transforms[run.transform_id as usize].m;
        let world = Affine::new([
            m[0] as f64,
            m[1] as f64,
            m[4] as f64,
            m[5] as f64,
            m[12] as f64,
            m[13] as f64,
        ]);
        GlyphRunBuilder::new(
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
        .normalized_coords(&prepared.coords)
        .fill_glyphs(run.glyphs.iter().map(|g| vello_sparse_text::Glyph {
            id: g.id,
            x: g.x,
            y: g.y,
        }));
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
    ) {
        run.build(glyphs, self.cache.as_mut(), AtlasCacher::Disabled)
            .fill_glyphs(self.renderer);
    }
    fn stroke_glyphs<G: Iterator<Item = vello_sparse_text::Glyph> + Clone>(
        self,
        run: GlyphRun<'a>,
        glyphs: G,
    ) {
        run.build(glyphs, self.cache.as_mut(), AtlasCacher::Disabled)
            .stroke_glyphs(self.renderer);
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
