// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Parameter admission for owned sessions; intentionally separate from legacy
//! operation-only capabilities. These checks are not a hostile-code sandbox.

use super::{SparseResourceLimits, SparseSessionError, budget};
use crate::scene::{GradientKind, NO_CLIP, PathOp, Scene, SceneClip, SceneOp, ScenePath};
use vello_sparse_common::kurbo::{Affine, Point};

fn invalid(index: Option<usize>, reason: &'static str) -> SparseSessionError {
    SparseSessionError::InvalidScene {
        op_index: index,
        reason,
    }
}

fn finite(values: &[f32]) -> bool {
    values.iter().all(|value| value.is_finite())
}

fn color(value: [f32; 4]) -> bool {
    finite(&value)
        && (0.0..=1.0).contains(&value[3])
        && value[..3]
            .iter()
            .all(|component| (0.0..=value[3]).contains(component))
}

fn rect(value: [f32; 4]) -> bool {
    finite(&value) && value[0] <= value[2] && value[1] <= value[3]
}

fn radii(value: [f32; 4]) -> bool {
    finite(&value) && value.iter().all(|v| *v >= 0.0)
}

fn clip(value: [f32; 4], corners: [f32; 4]) -> bool {
    (value == NO_CLIP || rect(value)) && radii(corners)
}

fn stroke(width: f32, dash: &[f32], offset: f32) -> bool {
    width.is_finite()
        && width >= 0.0
        && offset.is_finite()
        && finite(dash)
        && dash.iter().all(|v| *v > 0.0)
}

fn path(value: &ScenePath) -> bool {
    let mut started = false;
    for op in &value.ops {
        let valid = match *op {
            PathOp::MoveTo(x, y) => {
                started = true;
                finite(&[x, y])
            }
            PathOp::LineTo(x, y) => started && finite(&[x, y]),
            PathOp::QuadTo(cx, cy, x, y) => started && finite(&[cx, cy, x, y]),
            PathOp::CubicTo(a, b, c, d, x, y) => started && finite(&[a, b, c, d, x, y]),
            PathOp::Close => started,
        };
        if !valid {
            return false;
        }
    }
    true
}

