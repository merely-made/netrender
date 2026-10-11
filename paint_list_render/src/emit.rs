// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Per-command emitters: clips, layers, nine-patch borders, and gradients,
//! plus the box-shadow key base and transform/matrix helpers.

use std::collections::HashMap;

use log::warn;
use netrender::{
    GradientKind, ImageKey as NrImageKey, NO_CLIP, SHARP_CLIP, Scene, SceneBlendMode, SceneClip,
    SceneLayer, SceneStroke, SceneStrokeCap, SceneStrokeJoin, Transform,
};
use paint_list_api::{self as ple, ColorF, ImageKey};
use crate::convert::*;

pub(crate) const BOX_SHADOW_MASK_KEY_BASE: u64 = 0xFFFF_0000_0000_0000;

/// Apply a column-major 4×4 transform to a 2D point `(x, y, 0, 1)`,
/// returning the transformed `(x', y')`. Used to lift a shadow box
/// from its element-local coords into absolute scene coords for the
/// GPU mask pass (the mask is built in scene space, not local space).
pub(crate) fn apply_transform_2d(m: &[f32; 16], x: f32, y: f32) -> (f32, f32) {
    (m[0] * x + m[4] * y + m[12], m[1] * x + m[5] * y + m[13])
}

/// The `Transform` at a palette index; identity for index 0.
pub(crate) fn transform_at(scene: &Scene, tid: u32) -> Transform {
    scene
        .transforms
        .get(tid as usize)
        .copied()
        .unwrap_or(Transform::IDENTITY)
}

/// Column-major 4×4 matrix multiply: `a ∘ b` (apply `b` first, then
/// `a`). Used to compose a child transform with its parent so nested
/// `PushTransform`s accumulate.
pub(crate) fn mat_mul(a: &[f32; 16], b: &[f32; 16]) -> [f32; 16] {
    let mut out = [0.0f32; 16];
    for col in 0..4 {
        for row in 0..4 {
            let mut s = 0.0;
            for k in 0..4 {
                s += a[k * 4 + row] * b[col * 4 + k];
            }
            out[col * 4 + row] = s;
        }
    }
    out
}

// =============================================================================
// PaintCmd per-variant emit helpers
// =============================================================================

pub(crate) fn emit_push_clip(scene: &mut Scene, spec: &ple::ClipSpec, tid: u32) {
    let clip = match &spec.kind {
        ple::ClipKind::Rect(rect) => {
            let (x0, y0, x1, y1) = rect_corners(rect);
            SceneClip::Rect {
                rect: [x0, y0, x1, y1],
                radii: [0.0, 0.0, 0.0, 0.0],
            }
        }
        ple::ClipKind::RoundedRect { rect, radius, .. } => {
            let (x0, y0, x1, y1) = rect_corners(rect);
            SceneClip::Rect {
                rect: [x0, y0, x1, y1],
                radii: [
                    radius.top_left.width,
                    radius.top_right.width,
                    radius.bottom_right.width,
                    radius.bottom_left.width,
                ],
            }
        }
        // Arbitrary-path clip (CSS clip-path basic shapes). The scene + vello
        // rasterizer already lower `SceneClip::Path` to a kurbo BezPath clip
        // (Phase 9b'); reuse the same path reconstruction as DrawPath.
        ple::ClipKind::Path(pd) => SceneClip::Path(path_data_to_scene_path(pd)),
    };
    // The clip layer carries the active transform so its geometry is
    // resolved in the same coordinate space as the clipped content.
    scene.push_layer(SceneLayer {
        clip,
        alpha: 1.0,
        blend_mode: SceneBlendMode::Normal,
        compose: netrender::SceneCompose::SrcOver,
        transform_id: tid,
        backdrop_filter: None,
        filters: Vec::new(),
    });
}

