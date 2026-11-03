// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

use super::{BackendAdmissionError, VelloBackend};
use crate::scene::{
    GradientKind, NO_CLIP, PathOp, Scene, SceneBlendMode, SceneClip, SceneCompose, SceneGradient,
    SceneLayer, SceneOp, ScenePath, ScenePathStroke, SceneStrokeCap, SceneStrokeJoin, Transform,
};
use vello_sparse_common::{
    kurbo::{
        Affine, BezPath, Cap, Join, Point, Rect, RoundedRect, RoundedRectRadii, Shape, Stroke,
    },
    paint::PaintType,
    peniko::{BlendMode, Color, ColorStop, Compose, Extend, Fill, Gradient, Mix},
};

pub(super) trait SparseContext {
    fn set_transform(&mut self, transform: Affine);
    fn set_paint_transform(&mut self, transform: Affine);
    fn set_fill_rule(&mut self, fill: Fill);
    fn set_paint(&mut self, paint: PaintType);
    fn set_stroke(&mut self, stroke: Stroke);
    fn fill_rect(&mut self, rect: &Rect);
    fn fill_path(&mut self, path: &BezPath);
    fn stroke_path(&mut self, path: &BezPath);
    fn push_layer(&mut self, clip: Option<&BezPath>, blend: Option<BlendMode>, alpha: Option<f32>);
    fn pop_layer(&mut self);
}

#[cfg(feature = "vello-cpu")]
impl SparseContext for vello_cpu::RenderContext {
    fn set_transform(&mut self, transform: Affine) {
        self.set_transform(transform);
    }
    fn set_paint_transform(&mut self, transform: Affine) {
        self.set_paint_transform(transform);
    }
    fn set_fill_rule(&mut self, fill: Fill) {
        self.set_fill_rule(fill);
    }
    fn set_paint(&mut self, paint: PaintType) {
        self.set_paint(paint);
    }
    fn set_stroke(&mut self, stroke: Stroke) {
        self.set_stroke(stroke);
    }
    fn fill_rect(&mut self, rect: &Rect) {
        self.fill_rect(rect);
    }
    fn fill_path(&mut self, path: &BezPath) {
        self.fill_path(path);
    }
    fn stroke_path(&mut self, path: &BezPath) {
        self.stroke_path(path);
    }
    fn push_layer(&mut self, clip: Option<&BezPath>, blend: Option<BlendMode>, alpha: Option<f32>) {
        self.push_layer(clip, blend, alpha, None, None);
    }
    fn pop_layer(&mut self) {
        self.pop_layer();
    }
}

#[cfg(any(feature = "vello-hybrid", feature = "vello-webgl"))]
impl SparseContext for vello_hybrid::Scene {
    fn set_transform(&mut self, transform: Affine) {
        self.set_transform(transform);
    }
    fn set_paint_transform(&mut self, transform: Affine) {
        self.set_paint_transform(transform);
    }
    fn set_fill_rule(&mut self, fill: Fill) {
        self.set_fill_rule(fill);
    }
    fn set_paint(&mut self, paint: PaintType) {
        self.set_paint(paint);
    }
    fn set_stroke(&mut self, stroke: Stroke) {
        self.set_stroke(stroke);
    }
    fn fill_rect(&mut self, rect: &Rect) {
        self.fill_rect(rect);
    }
    fn fill_path(&mut self, path: &BezPath) {
        self.fill_path(path);
    }
    fn stroke_path(&mut self, path: &BezPath) {
        self.stroke_path(path);
    }
    fn push_layer(&mut self, clip: Option<&BezPath>, blend: Option<BlendMode>, alpha: Option<f32>) {
        self.push_layer(clip, blend, alpha, None, None);
    }
    fn pop_layer(&mut self) {
        self.pop_layer();
    }
}

pub(super) fn dimensions(
    scene: &Scene,
    backend: VelloBackend,
) -> Result<(u16, u16), BackendAdmissionError> {
    super::sparse_dimensions(backend, scene)
}

pub(super) fn lower<C: SparseContext>(
    context: &mut C,
    scene: &Scene,
    backend: VelloBackend,
) -> Result<(), BackendAdmissionError> {
    super::validate_scene_for_backend(backend, scene)?;

    lower_admitted(
        context,
        scene,
        &std::collections::HashMap::new(),
        |_, _, _| unreachable!("free lowerers refuse text"),
    );
    Ok(())
}

pub(super) struct ImagePaint {
    pub image: vello_sparse_common::paint::Image,
    pub transform: Affine,
}

