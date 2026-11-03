// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

use super::*;

impl Scene {
    /// Phase 8D general API: push an arbitrary-kind, arbitrary-stops
    /// gradient. The 2-stop convenience methods below build a
    /// `SceneGradient` and forward to this.
    pub fn push_gradient(&mut self, gradient: SceneGradient) {
        self.ops.push(SceneOp::Gradient(gradient));
    }

    /// 2-stop linear gradient (Phase 8A convenience; preserved post-8D).
    pub fn push_linear_gradient(
        &mut self,
        x0: f32,
        y0: f32,
        x1: f32,
        y1: f32,
        start: [f32; 2],
        end: [f32; 2],
        color0: [f32; 4],
        color1: [f32; 4],
    ) {
        self.ops.push(SceneOp::Gradient(two_stop_gradient(
            GradientKind::Linear,
            x0,
            y0,
            x1,
            y1,
            [start[0], start[1], end[0], end[1]],
            color0,
            color1,
            0,
            NO_CLIP,
        )));
    }

    /// 2-stop linear gradient with full control over transform and clip.
    pub fn push_linear_gradient_full(
        &mut self,
        x0: f32,
        y0: f32,
        x1: f32,
        y1: f32,
        start: [f32; 2],
        end: [f32; 2],
        color0: [f32; 4],
        color1: [f32; 4],
        transform_id: u32,
        clip_rect: [f32; 4],
    ) {
        self.ops.push(SceneOp::Gradient(two_stop_gradient(
            GradientKind::Linear,
            x0,
            y0,
            x1,
            y1,
            [start[0], start[1], end[0], end[1]],
            color0,
            color1,
            transform_id,
            clip_rect,
        )));
    }

    /// 2-stop radial gradient (Phase 8B convenience). For circular,
    /// pass `radii = [r, r]`. Color0 at center, color1 at the
    /// elliptical boundary (clamps beyond).
    pub fn push_radial_gradient(
        &mut self,
        x0: f32,
        y0: f32,
        x1: f32,
        y1: f32,
        center: [f32; 2],
        radii: [f32; 2],
        color0: [f32; 4],
        color1: [f32; 4],
    ) {
        self.ops.push(SceneOp::Gradient(two_stop_gradient(
            GradientKind::Radial,
            x0,
            y0,
            x1,
            y1,
            [center[0], center[1], radii[0], radii[1]],
            color0,
            color1,
            0,
            NO_CLIP,
        )));
    }

    /// 2-stop conic gradient (Phase 8C convenience). `t = 0` at
    /// `start_angle`, sweeping clockwise (with y-down screen coords)
    /// back to the seam at `t = 1`.
    pub fn push_conic_gradient(
        &mut self,
        x0: f32,
        y0: f32,
        x1: f32,
        y1: f32,
        center: [f32; 2],
        start_angle: f32,
        color0: [f32; 4],
        color1: [f32; 4],
    ) {
        self.ops.push(SceneOp::Gradient(two_stop_gradient(
            GradientKind::Conic,
            x0,
            y0,
            x1,
            y1,
            [center[0], center[1], start_angle, 0.0],
            color0,
            color1,
            0,
            NO_CLIP,
        )));
    }

    /// 2-stop conic gradient with full control over transform and clip.
    pub fn push_conic_gradient_full(
        &mut self,
        x0: f32,
        y0: f32,
        x1: f32,
        y1: f32,
        center: [f32; 2],
        start_angle: f32,
        color0: [f32; 4],
        color1: [f32; 4],
        transform_id: u32,
        clip_rect: [f32; 4],
    ) {
        self.ops.push(SceneOp::Gradient(two_stop_gradient(
            GradientKind::Conic,
            x0,
            y0,
            x1,
            y1,
            [center[0], center[1], start_angle, 0.0],
            color0,
            color1,
            transform_id,
            clip_rect,
        )));
    }

    /// 2-stop radial gradient with full control over transform and clip.
    pub fn push_radial_gradient_full(
        &mut self,
        x0: f32,
        y0: f32,
        x1: f32,
        y1: f32,
        center: [f32; 2],
        radii: [f32; 2],
        color0: [f32; 4],
        color1: [f32; 4],
        transform_id: u32,
        clip_rect: [f32; 4],
    ) {
        self.ops.push(SceneOp::Gradient(two_stop_gradient(
            GradientKind::Radial,
            x0,
            y0,
            x1,
            y1,
            [center[0], center[1], radii[0], radii[1]],
            color0,
            color1,
            transform_id,
            clip_rect,
        )));
    }
}
