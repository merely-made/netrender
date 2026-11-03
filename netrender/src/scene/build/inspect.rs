// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

use super::*;

impl Scene {
    /// Roadmap A1 — pretty-print the op list for debugging.
    ///
    /// Returns a multi-line string with one header line summarising the
    /// scene (viewport, palette sizes, op count) followed by one line
    /// per [`SceneOp`] in painter order. Each op line shows the op
    /// kind, the key geometry / color fields, and any non-default
    /// `transform_id` / `clip_rect` / `clip_corner_radii`.
    /// `PushLayer` / `PopLayer` pairs nest the indentation by two
    /// spaces per level — ops between them visibly belong to the
    /// layer scope.
    ///
    /// Output is for human reading; format is **not** stable. Don't
    /// parse it, snapshot-test it, or treat it as a contract.
    pub fn dump_ops(&self) -> String {
        use std::fmt::Write;

        let mut out = String::new();
        writeln!(
            out,
            "Scene {}x{}  transforms={}  fonts={}  images={}  surfaces={}  ops={}",
            self.viewport_width,
            self.viewport_height,
            self.transforms.len(),
            self.fonts.len(),
            self.image_sources.len(),
            self.compositor_surfaces.len(),
            self.ops.len(),
        )
        .ok();

        if self.root_alpha != 1.0 || self.root_blend_mode != SceneBlendMode::Normal {
            writeln!(
                out,
                "  root_alpha={}  root_blend_mode={:?}",
                self.root_alpha, self.root_blend_mode,
            )
            .ok();
        }

        let mut depth: usize = 0;
        for (i, op) in self.ops.iter().enumerate() {
            // PopLayer un-indents *before* its own line so the pop
            // visually closes the scope it ends.
            if matches!(op, SceneOp::PopLayer) {
                depth = depth.saturating_sub(1);
            }
            let pad = "  ".repeat(depth + 1);
            write!(out, "  {:04}{}", i, pad).ok();
            dump_op(&mut out, op);
            writeln!(out).ok();
            if matches!(op, SceneOp::PushLayer(_)) {
                depth += 1;
            }
        }

        out
    }

    /// Iterate the rect ops of the scene in painter order. Other op
    /// variants are filtered out.
    pub fn iter_rects(&self) -> impl Iterator<Item = &SceneRect> + '_ {
        self.ops.iter().filter_map(|op| match op {
            SceneOp::Rect(r) => Some(r),
            _ => None,
        })
    }

    /// Iterate the stroke ops of the scene in painter order.
    pub fn iter_strokes(&self) -> impl Iterator<Item = &SceneStroke> + '_ {
        self.ops.iter().filter_map(|op| match op {
            SceneOp::Stroke(s) => Some(s),
            _ => None,
        })
    }

    /// Iterate the gradient ops of the scene in painter order.
    pub fn iter_gradients(&self) -> impl Iterator<Item = &SceneGradient> + '_ {
        self.ops.iter().filter_map(|op| match op {
            SceneOp::Gradient(g) => Some(g),
            _ => None,
        })
    }

    /// Iterate the image ops of the scene in painter order.
    pub fn iter_images(&self) -> impl Iterator<Item = &SceneImage> + '_ {
        self.ops.iter().filter_map(|op| match op {
            SceneOp::Image(i) => Some(i),
            _ => None,
        })
    }

    /// Iterate the shape ops of the scene in painter order.
    pub fn iter_shapes(&self) -> impl Iterator<Item = &SceneShape> + '_ {
        self.ops.iter().filter_map(|op| match op {
            SceneOp::Shape(s) => Some(s),
            _ => None,
        })
    }

    /// Iterate the glyph-run ops of the scene in painter order.
    pub fn iter_glyph_runs(&self) -> impl Iterator<Item = &SceneGlyphRun> + '_ {
        self.ops.iter().filter_map(|op| match op {
            SceneOp::GlyphRun(g) => Some(g),
            _ => None,
        })
    }
}