/// Called only after complete resource and operation admission.
pub(super) fn lower_admitted<C: SparseContext>(
    context: &mut C,
    scene: &Scene,
    images: &std::collections::HashMap<usize, ImagePaint>,
    mut draw_text: impl FnMut(&mut C, usize, &crate::scene::SceneGlyphRun),
) {
    let has_root_layer = scene.root_alpha != 1.0 || scene.root_blend_mode != SceneBlendMode::Normal;
    if has_root_layer {
        context.set_transform(Affine::IDENTITY);
        context.push_layer(
            None,
            Some(map_blend(scene.root_blend_mode, SceneCompose::SrcOver)),
            Some(scene.root_alpha.clamp(0.0, 1.0)),
        );
    }

    for (op_index, op) in scene.ops.iter().enumerate() {
        match op {
            SceneOp::GlyphRun(run) => {
                let clip = primitive_clip(run.clip_rect, run.clip_corner_radii);
                if let Some(path) = clip.as_ref() {
                    context.set_transform(Affine::IDENTITY);
                    context.push_layer(Some(path), None, None);
                }
                context.set_transform(transform(&scene.transforms[run.transform_id as usize]));
                context.set_paint_transform(Affine::IDENTITY);
                context.set_fill_rule(Fill::NonZero);
                context.set_paint(color(run.color).into());
                draw_text(context, op_index, run);
                if clip.is_some() {
                    context.pop_layer();
                }
            }
            SceneOp::Image(image) => {
                let paint = &images[&op_index];
                let clip = primitive_clip(image.clip_rect, image.clip_corner_radii);
                if let Some(path) = clip.as_ref() {
                    context.set_transform(Affine::IDENTITY);
                    context.push_layer(Some(path), None, None);
                }
                context.set_transform(transform(&scene.transforms[image.transform_id as usize]));
                context.set_paint_transform(paint.transform);
                context.set_paint(paint.image.clone().into());
                context.fill_rect(&Rect::new(
                    image.x0 as f64,
                    image.y0 as f64,
                    image.x1 as f64,
                    image.y1 as f64,
                ));
                if clip.is_some() {
                    context.pop_layer();
                }
            }
            SceneOp::Pattern(pattern) => {
                let paint = &images[&op_index];
                let clip = primitive_clip(pattern.clip_rect, pattern.clip_corner_radii);
                if let Some(path) = clip.as_ref() {
                    context.set_transform(Affine::IDENTITY);
                    context.push_layer(Some(path), None, None);
                }
                context.set_transform(transform(&scene.transforms[pattern.transform_id as usize]));
                context.set_paint_transform(paint.transform);
                context.set_paint(paint.image.clone().into());
                let [x0, y0, x1, y1] = pattern.extent;
                context.fill_rect(&Rect::new(x0 as f64, y0 as f64, x1 as f64, y1 as f64));
                if clip.is_some() {
                    context.pop_layer();
                }
            }
            SceneOp::Rect(rect) => {
                let world = transform(&scene.transforms[rect.transform_id as usize]);
                let clip = primitive_clip(rect.clip_rect, rect.clip_corner_radii);
                if let Some(path) = clip.as_ref() {
                    // Primitive clips are carried in device space, unlike
                    // the primitive itself. Match the Classic adapter by
                    // recording the clip under identity.
                    context.set_transform(Affine::IDENTITY);
                    context.push_layer(Some(path), None, None);
                }
                context.set_transform(world);
                context.set_paint_transform(Affine::IDENTITY);
                context.set_paint(color(rect.color).into());
                context.fill_rect(&Rect::new(
                    rect.x0 as f64,
                    rect.y0 as f64,
                    rect.x1 as f64,
                    rect.y1 as f64,
                ));
                if clip.is_some() {
                    context.pop_layer();
                }
            }
            SceneOp::Stroke(rect) => {
                let world = transform(&scene.transforms[rect.transform_id as usize]);
                let clip = primitive_clip(rect.clip_rect, rect.clip_corner_radii);
                if let Some(path) = clip.as_ref() {
                    context.set_transform(Affine::IDENTITY);
                    context.push_layer(Some(path), None, None);
                }
                context.set_transform(world);
                context.set_paint_transform(Affine::IDENTITY);
                context.set_paint(color(rect.color).into());
                context.set_stroke(stroke_style(
                    rect.stroke_width,
                    rect.cap,
                    rect.join,
                    &rect.dash_pattern,
                    rect.dash_offset,
                ));
                let bounds = Rect::new(
                    rect.x0 as f64,
                    rect.y0 as f64,
                    rect.x1 as f64,
                    rect.y1 as f64,
                );
                let path = if rect.stroke_corner_radii.iter().any(|r| *r > 0.0) {
                    rounded_rect(bounds, rect.stroke_corner_radii)
                } else {
                    bounds.to_path(0.1)
                };
                context.stroke_path(&path);
                if clip.is_some() {
                    context.pop_layer();
                }
            }
            SceneOp::Gradient(gradient) => lower_gradient(context, gradient, scene),
            SceneOp::Shape(shape) => {
                let world = transform(&scene.transforms[shape.transform_id as usize]);
                let clip = primitive_clip(shape.clip_rect, shape.clip_corner_radii);
                if let Some(path) = clip.as_ref() {
                    context.set_transform(Affine::IDENTITY);
                    context.push_layer(Some(path), None, None);
                }
                context.set_transform(world);
                context.set_paint_transform(Affine::IDENTITY);
                let path = path(&shape.path);
                if let Some(fill) = shape.fill_color {
                    context.set_fill_rule(Fill::NonZero);
                    context.set_paint(color(fill).into());
                    context.fill_path(&path);
                }
                if let Some(stroke) = &shape.stroke {
                    context.set_paint(color(stroke.color).into());
                    context.set_stroke(path_stroke(stroke));
                    context.stroke_path(&path);
                }
                if clip.is_some() {
                    context.pop_layer();
                }
            }
            SceneOp::PushLayer(layer) => push_scene_layer(context, layer, scene),
            SceneOp::PopLayer => context.pop_layer(),
            SceneOp::Fragment(_) => {
                unreachable!("validated before lowering")
            }
        }
    }

    if has_root_layer {
        context.pop_layer();
    }
}