/// Map a paint-list `FilterOp` to a netrender `SceneFilter`. `Opacity` returns
/// `None` — it folds into the layer alpha, not the output-filter chain.
fn filter_op_to_scene(f: &ple::FilterOp) -> Option<netrender::SceneFilter> {
    use netrender::SceneFilter as F;
    use ple::FilterOp as O;
    Some(match *f {
        O::Blur(r) => F::Blur(r),
        O::Brightness(v) => F::Brightness(v),
        O::Contrast(v) => F::Contrast(v),
        O::Grayscale(v) => F::Grayscale(v),
        O::HueRotate(v) => F::HueRotate(v),
        O::Invert(v) => F::Invert(v),
        O::Saturate(v) => F::Saturate(v),
        O::Sepia(v) => F::Sepia(v),
        O::Opacity(_) => return None,
        // `drop-shadow()` is its own increment (an element-alpha shadow, like
        // box-shadow); `color-matrix()` (arbitrary 4x5) is a follow-up.
        O::DropShadow { .. } | O::ColorMatrix(_) => return None,
    })
}

pub(crate) fn compose_with_origin(t: &Transform, ox: f32, oy: f32) -> Transform {
    // The netrender Transform is a flat 16-float array. "translate by
    // (ox, oy) then apply t" sets the translation columns: [12] += ox,
    // [13] += oy.
    let mut out = t.m;
    out[12] += ox;
    out[13] += oy;
    Transform { m: out }
}

pub(crate) fn emit_push_layer(scene: &mut Scene, spec: &ple::LayerSpec, tid: u32) {
    let blend_mode = mix_blend_mode_to_scene(spec.mix_blend_mode);
    let mut alpha = spec.opacity;
    // `opacity()` in the chain folds into the layer alpha; the other filter
    // functions apply to the layer's own rasterized output (SceneLayer::filters,
    // rendered offscreen + filtered + composited by netrender).
    for filter in &spec.filters {
        if let ple::FilterOp::Opacity(a) = filter {
            alpha *= *a;
        }
    }
    let filters: Vec<netrender::SceneFilter> =
        spec.filters.iter().filter_map(filter_op_to_scene).collect();
    let _ = spec.raster_space; // Local vs Screen — deferred
    let _ = spec.flags; // BLEND_CONTAINER etc. — deferred
    let _ = &spec.mask; // alpha-mask layer — deferred
    scene.push_layer(SceneLayer {
        clip: SceneClip::None,
        alpha,
        blend_mode,
        compose: netrender::SceneCompose::SrcOver,
        transform_id: tid,
        backdrop_filter: None,
        filters,
    });
}

