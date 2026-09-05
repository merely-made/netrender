// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Experimental admission boundary for Vello Classic, Hybrid, and CPU.
//!
//! [`crate::scene::Scene`] remains the authoritative display-list model. Classic
//! continues to use [`crate::vello_rasterizer`]; the opt-in sparse backends use
//! one shared lowerer so their common behavior cannot drift silently.

/// A named realization of Netrender's scene contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VelloBackend {
    /// Vello's compute-heavy GPU renderer. This is Netrender's shipping path.
    Classic,
    /// CPU path processing with GPU rasterization and compositing.
    Hybrid,
    /// Pure CPU sparse-strips rasterization.
    Cpu,
}

/// Operations admitted by Netrender's current adapter, rather than every
/// operation the upstream renderer may eventually support.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BackendCapabilities {
    pub solid_geometry: bool,
    pub gradients: bool,
    pub layers: bool,
    pub transforms: bool,
    pub clips: bool,
    pub nested_layers: bool,
    pub images: bool,
    pub patterns: bool,
    pub text: bool,
    pub element_filters: bool,
    pub backdrop_blur: bool,
    pub backdrop_color_filters: bool,
    /// The backend packet has a native composition operation.
    pub native_scene_append: bool,
    /// Netrender's registry-bearing renderer is connected to that operation.
    /// The free Classic `scene_to_vello` translator cannot resolve fragments.
    pub retained_fragments: bool,
    pub gpu_output: bool,
    pub cpu_output: bool,
}

impl VelloBackend {
    /// Whether this backend's adapter was included in the current build.
    pub const fn is_compiled(self) -> bool {
        match self {
            Self::Classic => true,
            Self::Hybrid => cfg!(feature = "vello-hybrid"),
            Self::Cpu => cfg!(feature = "vello-cpu"),
        }
    }

    pub const fn capabilities(self) -> BackendCapabilities {
        match self {
            Self::Classic => BackendCapabilities {
                solid_geometry: true,
                gradients: true,
                layers: true,
                transforms: true,
                clips: true,
                nested_layers: true,
                images: true,
                patterns: true,
                text: true,
                element_filters: true,
                backdrop_blur: true,
                backdrop_color_filters: false,
                native_scene_append: true,
                retained_fragments: true,
                gpu_output: true,
                cpu_output: false,
            },
            Self::Hybrid => BackendCapabilities {
                solid_geometry: true,
                gradients: true,
                layers: true,
                transforms: true,
                clips: true,
                nested_layers: true,
                images: false,
                patterns: false,
                text: false,
                element_filters: false,
                backdrop_blur: false,
                backdrop_color_filters: false,
                native_scene_append: true,
                retained_fragments: false,
                gpu_output: true,
                cpu_output: false,
            },
            Self::Cpu => BackendCapabilities {
                solid_geometry: true,
                gradients: true,
                layers: true,
                transforms: true,
                clips: true,
                nested_layers: true,
                images: false,
                patterns: false,
                text: false,
                element_filters: false,
                backdrop_blur: false,
                backdrop_color_filters: false,
                native_scene_append: false,
                retained_fragments: false,
                gpu_output: false,
                cpu_output: true,
            },
        }
    }
}

/// A scene cannot be lowered faithfully by the selected backend adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackendAdmissionError {
    /// Sparse scenes use `u16` dimensions internally.
    ViewportTooLarge {
        backend: VelloBackend,
        width: u32,
        height: u32,
    },
    /// An operation is outside the adapter's currently verified subset.
    UnsupportedOperation {
        backend: VelloBackend,
        op_index: usize,
        operation: &'static str,
        reason: &'static str,
    },
    /// A primitive references a transform absent from the scene palette.
    InvalidTransform {
        backend: VelloBackend,
        op_index: usize,
        transform_id: u32,
    },
    /// Layer pushes and pops are not balanced.
    UnbalancedLayers {
        backend: VelloBackend,
        op_index: Option<usize>,
    },
}