fn lower_gradient<C: SparseContext>(context: &mut C, grad: &SceneGradient, scene: &Scene) {
    let stops: Vec<ColorStop> = grad
        .stops
        .iter()
        .map(|stop| ColorStop::from((stop.offset, color(stop.color))))
        .collect();
    let (mut paint, paint_transform) = match grad.kind {
        GradientKind::Linear => {
            let [sx, sy, ex, ey] = grad.params;
            (
                Gradient::new_linear(
                    Point::new(sx as f64, sy as f64),
                    Point::new(ex as f64, ey as f64),
                )
                .with_stops(stops.as_slice()),
                Affine::IDENTITY,
            )
        }
        GradientKind::Radial => {
            let [cx, cy, rx, ry] = grad.params;
            if (rx - ry).abs() < 1e-3 {
                (
                    Gradient::new_radial(Point::new(cx as f64, cy as f64), rx)
                        .with_stops(stops.as_slice()),
                    Affine::IDENTITY,
                )
            } else {
                (
                    Gradient::new_radial(Point::ORIGIN, 1.0).with_stops(stops.as_slice()),
                    Affine::translate((cx as f64, cy as f64))
                        * Affine::scale_non_uniform(rx as f64, ry as f64),
                )
            }
        }
        GradientKind::Conic => {
            let [cx, cy, start, _] = grad.params;
            (
                Gradient::new_sweep(
                    Point::new(cx as f64, cy as f64),
                    start,
                    start + std::f32::consts::TAU,
                )
                .with_stops(stops.as_slice()),
                Affine::IDENTITY,
            )
        }
    };
    if grad.repeat {
        paint.extend = Extend::Repeat;
    }
    let world = transform(&scene.transforms[grad.transform_id as usize]);
    let clip = primitive_clip(grad.clip_rect, grad.clip_corner_radii);
    if let Some(path) = clip.as_ref() {
        context.set_transform(Affine::IDENTITY);
        context.push_layer(Some(path), None, None);
    }
    context.set_transform(world);
    context.set_paint_transform(paint_transform);
    context.set_paint(paint.into());
    context.fill_rect(&Rect::new(
        grad.x0 as f64,
        grad.y0 as f64,
        grad.x1 as f64,
        grad.y1 as f64,
    ));
    if clip.is_some() {
        context.pop_layer();
    }
}

fn push_scene_layer<C: SparseContext>(context: &mut C, layer: &SceneLayer, scene: &Scene) {
    context.set_transform(transform(&scene.transforms[layer.transform_id as usize]));
    let clip = match &layer.clip {
        SceneClip::None => None,
        SceneClip::Rect { rect, radii } => Some(rounded_rect(
            Rect::new(
                rect[0] as f64,
                rect[1] as f64,
                rect[2] as f64,
                rect[3] as f64,
            ),
            *radii,
        )),
        SceneClip::Path(value) => Some(path(value)),
    };
    context.push_layer(
        clip.as_ref(),
        Some(map_blend(layer.blend_mode, layer.compose)),
        Some(layer.alpha.clamp(0.0, 1.0)),
    );
}

