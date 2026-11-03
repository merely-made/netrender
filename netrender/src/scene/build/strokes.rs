// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

use super::*;

impl Scene {
    /// Phase 11b': append an arbitrary path filled with a single
    /// solid color. Identity transform, no clip.
    pub fn push_shape_filled(&mut self, path: ScenePath, color: [f32; 4]) {
        self.ops.push(SceneOp::Shape(SceneShape {
            path,
            fill_color: Some(color),
            stroke: None,
            transform_id: 0,
            clip_rect: NO_CLIP,
            clip_corner_radii: SHARP_CLIP,
        }));
    }

    /// Phase 11b': append an arbitrary path stroked with a single
    /// solid color and line width. Identity transform, no clip.
    pub fn push_shape_stroked(&mut self, path: ScenePath, color: [f32; 4], stroke_width: f32) {
        self.ops.push(SceneOp::Shape(SceneShape {
            path,
            fill_color: None,
            stroke: Some(ScenePathStroke {
                color,
                width: stroke_width,
                cap: SceneStrokeCap::default(),
                join: SceneStrokeJoin::default(),
                dash_pattern: Vec::new(),
                dash_offset: 0.0,
            }),
            transform_id: 0,
            clip_rect: NO_CLIP,
            clip_corner_radii: SHARP_CLIP,
        }));
    }

    /// Phase 11': append a sharp axis-aligned stroked rect (border).
    /// Append a fully-specified stroke op. The struct-accepting escape hatch for
    /// callers needing rounded corners + dashes + transform/clip at once (e.g.
    /// a rounded, dashed CSS border) — the positional `push_stroke*` helpers each
    /// cover only a subset.
    pub fn push_stroke_op(&mut self, stroke: SceneStroke) {
        self.ops.push(SceneOp::Stroke(stroke));
    }

    pub fn push_stroke(
        &mut self,
        x0: f32,
        y0: f32,
        x1: f32,
        y1: f32,
        color: [f32; 4],
        stroke_width: f32,
    ) {
        self.ops.push(SceneOp::Stroke(SceneStroke {
            x0,
            y0,
            x1,
            y1,
            color,
            stroke_width,
            stroke_corner_radii: SHARP_CLIP,
            transform_id: 0,
            clip_rect: NO_CLIP,
            clip_corner_radii: SHARP_CLIP,
            cap: SceneStrokeCap::default(),
            join: SceneStrokeJoin::default(),
            dash_pattern: Vec::new(),
            dash_offset: 0.0,
        }));
    }

    /// Phase 11': append a stroked rounded-rect (CSS border with
    /// `border-radius`). `stroke_corner_radii` rounds the path
    /// itself, in `[top_left, top_right, bottom_right, bottom_left]`
    /// order. All-zero radii produce a sharp rectangular stroke.
    pub fn push_stroke_rounded(
        &mut self,
        x0: f32,
        y0: f32,
        x1: f32,
        y1: f32,
        color: [f32; 4],
        stroke_width: f32,
        stroke_corner_radii: [f32; 4],
    ) {
        self.ops.push(SceneOp::Stroke(SceneStroke {
            x0,
            y0,
            x1,
            y1,
            color,
            stroke_width,
            stroke_corner_radii,
            transform_id: 0,
            clip_rect: NO_CLIP,
            clip_corner_radii: SHARP_CLIP,
            cap: SceneStrokeCap::default(),
            join: SceneStrokeJoin::default(),
            dash_pattern: Vec::new(),
            dash_offset: 0.0,
        }));
    }

    /// Phase 11': append a stroked rect/rounded-rect with full
    /// control over transform, clip, and clip corner radii.
    pub fn push_stroke_full(
        &mut self,
        x0: f32,
        y0: f32,
        x1: f32,
        y1: f32,
        color: [f32; 4],
        stroke_width: f32,
        stroke_corner_radii: [f32; 4],
        transform_id: u32,
        clip_rect: [f32; 4],
        clip_corner_radii: [f32; 4],
    ) {
        self.ops.push(SceneOp::Stroke(SceneStroke {
            x0,
            y0,
            x1,
            y1,
            color,
            stroke_width,
            stroke_corner_radii,
            transform_id,
            clip_rect,
            clip_corner_radii,
            cap: SceneStrokeCap::default(),
            join: SceneStrokeJoin::default(),
            dash_pattern: Vec::new(),
            dash_offset: 0.0,
        }));
    }

    /// Roadmap C1 — append a dashed stroked rect with explicit
    /// cap / join / dash pattern. `dash_pattern` is alternating
    /// on/off lengths in device pixels; `dash_offset` shifts the
    /// pattern phase. Empty `dash_pattern` produces a solid stroke.
    /// All other fields default to identity transform / no clip /
    /// sharp corners — for richer construction, build the
    /// [`SceneStroke`] struct directly and push it.
    pub fn push_stroke_decorated(
        &mut self,
        x0: f32,
        y0: f32,
        x1: f32,
        y1: f32,
        color: [f32; 4],
        stroke_width: f32,
        cap: SceneStrokeCap,
        join: SceneStrokeJoin,
        dash_pattern: Vec<f32>,
    ) {
        self.ops.push(SceneOp::Stroke(SceneStroke {
            x0,
            y0,
            x1,
            y1,
            color,
            stroke_width,
            stroke_corner_radii: SHARP_CLIP,
            transform_id: 0,
            clip_rect: NO_CLIP,
            clip_corner_radii: SHARP_CLIP,
            cap,
            join,
            dash_pattern,
            dash_offset: 0.0,
        }));
    }
}