/// Lower a nine-patch (`border-image`) border. The source image is sliced into
/// four corners, four edges, and an optional fill center; each region is sampled
/// from the source by a UV sub-rect and drawn to its destination — corners
/// scaled, edges stretched or tiled per `repeat_horizontal` / `repeat_vertical`,
/// the center filled when `fill`. One source image, UV-sampled per region (no
/// producer-side pre-slicing), and partial `repeat` tiles are UV-cropped rather
/// than clipped — so no clip-layer edge AA.
///
/// `border.placement.bounds` is the border-image area; `border.widths` the
/// destination border widths; `np.slice` the source slice insets (px); `np.width`
/// / `np.height` the source intrinsic size.
pub(crate) fn emit_nine_patch(
    scene: &mut Scene,
    border: &ple::BorderItem,
    np: &ple::NinePatchBorder,
    image_map: &HashMap<ImageKey, NrImageKey>,
    tid: u32,
) {
    let key = match np.source {
        ple::NinePatchSource::Image(k, _rendering) => match image_map.get(&k) {
            Some(&nr) => nr,
            None => {
                warn!(
                    "[paint translator] nine-patch source image {:?} unregistered; skipping",
                    k
                );
                return;
            }
        },
        // Gradient sources emit directly elsewhere; only image sources slice here.
        _ => {
            warn!("[paint translator] nine-patch gradient source deferred");
            return;
        }
    };
    let (w, h) = (np.width as f32, np.height as f32);
    if w <= 0.0 || h <= 0.0 {
        return;
    }
    let (dx0, dy0, dx1, dy1) = rect_corners(&border.placement.bounds);
    let (wt, wr, wb, wl) = (
        border.widths.top,
        border.widths.right,
        border.widths.bottom,
        border.widths.left,
    );
    let (st, sr, sb, sl) = (
        np.slice.top as f32,
        np.slice.right as f32,
        np.slice.bottom as f32,
        np.slice.left as f32,
    );

    // Sample source px-rect (sx0,sy0)-(sx1,sy1) onto dest rect (a,b)-(c,d), scaled.
    // `push_image_clamped` crops to the UV sub-rect so a slice region never bleeds
    // its neighbour's source pixels across the seam.
    let img = |scene: &mut Scene, a, b, c, d, sx0: f32, sy0: f32, sx1: f32, sy1: f32| {
        if c - a <= 0.0 || d - b <= 0.0 || sx1 - sx0 <= 0.0 || sy1 - sy0 <= 0.0 {
            return;
        }
        scene.push_image_clamped(
            a,
            b,
            c,
            d,
            [sx0 / w, sy0 / h, sx1 / w, sy1 / h],
            [1.0, 1.0, 1.0, 1.0],
            key,
            tid,
            NO_CLIP,
        );
    };

    // Corners — always scaled.
    img(scene, dx0, dy0, dx0 + wl, dy0 + wt, 0.0, 0.0, sl, st);
    img(scene, dx1 - wr, dy0, dx1, dy0 + wt, w - sr, 0.0, w, st);
    img(scene, dx0, dy1 - wb, dx0 + wl, dy1, 0.0, h - sb, sl, h);
    img(scene, dx1 - wr, dy1 - wb, dx1, dy1, w - sr, h - sb, w, h);

    // One edge: lay the source strip [s_along_lo..s_along_hi] × [s_cross_lo..hi]
    // along the dest [d_along_lo..hi] at cross [d_cross_lo..hi], per `repeat`. For
    // a horizontal edge `along` is x and `cross` is y; for a vertical edge they
    // swap. The cross axis fills the border width; the tile's along-length scales
    // proportionally.
    #[allow(clippy::too_many_arguments)]
    fn tile_edge(
        scene: &mut Scene,
        img: &dyn Fn(&mut Scene, f32, f32, f32, f32, f32, f32, f32, f32),
        horizontal: bool,
        d_along_lo: f32,
        d_along_hi: f32,
        d_cross_lo: f32,
        d_cross_hi: f32,
        s_along_lo: f32,
        s_along_hi: f32,
        s_cross_lo: f32,
        s_cross_hi: f32,
        repeat: ple::RepeatMode,
    ) {
        let d_len = d_along_hi - d_along_lo;
        let d_cross = d_cross_hi - d_cross_lo;
        let s_along = s_along_hi - s_along_lo;
        let s_cross = s_cross_hi - s_cross_lo;
        if d_len <= 0.0 || d_cross <= 0.0 || s_along <= 0.0 || s_cross <= 0.0 {
            return;
        }
        // One tile covering dest along [a0,a1] sampling source along [su0,su1].
        let put = |scene: &mut Scene, a0: f32, a1: f32, su0: f32, su1: f32| {
            if horizontal {
                img(
                    scene, a0, d_cross_lo, a1, d_cross_hi, su0, s_cross_lo, su1, s_cross_hi,
                );
            } else {
                img(
                    scene, d_cross_lo, a0, d_cross_hi, a1, s_cross_lo, su0, s_cross_hi, su1,
                );
            }
        };
        // Natural tile along-length: the source strip scaled so its cross fills the
        // border width.
        let natural = (s_along * (d_cross / s_cross)).max(1.0);
        match repeat {
            ple::RepeatMode::Stretch => put(scene, d_along_lo, d_along_hi, s_along_lo, s_along_hi),
            ple::RepeatMode::Round => {
                // Rescale so a whole number of tiles fills the edge exactly.
                let n = (d_len / natural).round().max(1.0);
                let t = d_len / n;
                for i in 0..(n as usize) {
                    let a0 = d_along_lo + i as f32 * t;
                    put(scene, a0, a0 + t, s_along_lo, s_along_hi);
                }
            }
            ple::RepeatMode::Space => {
                // Whole tiles at natural size with equal gaps; first/last at the
                // edges. None if not even one fits.
                let n = (d_len / natural).floor();
                if n < 1.0 {
                    return;
                }
                let gap = if n > 1.0 {
                    (d_len - n * natural) / (n - 1.0)
                } else {
                    0.0
                };
                // A lone tile (no room for two) centers in the edge.
                let base = if n == 1.0 {
                    d_along_lo + (d_len - natural) / 2.0
                } else {
                    d_along_lo
                };
                for i in 0..(n as usize) {
                    let a0 = base + i as f32 * (natural + gap);
                    put(scene, a0, a0 + natural, s_along_lo, s_along_hi);
                }
            }
            ple::RepeatMode::Repeat => {
                // Whole tiles from the start, then a UV-cropped partial.
                let mut a0 = d_along_lo;
                let mut guard = 0;
                while a0 < d_along_hi - 0.01 && guard < 4096 {
                    let a1 = (a0 + natural).min(d_along_hi);
                    let su1 = s_along_lo + s_along * ((a1 - a0) / natural);
                    put(scene, a0, a1, s_along_lo, su1);
                    a0 += natural;
                    guard += 1;
                }
            }
        }
    }

    // Top + bottom edges (horizontal), then left + right (vertical).
    tile_edge(
        scene,
        &img,
        true,
        dx0 + wl,
        dx1 - wr,
        dy0,
        dy0 + wt,
        sl,
        w - sr,
        0.0,
        st,
        np.repeat_horizontal,
    );
    tile_edge(
        scene,
        &img,
        true,
        dx0 + wl,
        dx1 - wr,
        dy1 - wb,
        dy1,
        sl,
        w - sr,
        h - sb,
        h,
        np.repeat_horizontal,
    );
    tile_edge(
        scene,
        &img,
        false,
        dy0 + wt,
        dy1 - wb,
        dx0,
        dx0 + wl,
        st,
        h - sb,
        0.0,
        sl,
        np.repeat_vertical,
    );
    tile_edge(
        scene,
        &img,
        false,
        dy0 + wt,
        dy1 - wb,
        dx1 - wr,
        dx1,
        st,
        h - sb,
        w - sr,
        w,
        np.repeat_vertical,
    );

    // Center — only with `fill`; stretched (tiled fill is a refinement).
    if np.fill {
        img(
            scene,
            dx0 + wl,
            dy0 + wt,
            dx1 - wr,
            dy1 - wb,
            sl,
            st,
            w - sr,
            h - sb,
        );
    }
}