fn primitive_clip(rect: [f32; 4], radii: [f32; 4]) -> Option<BezPath> {
    (rect != NO_CLIP).then(|| {
        rounded_rect(
            Rect::new(
                rect[0] as f64,
                rect[1] as f64,
                rect[2] as f64,
                rect[3] as f64,
            ),
            radii,
        )
    })
}

fn rounded_rect(rect: Rect, radii: [f32; 4]) -> BezPath {
    if radii.iter().any(|r| *r > 0.0) {
        RoundedRect::from_rect(
            rect,
            RoundedRectRadii::new(
                radii[0] as f64,
                radii[1] as f64,
                radii[2] as f64,
                radii[3] as f64,
            ),
        )
        .to_path(0.1)
    } else {
        rect.to_path(0.1)
    }
}

fn path(value: &ScenePath) -> BezPath {
    let mut path = BezPath::new();
    for op in &value.ops {
        match *op {
            PathOp::MoveTo(x, y) => path.move_to((x as f64, y as f64)),
            PathOp::LineTo(x, y) => path.line_to((x as f64, y as f64)),
            PathOp::QuadTo(cx, cy, x, y) => {
                path.quad_to((cx as f64, cy as f64), (x as f64, y as f64));
            }
            PathOp::CubicTo(c1x, c1y, c2x, c2y, x, y) => path.curve_to(
                (c1x as f64, c1y as f64),
                (c2x as f64, c2y as f64),
                (x as f64, y as f64),
            ),
            PathOp::Close => path.close_path(),
        }
    }
    path
}

fn path_stroke(value: &ScenePathStroke) -> Stroke {
    stroke_style(
        value.width,
        value.cap,
        value.join,
        &value.dash_pattern,
        value.dash_offset,
    )
}

fn stroke_style(
    width: f32,
    cap: SceneStrokeCap,
    join: SceneStrokeJoin,
    dash_pattern: &[f32],
    dash_offset: f32,
) -> Stroke {
    let cap = match cap {
        SceneStrokeCap::Butt => Cap::Butt,
        SceneStrokeCap::Round => Cap::Round,
        SceneStrokeCap::Square => Cap::Square,
    };
    let join = match join {
        SceneStrokeJoin::Bevel => Join::Bevel,
        SceneStrokeJoin::Miter => Join::Miter,
        SceneStrokeJoin::Round => Join::Round,
    };
    let mut stroke = Stroke::new(width as f64).with_caps(cap).with_join(join);
    if !dash_pattern.is_empty() {
        stroke = stroke.with_dashes(
            dash_offset as f64,
            dash_pattern.iter().map(|value| *value as f64),
        );
    }
    stroke
}

fn transform(value: &Transform) -> Affine {
    Affine::new([
        value.m[0] as f64,
        value.m[1] as f64,
        value.m[4] as f64,
        value.m[5] as f64,
        value.m[12] as f64,
        value.m[13] as f64,
    ])
}

fn color(value: [f32; 4]) -> Color {
    let alpha = value[3];
    if alpha > 0.0 {
        Color::from_rgba8(
            (value[0] / alpha * 255.0).round().clamp(0.0, 255.0) as u8,
            (value[1] / alpha * 255.0).round().clamp(0.0, 255.0) as u8,
            (value[2] / alpha * 255.0).round().clamp(0.0, 255.0) as u8,
            (alpha * 255.0).round().clamp(0.0, 255.0) as u8,
        )
    } else {
        Color::TRANSPARENT
    }
}

fn map_blend(blend: SceneBlendMode, compose: SceneCompose) -> BlendMode {
    let mix = match blend {
        SceneBlendMode::Normal => Mix::Normal,
        SceneBlendMode::Multiply => Mix::Multiply,
        SceneBlendMode::Screen => Mix::Screen,
        SceneBlendMode::Overlay => Mix::Overlay,
        SceneBlendMode::Darken => Mix::Darken,
        SceneBlendMode::Lighten => Mix::Lighten,
    };
    let compose = match compose {
        SceneCompose::SrcOver => Compose::SrcOver,
        SceneCompose::DestIn => Compose::DestIn,
        SceneCompose::Plus => Compose::Plus,
    };
    BlendMode::new(mix, compose)
}