pub(super) fn scene(scene: &Scene, limits: SparseResourceLimits) -> Result<(), SparseSessionError> {
    if scene.viewport_width == 0 || scene.viewport_height == 0 {
        return Err(invalid(None, "viewport dimensions must be nonzero"));
    }
    budget(
        "viewport pixels",
        (scene.viewport_width as usize).saturating_mul(scene.viewport_height as usize),
        limits.max_viewport_pixels,
    )?;
    if !scene.root_alpha.is_finite() || !(0.0..=1.0).contains(&scene.root_alpha) {
        return Err(invalid(None, "root alpha must be finite and in [0,1]"));
    }
    if !scene.compositor_surfaces.is_empty() {
        return Err(invalid(
            None,
            "owned sparse sessions do not compose native surfaces",
        ));
    }
    let mut cost = scene.ops.len().saturating_add(scene.transforms.len());
    budget("scene complexity", cost, limits.max_ops)?;
    for transform in &scene.transforms {
        let m = &transform.m;
        if !finite(m)
            || m[2] != 0.0
            || m[3] != 0.0
            || m[6] != 0.0
            || m[7] != 0.0
            || m[8] != 0.0
            || m[9] != 0.0
            || m[10] != 1.0
            || m[11] != 0.0
            || m[14] != 0.0
            || m[15] != 1.0
        {
            return Err(invalid(None, "transform must be a finite 2D affine matrix"));
        }
        let affine = vello_sparse_common::kurbo::Affine::new([
            m[0] as f64,
            m[1] as f64,
            m[4] as f64,
            m[5] as f64,
            m[12] as f64,
            m[13] as f64,
        ]);
        if !safe_affine(affine) {
            return Err(invalid(
                None,
                "transform or its inverse exceeds sparse f32 precision/range",
            ));
        }
    }
    let mut depth = usize::from(
        scene.root_alpha != 1.0 || scene.root_blend_mode != crate::scene::SceneBlendMode::Normal,
    );
    budget("layer depth", depth, limits.max_layers)?;
    for (index, op) in scene.ops.iter().enumerate() {
        let valid = match op {
            SceneOp::GlyphRun(v) => {
                cost = cost
                    .saturating_add(v.glyphs.len())
                    .saturating_add(v.font_axis_values.len());
                if v.clip_rect != NO_CLIP {
                    budget("layer depth", depth.saturating_add(1), limits.max_layers)?;
                }
                v.font_size.is_finite()
                    && v.font_size > 0.0
                    && color(v.color)
                    && clip(v.clip_rect, v.clip_corner_radii)
                    && v.glyphs.iter().all(|glyph| finite(&[glyph.x, glyph.y]))
                    && v.font_axis_values.iter().all(|(tag, value)| {
                        value.is_finite() && tag.iter().all(|byte| (32..=126).contains(byte))
                    })
            }
            SceneOp::Image(v) => {
                // Ordered normalized UVs are this slice's supported subset.
                // Degenerate/flipped/out-of-range UVs are explicit refusals.
                let valid = rect([v.x0, v.y0, v.x1, v.y1])
                    && v.x0 < v.x1
                    && v.y0 < v.y1
                    && color(v.color)
                    && clip(v.clip_rect, v.clip_corner_radii)
                    && finite(&v.uv)
                    && v.uv.iter().all(|x| (0.0..=1.0).contains(x))
                    && v.uv[0] < v.uv[2]
                    && v.uv[1] < v.uv[3];
                if v.clip_rect != NO_CLIP {
                    budget("layer depth", depth.saturating_add(1), limits.max_layers)?;
                }
                valid
            }
            SceneOp::Pattern(v) => {
                if v.clip_rect != NO_CLIP {
                    budget("layer depth", depth.saturating_add(1), limits.max_layers)?;
                }
                rect(v.extent)
                    && v.extent[0] < v.extent[2]
                    && v.extent[1] < v.extent[3]
                    && finite(&v.scale)
                    && clip(v.clip_rect, v.clip_corner_radii)
            }
            SceneOp::Rect(v) => {
                if v.clip_rect != NO_CLIP {
                    budget("layer depth", depth.saturating_add(1), limits.max_layers)?;
                }
                rect([v.x0, v.y0, v.x1, v.y1])
                    && color(v.color)
                    && clip(v.clip_rect, v.clip_corner_radii)
            }
            SceneOp::Stroke(v) => {
                cost = cost.saturating_add(v.dash_pattern.len());
                if v.clip_rect != NO_CLIP {
                    budget("layer depth", depth.saturating_add(1), limits.max_layers)?;
                }
                rect([v.x0, v.y0, v.x1, v.y1])
                    && color(v.color)
                    && clip(v.clip_rect, v.clip_corner_radii)
                    && radii(v.stroke_corner_radii)
                    && stroke(v.stroke_width, &v.dash_pattern, v.dash_offset)
            }
            SceneOp::Gradient(v) => {
                cost = cost.saturating_add(v.stops.len());
                if v.clip_rect != NO_CLIP {
                    budget("layer depth", depth.saturating_add(1), limits.max_layers)?;
                }
                rect([v.x0, v.y0, v.x1, v.y1])
                    && clip(v.clip_rect, v.clip_corner_radii)
                    && finite(&v.params)
                    && !v.stops.is_empty()
                    && v.stops.iter().all(|stop| {
                        stop.offset.is_finite()
                            && (0.0..=1.0).contains(&stop.offset)
                            && color(stop.color)
                    })
                    && v.stops
                        .windows(2)
                        .all(|pair| pair[0].offset <= pair[1].offset)
                    && (!matches!(v.kind, GradientKind::Radial)
                        || (v.params[2] > 0.0 && v.params[3] > 0.0))
            }
            SceneOp::Shape(v) => {
                cost = cost.saturating_add(v.path.ops.len());
                if v.clip_rect != NO_CLIP {
                    budget("layer depth", depth.saturating_add(1), limits.max_layers)?;
                }
                let valid_stroke = v.stroke.as_ref().is_none_or(|v| {
                    cost = cost.saturating_add(v.dash_pattern.len());
                    color(v.color) && stroke(v.width, &v.dash_pattern, v.dash_offset)
                });
                path(&v.path)
                    && v.fill_color.is_none_or(color)
                    && valid_stroke
                    && clip(v.clip_rect, v.clip_corner_radii)
            }
            SceneOp::PushLayer(v) => {
                depth = depth.saturating_add(1);
                budget("layer depth", depth, limits.max_layers)?;
                v.alpha.is_finite()
                    && (0.0..=1.0).contains(&v.alpha)
                    && match &v.clip {
                        SceneClip::None => true,
                        SceneClip::Rect {
                            rect: bounds,
                            radii: corners,
                        } => rect(*bounds) && radii(*corners),
                        SceneClip::Path(value) => {
                            cost = cost.saturating_add(value.ops.len());
                            path(value)
                        }
                    }
            }
            SceneOp::PopLayer => {
                depth -= 1;
                true
            }
            // Operation-level admission rejects these before parameter checks.
            SceneOp::Fragment(_) => unreachable!(),
        };
        if !valid {
            return Err(invalid(
                Some(index),
                "non-finite, degenerate, or unsupported primitive parameters",
            ));
        }
        if !derived_geometry(scene, op) {
            return Err(invalid(
                Some(index),
                "derived geometry or gradient mapping exceeds sparse f32 precision/range",
            ));
        }
        budget("scene complexity", cost, limits.max_ops)?;
    }
    Ok(())
}