/// CSS radius normalization uses one common factor for all corner axes.
fn normalized_radii(mut radii: [[f32; 2]; 4], width: f32, height: f32) -> [[f32; 2]; 4] {
    let mut factor = 1.0f64;
    for (extent, sum) in [
        (width, f64::from(radii[0][0]) + f64::from(radii[1][0])),
        (width, f64::from(radii[3][0]) + f64::from(radii[2][0])),
        (height, f64::from(radii[0][1]) + f64::from(radii[3][1])),
        (height, f64::from(radii[1][1]) + f64::from(radii[2][1])),
    ] {
        if sum > 0.0 {
            factor = factor.min(f64::from(extent.max(0.0)) / sum);
        }
    }
    for corner in &mut radii {
        for axis in corner {
            *axis = (f64::from(axis.max(0.0)) * factor) as f32;
        }
    }
    radii
}
/// One cubic per quarter ellipse, the same geometric approximation used by
/// normal renderer rounded paths. Opposite winding makes the inner hole.
fn rounded_contour(
    path: &mut netrender::ScenePath,
    b: [f32; 4],
    r: [[f32; 2]; 4],
    clockwise: bool,
) {
    let [x0, y0, x1, y1] = b;
    let [[tlx, tly], [trx, try_], [brx, bry], [blx, bly]] = r;
    let k = 0.552_284_8;
    path.move_to(x0 + tlx, y0);
    if clockwise {
        path.line_to(x1 - trx, y0)
            .cubic_to(
                x1 - trx + k * trx,
                y0,
                x1,
                y0 + try_ - k * try_,
                x1,
                y0 + try_,
            )
            .line_to(x1, y1 - bry)
            .cubic_to(x1, y1 - bry + k * bry, x1 - brx + k * brx, y1, x1 - brx, y1)
            .line_to(x0 + blx, y1)
            .cubic_to(x0 + blx - k * blx, y1, x0, y1 - bly + k * bly, x0, y1 - bly)
            .line_to(x0, y0 + tly)
            .cubic_to(x0, y0 + tly - k * tly, x0 + tlx - k * tlx, y0, x0 + tlx, y0);
    } else {
        path.cubic_to(x0 + tlx - k * tlx, y0, x0, y0 + tly - k * tly, x0, y0 + tly)
            .line_to(x0, y1 - bly)
            .cubic_to(x0, y1 - bly + k * bly, x0 + blx - k * blx, y1, x0 + blx, y1)
            .line_to(x1 - brx, y1)
            .cubic_to(x1 - brx + k * brx, y1, x1, y1 - bry + k * bry, x1, y1 - bry)
            .line_to(x1, y0 + try_)
            .cubic_to(
                x1,
                y0 + try_ - k * try_,
                x1 - trx + k * trx,
                y0,
                x1 - trx,
                y0,
            )
            .line_to(x0 + tlx, y0);
    }
    path.close();
}
/// Extend a corner seam through the rectangular inner corner to its first
/// intersection with the rounded inner hole. Stopping at the inner bounding
/// box would leave the corner crescent unpainted. The four returned points
/// belong to the convex hole; their connecting quadrilateral is also inside
/// it, so the side polygons cover the complete ring without overlap.
fn inner_seam(outer: [f32; 2], inner: [f32; 2], center: [f32; 2], radius: [f32; 2]) -> [f32; 2] {
    if radius[0] <= 0.0 || radius[1] <= 0.0 {
        return inner;
    }
    let direction = [
        f64::from(inner[0] - outer[0]),
        f64::from(inner[1] - outer[1]),
    ];
    let offset = [
        f64::from(outer[0] - center[0]),
        f64::from(outer[1] - center[1]),
    ];
    let radii = radius.map(f64::from);
    let a = (direction[0] / radii[0]).powi(2) + (direction[1] / radii[1]).powi(2);
    if a == 0.0 {
        return inner;
    }
    let b = 2.0
        * (offset[0] * direction[0] / radii[0].powi(2)
            + offset[1] * direction[1] / radii[1].powi(2));
    let c = (offset[0] / radii[0]).powi(2) + (offset[1] / radii[1]).powi(2) - 1.0;
    let t = ((-b - (b * b - 4.0 * a * c).max(0.0).sqrt()) / (2.0 * a)).max(1.0);
    [
        (f64::from(outer[0]) + direction[0] * t) as f32,
        (f64::from(outer[1]) + direction[1] * t) as f32,
    ]
}