impl std::fmt::Display for BackendAdmissionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ViewportTooLarge {
                backend,
                width,
                height,
            } => write!(
                f,
                "{backend:?} sparse Vello viewport {width}x{height} exceeds the u16 dimension limit"
            ),
            Self::UnsupportedOperation {
                backend,
                op_index,
                operation,
                reason,
            } => write!(
                f,
                "{backend:?} cannot admit scene op {op_index} ({operation}): {reason}"
            ),
            Self::InvalidTransform {
                backend,
                op_index,
                transform_id,
            } => write!(
                f,
                "{backend:?} scene op {op_index} references missing transform {transform_id}"
            ),
            Self::UnbalancedLayers { backend, op_index } => match op_index {
                Some(op_index) => write!(
                    f,
                    "{backend:?} scene layers are unbalanced at op {op_index}"
                ),
                None => write!(f, "{backend:?} scene layers are unbalanced at end of scene"),
            },
        }
    }
}

impl std::error::Error for BackendAdmissionError {}

/// Validate the operation-level scene semantics supported by `backend`.
///
/// This checks viewport shape, operation kinds, transform references, and
/// layer balance. It deliberately cannot prove Classic registry state for
/// external images or retained fragments; resource-bearing Classic scenes
/// still require the registry-bearing [`crate::Renderer`] path. Callers choose
/// the backend-specific lowering and execution path after this preflight.
pub fn validate_scene_for_backend(
    backend: VelloBackend,
    scene: &crate::scene::Scene,
) -> Result<(), BackendAdmissionError> {
    use crate::scene::{SceneFilter, SceneOp};

    if backend != VelloBackend::Classic
        && (scene.viewport_width > u16::MAX as u32 || scene.viewport_height > u16::MAX as u32)
    {
        return Err(BackendAdmissionError::ViewportTooLarge {
            backend,
            width: scene.viewport_width,
            height: scene.viewport_height,
        });
    }

    let mut depth = 0usize;
    for (op_index, op) in scene.ops.iter().enumerate() {
        let transform_id = match op {
            SceneOp::Rect(v) => Some(v.transform_id),
            SceneOp::Stroke(v) => Some(v.transform_id),
            SceneOp::Gradient(v) => Some(v.transform_id),
            SceneOp::Image(v) => Some(v.transform_id),
            SceneOp::Pattern(v) => Some(v.transform_id),
            SceneOp::Shape(v) => Some(v.transform_id),
            SceneOp::GlyphRun(v) => Some(v.transform_id),
            SceneOp::PushLayer(v) => Some(v.transform_id),
            SceneOp::Fragment(v) => Some(v.transform_id),
            SceneOp::PopLayer => None,
        };
        if transform_id.is_some_and(|id| id as usize >= scene.transforms.len()) {
            return Err(BackendAdmissionError::InvalidTransform {
                backend,
                op_index,
                transform_id: transform_id.expect("checked Some transform"),
            });
        }

        match op {
            SceneOp::Image(_) if backend != VelloBackend::Classic => {
                return Err(unsupported(
                    backend,
                    op_index,
                    "Image",
                    "sparse image hydration is not wired yet",
                ));
            }
            SceneOp::Pattern(_) if backend != VelloBackend::Classic => {
                return Err(unsupported(
                    backend,
                    op_index,
                    "Pattern",
                    "sparse image hydration is not wired yet",
                ));
            }
            SceneOp::GlyphRun(_) if backend != VelloBackend::Classic => {
                return Err(unsupported(
                    backend,
                    op_index,
                    "GlyphRun",
                    "sparse text resources are not wired yet",
                ));
            }
            SceneOp::Fragment(_) if backend != VelloBackend::Classic => {
                return Err(unsupported(
                    backend,
                    op_index,
                    "Fragment",
                    "fragment registries are backend-owned; only Hybrid append has been proven upstream",
                ));
            }
            SceneOp::PushLayer(layer)
                if backend != VelloBackend::Classic
                    && (layer.backdrop_filter.is_some() || !layer.filters.is_empty()) =>
            {
                return Err(unsupported(
                    backend,
                    op_index,
                    "PushLayer",
                    "Netrender filter graphs have not been mapped to sparse filters",
                ));
            }
            SceneOp::PushLayer(layer)
                if backend == VelloBackend::Classic
                    && layer
                        .backdrop_filter
                        .is_some_and(|filter| !matches!(filter, SceneFilter::Blur(_))) =>
            {
                return Err(unsupported(
                    backend,
                    op_index,
                    "PushLayer",
                    "Classic backdrop preprocessing currently supports Blur only",
                ));
            }
            SceneOp::PushLayer(_) => depth += 1,
            SceneOp::PopLayer if depth == 0 => {
                return Err(BackendAdmissionError::UnbalancedLayers {
                    backend,
                    op_index: Some(op_index),
                });
            }
            SceneOp::PopLayer => depth -= 1,
            _ => {}
        }
    }
    if depth != 0 {
        return Err(BackendAdmissionError::UnbalancedLayers {
            backend,
            op_index: None,
        });
    }
    Ok(())
}