fn world(scene: &Scene, id: u32) -> Affine {
    let m = &scene.transforms[id as usize].m;
    Affine::new([
        m[0] as f64,
        m[1] as f64,
        m[4] as f64,
        m[5] as f64,
        m[12] as f64,
        m[13] as f64,
    ])
}

fn point_safe(affine: Affine, x: f64, y: f64) -> bool {
    let point = affine * Point::new(x, y);
    (point.x as f32).is_finite() && (point.y as f32).is_finite()
}

fn bounds_safe(affine: Affine, bounds: [f32; 4], margin: f64) -> bool {
    [
        (bounds[0] as f64 - margin, bounds[1] as f64 - margin),
        (bounds[0] as f64 - margin, bounds[3] as f64 + margin),
        (bounds[2] as f64 + margin, bounds[1] as f64 - margin),
        (bounds[2] as f64 + margin, bounds[3] as f64 + margin),
    ]
    .into_iter()
    .all(|(x, y)| point_safe(affine, x, y))
}

fn path_safe(affine: Affine, path: &ScenePath, margin: f64) -> bool {
    path.ops.iter().all(|op| {
        let point = |x: f32, y: f32| bounds_safe(affine, [x, y, x, y], margin);
        match *op {
            PathOp::MoveTo(x, y) | PathOp::LineTo(x, y) => point(x, y),
            PathOp::QuadTo(a, b, x, y) => point(a, b) && point(x, y),
            PathOp::CubicTo(a, b, c, d, x, y) => point(a, b) && point(c, d) && point(x, y),
            PathOp::Close => true,
        }
    })
}

fn derived_geometry(scene: &Scene, op: &SceneOp) -> bool {
    match op {
        SceneOp::Pattern(v) => {
            let world = world(scene, v.transform_id);
            let brush = super::pattern_transform(v);
            safe_affine(brush) && safe_affine(world * brush) && bounds_safe(world, v.extent, 0.0)
        }
        SceneOp::Rect(v) => {
            bounds_safe(world(scene, v.transform_id), [v.x0, v.y0, v.x1, v.y1], 0.0)
        }
        SceneOp::Stroke(v) => bounds_safe(
            world(scene, v.transform_id),
            [v.x0, v.y0, v.x1, v.y1],
            v.stroke_width as f64 * 4.0,
        ),
        SceneOp::Gradient(v) => {
            let world = world(scene, v.transform_id);
            let [a, b, c, d] = v.params;
            let brush = match v.kind {
                GradientKind::Radial => {
                    Affine::translate((a as f64, b as f64))
                        * Affine::scale_non_uniform(c as f64, d as f64)
                }
                GradientKind::Linear => {
                    let dx = c as f64 - a as f64;
                    let dy = d as f64 - b as f64;
                    Affine::new([dx, dy, -dy, dx, a as f64, b as f64])
                }
                _ => Affine::IDENTITY,
            };
            let paint_valid = match v.kind {
                GradientKind::Linear => {
                    bounds_safe(world, [a.min(c), b.min(d), a.max(c), b.max(d)], 0.0)
                }
                GradientKind::Radial => bounds_safe(world, [a, b, a, b], c.max(d) as f64),
                GradientKind::Conic => {
                    point_safe(world, a as f64, b as f64)
                        && ((c as f64 + std::f32::consts::TAU as f64) as f32).is_finite()
                }
            };
            paint_valid
                && safe_affine(brush)
                && safe_affine(world * brush)
                && bounds_safe(world, [v.x0, v.y0, v.x1, v.y1], 0.0)
        }
        SceneOp::Shape(v) => path_safe(
            world(scene, v.transform_id),
            &v.path,
            v.stroke
                .as_ref()
                .map_or(0.0, |stroke| stroke.width as f64 * 4.0),
        ),
        SceneOp::PushLayer(v) => match &v.clip {
            SceneClip::None => true,
            SceneClip::Rect { rect, .. } => bounds_safe(world(scene, v.transform_id), *rect, 0.0),
            SceneClip::Path(path) => path_safe(world(scene, v.transform_id), path, 0.0),
        },
        _ => true,
    }
}

/// Sparse paint encoding ultimately narrows affine coefficients to f32. Avoid
/// infinities, underflow-to-zero determinants and overflowing inverses there.
pub(super) fn safe_affine(affine: vello_sparse_common::kurbo::Affine) -> bool {
    let coeffs = affine.as_coeffs();
    let f32_coeffs = coeffs.map(|value| value as f32);
    if !finite(&f32_coeffs) {
        return false;
    }
    let determinant = f32_coeffs[0] * f32_coeffs[3] - f32_coeffs[1] * f32_coeffs[2];
    determinant.is_finite()
        && determinant != 0.0
        && affine
            .inverse()
            .as_coeffs()
            .iter()
            .all(|value| (*value as f32).is_finite())
}