/// Joined solid side polygons share diagonal seams. The rounded ring clips
/// their union, so side colors never paint overlapping square rectangles or
/// leak into the transparent inner hole. Corner ellipses retain both axes.
fn emit_joined_solid_border(
    scene: &mut Scene,
    border: &ple::BorderItem,
    sides: &ple::NormalBorder,
    tid: u32,
) {
    let [x0, y0, x1, y1] = {
        let (a, b, c, d) = rect_corners(&border.placement.bounds);
        [a, b, c, d]
    };
    let width = x1 - x0;
    let height = y1 - y0;
    if width <= 0.0 || height <= 0.0 {
        return;
    }
    let effective = |width: f32, side: ple::BorderSide| {
        if side.style == ple::BorderStyle::Solid {
            width.max(0.0)
        } else {
            0.0
        }
    };
    let top = effective(border.widths.top, sides.top);
    let right = effective(border.widths.right, sides.right);
    let bottom = effective(border.widths.bottom, sides.bottom);
    let left = effective(border.widths.left, sides.left);
    if [top, right, bottom, left].iter().all(|w| *w == 0.0) {
        return;
    }
    let [mut ix0, mut iy0, mut ix1, mut iy1] = [x0 + left, y0 + top, x1 - right, y1 - bottom];
    if ix0 > ix1 {
        ix0 = (f64::from(x0)
            + f64::from(width) * f64::from(left) / (f64::from(left) + f64::from(right)))
            as f32;
        ix1 = ix0;
    }
    if iy0 > iy1 {
        iy0 = (f64::from(y0)
            + f64::from(height) * f64::from(top) / (f64::from(top) + f64::from(bottom)))
            as f32;
        iy1 = iy0;
    }
    let radius = sides.radius;
    let outer = normalized_radii(
        [
            radius.top_left,
            radius.top_right,
            radius.bottom_right,
            radius.bottom_left,
        ]
        .map(|r| [r.width, r.height]),
        width,
        height,
    );
    let inner = normalized_radii(
        [
            [(outer[0][0] - left).max(0.0), (outer[0][1] - top).max(0.0)],
            [(outer[1][0] - right).max(0.0), (outer[1][1] - top).max(0.0)],
            [
                (outer[2][0] - right).max(0.0),
                (outer[2][1] - bottom).max(0.0),
            ],
            [
                (outer[3][0] - left).max(0.0),
                (outer[3][1] - bottom).max(0.0),
            ],
        ],
        ix1 - ix0,
        iy1 - iy0,
    );
    let mut seam = [[ix0, iy0], [ix1, iy0], [ix1, iy1], [ix0, iy1]];
    if ix1 > ix0 && iy1 > iy0 {
        for (i, (outer_corner, center)) in [
            ([x0, y0], [ix0 + inner[0][0], iy0 + inner[0][1]]),
            ([x1, y0], [ix1 - inner[1][0], iy0 + inner[1][1]]),
            ([x1, y1], [ix1 - inner[2][0], iy1 - inner[2][1]]),
            ([x0, y1], [ix0 + inner[3][0], iy1 - inner[3][1]]),
        ]
        .into_iter()
        .enumerate()
        {
            seam[i] = inner_seam(outer_corner, seam[i], center, inner[i]);
        }
    }
    let mut ring = netrender::ScenePath::new();
    rounded_contour(&mut ring, [x0, y0, x1, y1], outer, true);
    if ix1 > ix0 && iy1 > iy0 {
        rounded_contour(&mut ring, [ix0, iy0, ix1, iy1], inner, false);
    }
    let active_colors: Vec<_> = [
        (top, sides.top.color),
        (right, sides.right.color),
        (bottom, sides.bottom.color),
        (left, sides.left.color),
    ]
    .into_iter()
    .filter_map(|(w, c)| if w > 0.0 { Some(c) } else { None })
    .collect();
    if active_colors.iter().all(|c| *c == active_colors[0]) {
        scene.push_shape(netrender::SceneShape {
            path: ring,
            fill_color: Some(color_to_array(&active_colors[0])),
            stroke: None,
            transform_id: tid,
            clip_rect: NO_CLIP,
            clip_corner_radii: SHARP_CLIP,
        });
        return;
    }
    scene.push_layer(SceneLayer {
        clip: SceneClip::Path(ring),
        alpha: 1.0,
        blend_mode: SceneBlendMode::Normal,
        compose: netrender::SceneCompose::SrcOver,
        transform_id: tid,
        backdrop_filter: None,
        filters: Vec::new(),
    });
    for (w, color, points) in [
        (top, sides.top.color, [[x0, y0], [x1, y0], seam[1], seam[0]]),
        (
            right,
            sides.right.color,
            [[x1, y0], [x1, y1], seam[2], seam[1]],
        ),
        (
            bottom,
            sides.bottom.color,
            [[x1, y1], [x0, y1], seam[3], seam[2]],
        ),
        (
            left,
            sides.left.color,
            [[x0, y1], [x0, y0], seam[0], seam[3]],
        ),
    ] {
        if w == 0.0 {
            continue;
        }
        let mut path = netrender::ScenePath::new();
        path.move_to(points[0][0], points[0][1]);
        for p in &points[1..] {
            path.line_to(p[0], p[1]);
        }
        path.close();
        // Adjacent antialiased coverage is a partition, so add its premultiplied
        // RGBA instead of attenuating earlier coverage with SrcOver. The outer
        // ring layer subsequently composes this isolated result over its parent.
        scene.push_layer(SceneLayer {
            clip: SceneClip::None,
            alpha: 1.0,
            blend_mode: SceneBlendMode::Normal,
            compose: netrender::SceneCompose::Plus,
            transform_id: 0,
            backdrop_filter: None,
            filters: Vec::new(),
        });
        scene.push_shape(netrender::SceneShape {
            path,
            fill_color: Some(color_to_array(&color)),
            stroke: None,
            transform_id: tid,
            clip_rect: NO_CLIP,
            clip_corner_radii: SHARP_CLIP,
        });
        scene.pop_layer();
    }
    scene.pop_layer();
}