fn unsupported(
    backend: VelloBackend,
    op_index: usize,
    operation: &'static str,
    reason: &'static str,
) -> BackendAdmissionError {
    BackendAdmissionError::UnsupportedOperation {
        backend,
        op_index,
        operation,
        reason,
    }
}

#[cfg(any(feature = "vello-cpu", feature = "vello-hybrid"))]
mod sparse {
    use super::{BackendAdmissionError, VelloBackend};
    use crate::scene::{
        GradientKind, NO_CLIP, PathOp, Scene, SceneBlendMode, SceneClip, SceneCompose,
        SceneGradient, SceneLayer, SceneOp, ScenePath, ScenePathStroke, SceneStrokeCap,
        SceneStrokeJoin, Transform,
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
        fn push_layer(
            &mut self,
            clip: Option<&BezPath>,
            blend: Option<BlendMode>,
            alpha: Option<f32>,
        );
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
        fn push_layer(
            &mut self,
            clip: Option<&BezPath>,
            blend: Option<BlendMode>,
            alpha: Option<f32>,
        ) {
            self.push_layer(clip, blend, alpha, None, None);
        }
        fn pop_layer(&mut self) {
            self.pop_layer();
        }
    }

    #[cfg(feature = "vello-hybrid")]
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
        fn push_layer(
            &mut self,
            clip: Option<&BezPath>,
            blend: Option<BlendMode>,
            alpha: Option<f32>,
        ) {
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
        let width = u16::try_from(scene.viewport_width).map_err(|_| {
            BackendAdmissionError::ViewportTooLarge {
                backend,
                width: scene.viewport_width,
                height: scene.viewport_height,
            }
        })?;
        let height = u16::try_from(scene.viewport_height).map_err(|_| {
            BackendAdmissionError::ViewportTooLarge {
                backend,
                width: scene.viewport_width,
                height: scene.viewport_height,
            }
        })?;
        Ok((width, height))
    }

    pub(super) fn lower<C: SparseContext>(
        context: &mut C,
        scene: &Scene,
        backend: VelloBackend,
    ) -> Result<(), BackendAdmissionError> {
        super::validate_scene_for_backend(backend, scene)?;

        let has_root_layer =
            scene.root_alpha != 1.0 || scene.root_blend_mode != SceneBlendMode::Normal;
        if has_root_layer {
            context.set_transform(Affine::IDENTITY);
            context.push_layer(
                None,
                Some(map_blend(scene.root_blend_mode, SceneCompose::SrcOver)),
                Some(scene.root_alpha.clamp(0.0, 1.0)),
            );
        }

        for op in &scene.ops {
            match op {
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
                SceneOp::Image(_)
                | SceneOp::Pattern(_)
                | SceneOp::GlyphRun(_)
                | SceneOp::Fragment(_) => unreachable!("validated before lowering"),
            }
        }

        if has_root_layer {
            context.pop_layer();
        }
        Ok(())
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
        };
        BlendMode::new(mix, compose)
    }
}