pub(crate) fn emit_border_first_cut(scene: &mut Scene, border: &ple::BorderItem, tid: u32) {
    use ple::BorderStyle as BS;
    let rect = &border.placement.bounds;
    let widths = &border.widths;
    let sides = match &border.details {
        ple::BorderDetails::Normal(n) => n,
        ple::BorderDetails::NinePatch(_) => {
            warn!("[paint translator] nine-patch border deferred");
            return;
        }
    };

    if [
        (widths.top, sides.top),
        (widths.right, sides.right),
        (widths.bottom, sides.bottom),
        (widths.left, sides.left),
    ]
    .iter()
    .all(|(width, side)| *width <= 0.0 || matches!(side.style, BS::Solid | BS::None | BS::Hidden))
    {
        emit_joined_solid_border(scene, border, sides, tid);
        return;
    }

    // Uniform border (all four sides identical width / style / color) is the
    // common case and the only one a single stroked path can render — strokes
    // can't carry per-side colors. When it's also rounded or dashed/dotted, take
    // the stroke fast path; the 4-rect fallback below handles the rest (and
    // per-side borders, always square).
    let r = &sides.radius;
    let has_radius = r.top_left.width > 0.0
        || r.top_right.width > 0.0
        || r.bottom_right.width > 0.0
        || r.bottom_left.width > 0.0;
    let w = widths.top;
    let uniform_width = (widths.right - w).abs() < 0.01
        && (widths.bottom - w).abs() < 0.01
        && (widths.left - w).abs() < 0.01;
    let s = sides.top.style;
    let uniform_style = sides.right.style == s && sides.bottom.style == s && sides.left.style == s;
    let c = sides.top.color;
    let col_eq = |a: &ColorF, b: &ColorF| {
        (a.r - b.r).abs() < 0.004
            && (a.g - b.g).abs() < 0.004
            && (a.b - b.b).abs() < 0.004
            && (a.a - b.a).abs() < 0.004
    };
    let uniform_color = col_eq(&sides.right.color, &c)
        && col_eq(&sides.bottom.color, &c)
        && col_eq(&sides.left.color, &c);
    // dotted/dashed need a stroke (dash pattern); solid needs a stroke only to
    // round. `double`/`groove`/etc. fall through to the (square, solid) fallback.
    let dash = match s {
        BS::Dotted => Some(vec![w.max(1.0), w.max(1.0)]),
        BS::Dashed => Some(vec![(w * 3.0).max(1.0), w.max(1.0)]),
        _ => None,
    };
    let strokeable = matches!(s, BS::Solid | BS::Dotted | BS::Dashed);

    if w > 0.01
        && uniform_width
        && uniform_style
        && uniform_color
        && strokeable
        && (has_radius || dash.is_some())
    {
        // Stroke is centered on its path; a CSS border sits inside the
        // border-box, so inset the path by w/2 and shrink the corner radii to
        // match (radius is measured at the border-box edge).
        let h = w * 0.5;
        let inset = |r: f32| (r - h).max(0.0);
        scene.push_stroke_op(SceneStroke {
            x0: rect.min.x + h,
            y0: rect.min.y + h,
            x1: rect.max.x - h,
            y1: rect.max.y - h,
            color: color_to_array(&c),
            stroke_width: w,
            stroke_corner_radii: [
                inset(r.top_left.width),
                inset(r.top_right.width),
                inset(r.bottom_right.width),
                inset(r.bottom_left.width),
            ],
            transform_id: tid,
            clip_rect: NO_CLIP,
            clip_corner_radii: SHARP_CLIP,
            cap: SceneStrokeCap::Butt,
            join: SceneStrokeJoin::Miter,
            dash_pattern: dash.unwrap_or_default(),
            dash_offset: 0.0,
        });
        return;
    }

    // Fallback: per-side filled rects (square corners). Handles non-uniform
    // borders and styles the stroke path doesn't model yet (double/groove/...).
    if widths.top > 0.0 {
        scene.push_rect_transformed(
            rect.min.x,
            rect.min.y,
            rect.max.x,
            rect.min.y + widths.top,
            color_to_array(&sides.top.color),
            tid,
        );
    }
    if widths.bottom > 0.0 {
        scene.push_rect_transformed(
            rect.min.x,
            rect.max.y - widths.bottom,
            rect.max.x,
            rect.max.y,
            color_to_array(&sides.bottom.color),
            tid,
        );
    }
    if widths.left > 0.0 {
        scene.push_rect_transformed(
            rect.min.x,
            rect.min.y,
            rect.min.x + widths.left,
            rect.max.y,
            color_to_array(&sides.left.color),
            tid,
        );
    }
    if widths.right > 0.0 {
        scene.push_rect_transformed(
            rect.max.x - widths.right,
            rect.min.y,
            rect.max.x,
            rect.max.y,
            color_to_array(&sides.right.color),
            tid,
        );
    }
    let _ = sides.radius;
    let _ = sides.do_aa;
}