/// Lower a Netrender scene into Vello CPU's stateful render context.
#[cfg(feature = "vello-cpu")]
pub fn scene_to_vello_cpu(
    scene: &crate::scene::Scene,
) -> Result<vello_cpu::RenderContext, BackendAdmissionError> {
    let (width, height) = sparse::dimensions(scene, VelloBackend::Cpu)?;
    let mut context = vello_cpu::RenderContext::new(width, height);
    sparse::lower(&mut context, scene, VelloBackend::Cpu)?;
    context.flush();
    Ok(context)
}

/// Lower a Netrender scene into Vello Hybrid's sparse scene packet.
#[cfg(feature = "vello-hybrid")]
pub fn scene_to_vello_hybrid(
    scene: &crate::scene::Scene,
) -> Result<vello_hybrid::Scene, BackendAdmissionError> {
    let (width, height) = sparse::dimensions(scene, VelloBackend::Hybrid)?;
    let mut context = vello_hybrid::Scene::new(width, height);
    sparse::lower(&mut context, scene, VelloBackend::Hybrid)?;
    Ok(context)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "vello-all")]
    use crate::scene::ImageData;
    #[cfg(any(feature = "vello-cpu", feature = "vello-hybrid"))]
    use crate::scene::Scene;

    #[test]
    fn capabilities_keep_one_contract_and_three_realizations() {
        assert!(VelloBackend::Classic.is_compiled());
        assert_eq!(
            VelloBackend::Hybrid.is_compiled(),
            cfg!(feature = "vello-hybrid")
        );
        assert_eq!(VelloBackend::Cpu.is_compiled(), cfg!(feature = "vello-cpu"));
        assert!(VelloBackend::Classic.capabilities().retained_fragments);
        assert!(VelloBackend::Hybrid.capabilities().native_scene_append);
        assert!(!VelloBackend::Hybrid.capabilities().retained_fragments);
        assert!(VelloBackend::Cpu.capabilities().cpu_output);
        assert!(!VelloBackend::Cpu.capabilities().gpu_output);
        for backend in [
            VelloBackend::Classic,
            VelloBackend::Hybrid,
            VelloBackend::Cpu,
        ] {
            let capabilities = backend.capabilities();
            assert!(capabilities.transforms);
            assert!(capabilities.clips);
            assert!(capabilities.nested_layers);
        }
    }

    #[cfg(feature = "vello-cpu")]
    #[test]
    fn cpu_lowerer_rasterizes_a_netrender_scene() {
        let mut scene = Scene::new(8, 6);
        scene.push_rect(2.0, 1.0, 6.0, 5.0, [1.0, 0.0, 1.0, 1.0]);
        let context = scene_to_vello_cpu(&scene).unwrap();
        let mut target = vello_cpu::Pixmap::new(8, 6);
        let mut resources = vello_cpu::Resources::new();
        context.render(&mut target, &mut resources);

        let center = target.data()[3 * 8 + 3];
        assert_eq!((center.r, center.g, center.b, center.a), (255, 0, 255, 255));
        let corner = target.data()[0];
        assert_eq!((corner.r, corner.g, corner.b, corner.a), (0, 0, 0, 0));
    }

    #[cfg(feature = "vello-hybrid")]
    #[test]
    fn hybrid_lowerer_produces_an_appendable_scene() {
        let mut source = Scene::new(32, 32);
        source.push_rect(0.0, 0.0, 8.0, 8.0, [0.0, 1.0, 0.0, 1.0]);
        let donor = scene_to_vello_hybrid(&source).unwrap();
        let mut master = vello_hybrid::Scene::new(32, 32);
        master.append_scene(donor, Some((16, 16))).unwrap();
    }

    #[cfg(all(feature = "vello-hybrid", not(target_arch = "wasm32")))]
    #[test]
    fn hybrid_gpu_renders_on_netrenders_wgpu_device() {
        let handles = netrender_device::boot().expect("wgpu device");
        let mut source = Scene::new(8, 8);
        source.push_rect(0.0, 0.0, 8.0, 8.0, [1.0, 0.0, 1.0, 1.0]);
        let scene = scene_to_vello_hybrid(&source).unwrap();

        let texture = handles.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Netrender Vello Hybrid proof target"),
            size: wgpu::Extent3d {
                width: 8,
                height: 8,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let render_size = vello_hybrid::RenderSize {
            width: 8,
            height: 8,
        };
        let (mut renderer, mut resources) = vello_hybrid::Renderer::new(
            &handles.device,
            &vello_hybrid::RenderTargetConfig {
                format: wgpu::TextureFormat::Rgba8Unorm,
                width: 8,
                height: 8,
            },
        );
        let depth =
            vello_hybrid::Renderer::create_depth_texture_view(&handles.device, &render_size);
        let mut encoder = handles
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Netrender Vello Hybrid proof encoder"),
            });
        renderer
            .render(
                &scene,
                &mut resources,
                &handles.device,
                &handles.queue,
                &mut encoder,
                &render_size,
                &view,
                Some(&depth),
                &vello_hybrid::TextureBindings::new(),
            )
            .unwrap();

        let bytes_per_row = 256;
        let buffer = handles.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Netrender Vello Hybrid proof readback"),
            size: bytes_per_row * 8,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(bytes_per_row as u32),
                    rows_per_image: Some(8),
                },
            },
            wgpu::Extent3d {
                width: 8,
                height: 8,
                depth_or_array_layers: 1,
            },
        );
        handles.queue.submit([encoder.finish()]);

        let slice = buffer.slice(..);
        let (sender, receiver) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            sender.send(result).unwrap();
        });
        handles
            .device
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
        receiver.recv().unwrap().unwrap();
        let mapped = slice.get_mapped_range().unwrap();
        let center = 4 * bytes_per_row as usize + 4 * 4;
        assert_eq!(&mapped[center..center + 4], &[255, 0, 255, 255]);
    }

    #[cfg(feature = "vello-all")]
    #[test]
    fn sparse_backends_refuse_unwired_images_identically() {
        let mut scene = Scene::new(2, 2);
        scene.push_image(
            0.0,
            0.0,
            2.0,
            2.0,
            7,
            ImageData::from_bytes(1, 1, vec![255, 255, 255, 255]),
        );
        let cpu = scene_to_vello_cpu(&scene).unwrap_err();
        let hybrid = scene_to_vello_hybrid(&scene).unwrap_err();
        assert!(matches!(
            cpu,
            BackendAdmissionError::UnsupportedOperation {
                backend: VelloBackend::Cpu,
                operation: "Image",
                ..
            }
        ));
        assert!(matches!(
            hybrid,
            BackendAdmissionError::UnsupportedOperation {
                backend: VelloBackend::Hybrid,
                operation: "Image",
                ..
            }
        ));
    }

    #[cfg(feature = "vello-all")]
    #[test]
    fn sparse_backends_admit_the_same_gradient_layer_subset() {
        let mut scene = Scene::new(32, 32);
        scene.push_layer_alpha(0.75);
        scene.push_linear_gradient(
            0.0,
            0.0,
            32.0,
            32.0,
            [0.0, 0.0],
            [32.0, 0.0],
            [1.0, 0.0, 0.0, 1.0],
            [0.0, 0.0, 1.0, 1.0],
        );
        scene.pop_layer();

        scene_to_vello_cpu(&scene).unwrap();
        scene_to_vello_hybrid(&scene).unwrap();
    }
}