pub(crate) fn emit_linear_gradient(scene: &mut Scene, item: &ple::LinearGradientItem, tid: u32) {
    let rect = &item.placement.bounds;
    let g = &item.gradient;
    scene.push_gradient(netrender::SceneGradient {
        x0: rect.min.x,
        y0: rect.min.y,
        x1: rect.max.x,
        y1: rect.max.y,
        kind: GradientKind::Linear,
        repeat: matches!(g.extend_mode, ple::ExtendMode::Repeat),
        params: [
            g.start_point.x,
            g.start_point.y,
            g.end_point.x,
            g.end_point.y,
        ],
        stops: gradient_stops(&g.stops),
        transform_id: tid,
        clip_rect: NO_CLIP,
        clip_corner_radii: [0.0; 4],
    });
}

pub(crate) fn emit_radial_gradient(scene: &mut Scene, item: &ple::RadialGradientItem, tid: u32) {
    let rect = &item.placement.bounds;
    let g = &item.gradient;
    scene.push_gradient(netrender::SceneGradient {
        x0: rect.min.x,
        y0: rect.min.y,
        x1: rect.max.x,
        y1: rect.max.y,
        kind: GradientKind::Radial,
        repeat: matches!(g.extend_mode, ple::ExtendMode::Repeat),
        params: [g.center.x, g.center.y, g.radius.width, g.radius.height],
        stops: gradient_stops(&g.stops),
        transform_id: tid,
        clip_rect: NO_CLIP,
        clip_corner_radii: [0.0; 4],
    });
}

pub(crate) fn emit_conic_gradient(scene: &mut Scene, item: &ple::ConicGradientItem, tid: u32) {
    let rect = &item.placement.bounds;
    let g = &item.gradient;
    scene.push_gradient(netrender::SceneGradient {
        x0: rect.min.x,
        y0: rect.min.y,
        x1: rect.max.x,
        y1: rect.max.y,
        kind: GradientKind::Conic,
        repeat: matches!(g.extend_mode, ple::ExtendMode::Repeat),
        params: [g.center.x, g.center.y, g.angle, 0.0],
        stops: gradient_stops(&g.stops),
        transform_id: tid,
        clip_rect: NO_CLIP,
        clip_corner_radii: [0.0; 4],
    });
}
