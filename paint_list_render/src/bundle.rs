// Copyright 2026 Mark Alan Boykin
// SPDX-License-Identifier: MPL-2.0

//! Portable, complete producer resources. Admission is pure: it validates the
//! intact envelope, source snapshots, translated scene and every side channel
//! before constructing a new image-bearing scene. It never updates a renderer
//! cache and never drops an unavailable command or substitutes a background.

use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    sync::Arc,
};
use netrender::{Scene, SceneOp, ImageData, external_image_key};
use paint_list_api::{PaintCmd, PaintEnvelope};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use skrifa::raw::TableProvider;

pub const BUNDLE_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ResourceAlpha {
    Straight,
    Premultiplied,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ResourceEncoding {
    Srgb,
    LinearSrgb,
}

/// The output of the actual bound producer callback. Identity and revision are
/// producer metadata, independent of GPU textures and renderer allocation IDs.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProducerSource {
    pub texture_key: u64,
    pub content_generation: u64,
    pub source_identity: u64,
    pub source_revision: u64,
    pub logical_size: [f32; 2],
    pub physical_size: [u32; 2],
    pub layout_scale: f32,
    pub alpha: ResourceAlpha,
    pub encoding: ResourceEncoding,
    pub scene: Scene,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PortablePaintCapture {
    pub envelope: PaintEnvelope,
    pub producers: Vec<ProducerSource>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExternalResourceDraw {
    pub texture_key: u64,
    pub content_generation: Option<u64>,
    pub image_key: u64,
    pub dest_rect: [f32; 4],
    pub uv: [f32; 4],
    pub opacity: f32,
    pub scene_op_boundary: usize,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ShadowResourceRequest {
    pub key: u64,
    pub dim: u32,
    pub bounds: [f32; 4],
    pub corner_radius: f32,
    pub blur_radius_px: f32,
    pub invert: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TranslatedResourceScene {
    pub scene: Scene,
    pub external_textures: Vec<ExternalResourceDraw>,
    pub box_shadow_masks: Vec<ShadowResourceRequest>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ResourceRef {
    ExternalTexture(u64),
    ShadowMask(u64),
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ResourceBinding {
    Producer {
        content_generation: u64,
        source_identity: u64,
        source_revision: u64,
        logical_size: [f32; 2],
        physical_size: [u32; 2],
        layout_scale: f32,
        source_scene_digest: [u8; 32],
    },
    Shadow {
        request_digest: [u8; 32],
    },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ResourcePayload {
    pub resource: ResourceRef,
    pub binding: ResourceBinding,
    pub width: u32,
    pub height: u32,
    pub alpha: ResourceAlpha,
    pub encoding: ResourceEncoding,
    pub rgba: Vec<u8>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CompleteResourceBundle {
    pub version: u32,
    pub capture_digest: [u8; 32],
    pub capture: PortablePaintCapture,
    pub translated: TranslatedResourceScene,
    pub resources: Vec<ResourcePayload>,
}
#[derive(Clone, Copy, Debug)]
pub struct BundleLimits {
    pub max_sources: usize,
    pub max_resources: usize,
    pub max_bytes: usize,
    pub max_ops: usize,
    pub max_depth: usize,
    pub max_dimension: u32,
    pub max_pixels: usize,
}
impl Default for BundleLimits {
    fn default() -> Self {
        Self {
            max_sources: 32,
            max_resources: 64,
            max_bytes: 256 * 1024 * 1024,
            max_ops: 100_000,
            max_depth: 64,
            max_dimension: 16_384,
            max_pixels: 16 * 1024 * 1024,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BundleError {
    Version(u32),
    Budget {
        resource: &'static str,
        requested: usize,
        limit: usize,
    },
    InvalidEnvelope {
        command: Option<usize>,
        reason: &'static str,
    },
    InvalidScene {
        reason: &'static str,
    },
    DuplicateProducer(u64),
    MissingProducer(u64),
    ForeignProducer(u64),
    StaleProducer(u64),
    InvalidProducer {
        key: u64,
        reason: &'static str,
    },
    DuplicateResource(ResourceRef),
    MissingResource(ResourceRef),
    ForeignResource(ResourceRef),
    StaleResource(ResourceRef),
    InvalidPayload {
        resource: ResourceRef,
        reason: &'static str,
    },
    CaptureIdentity,
    TranslationIdentity,
    SideChannelIdentity,
    Serialization(String),
}
impl std::fmt::Display for BundleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "resource bundle: {self:?}")
    }
}
impl std::error::Error for BundleError {}
fn serialized<T: Serialize>(value: &T) -> Result<Vec<u8>, BundleError> {
    postcard::to_allocvec(value).map_err(|e| BundleError::Serialization(e.to_string()))
}
fn digest<T: Serialize>(value: &T) -> Result<[u8; 32], BundleError> {
    Ok(Sha256::digest(serialized(value)?).into())
}
fn bound(resource: &'static str, requested: usize, limit: usize) -> Result<(), BundleError> {
    if requested > limit {
        Err(BundleError::Budget {
            resource,
            requested,
            limit,
        })
    } else {
        Ok(())
    }
}
fn add(total: &mut usize, value: usize, limits: BundleLimits) -> Result<(), BundleError> {
    *total = total.checked_add(value).ok_or(BundleError::Budget {
        resource: "bytes",
        requested: usize::MAX,
        limit: limits.max_bytes,
    })?;
    bound("bytes", *total, limits.max_bytes)
}
fn extent(width: u32, height: u32, limits: BundleLimits) -> Result<usize, BundleError> {
    if width == 0 || height == 0 || width > limits.max_dimension || height > limits.max_dimension {
        return Err(BundleError::InvalidScene {
            reason: "nonzero extent exceeds dimension contract",
        });
    }
    let pixels =
        (width as usize)
            .checked_mul(height as usize)
            .ok_or(BundleError::InvalidScene {
                reason: "extent overflow",
            })?;
    bound("viewport pixels", pixels, limits.max_pixels)?;
    pixels.checked_mul(4).ok_or(BundleError::InvalidScene {
        reason: "byte extent overflow",
    })
}

/// Blob allocation IDs are cache mechanics, not portable scene meaning.
/// Normalize only those IDs; painter order, fonts, pixels and geometry remain.
pub fn scene_digest(scene: &Scene) -> Result<[u8; 32], BundleError> {
    let mut canonical = scene.clone();
    for (i, font) in canonical.fonts.iter_mut().enumerate() {
        font.data = netrender::peniko::Blob::from_raw_parts(
            Arc::new(font.data.as_ref().to_vec()),
            i as u64,
        );
    }
    for (&key, image) in &mut canonical.image_sources {
        image.data =
            netrender::peniko::Blob::from_raw_parts(Arc::new(image.data.as_ref().to_vec()), key);
    }
    digest(&canonical)
}
pub fn capture_digest(capture: &PortablePaintCapture) -> Result<[u8; 32], BundleError> {
    let sources: Vec<_> = capture
        .producers
        .iter()
        .map(|s| {
            Ok((
                s.texture_key,
                s.content_generation,
                s.source_identity,
                s.source_revision,
                s.logical_size,
                s.physical_size,
                s.layout_scale,
                s.alpha,
                s.encoding,
                scene_digest(&s.scene)?,
            ))
        })
        .collect::<Result<_, BundleError>>()?;
    digest(&(digest(&capture.envelope)?, sources))
}

// Traverse serde values directly: Option::None is valid, while NaN/infinity
// are refused at serialize_f32/f64. No JSON allocation or field-name guess.
impl serde::ser::Error for BundleError {
    fn custom<T: std::fmt::Display>(message: T) -> Self {
        Self::Serialization(message.to_string())
    }
}
struct FiniteVisitor {
    values: usize,
    bytes: usize,
    limits: BundleLimits,
}
impl FiniteVisitor {
    fn value(&mut self) -> Result<(), BundleError> {
        self.values = self.values.checked_add(1).unwrap_or(usize::MAX);
        bound(
            "graphics values",
            self.values,
            self.limits.max_ops.saturating_mul(64),
        )
    }
    fn float(&mut self, v: f64) -> Result<(), BundleError> {
        self.value()?;
        if v.is_finite() {
            Ok(())
        } else {
            Err(BundleError::InvalidScene {
                reason: "non-finite graphics value",
            })
        }
    }
    fn text(&mut self, bytes: usize) -> Result<(), BundleError> {
        self.value()?;
        add(&mut self.bytes, bytes, self.limits)
    }
}
impl<'a> serde::Serializer for &'a mut FiniteVisitor {
    type Ok = ();
    type Error = BundleError;
    type SerializeSeq = Self;
    type SerializeTuple = Self;
    type SerializeTupleStruct = Self;
    type SerializeTupleVariant = Self;
    type SerializeMap = Self;
    type SerializeStruct = Self;
    type SerializeStructVariant = Self;
    fn serialize_bool(self, _: bool) -> Result<(), BundleError> {
        self.value()
    }
    fn serialize_i8(self, _: i8) -> Result<(), BundleError> {
        self.value()
    }
    fn serialize_i16(self, _: i16) -> Result<(), BundleError> {
        self.value()
    }
    fn serialize_i32(self, _: i32) -> Result<(), BundleError> {
        self.value()
    }
    fn serialize_i64(self, _: i64) -> Result<(), BundleError> {
        self.value()
    }
    fn serialize_u8(self, _: u8) -> Result<(), BundleError> {
        self.value()
    }
    fn serialize_u16(self, _: u16) -> Result<(), BundleError> {
        self.value()
    }
    fn serialize_u32(self, _: u32) -> Result<(), BundleError> {
        self.value()
    }
    fn serialize_u64(self, _: u64) -> Result<(), BundleError> {
        self.value()
    }
    fn serialize_f32(self, v: f32) -> Result<(), BundleError> {
        self.float(f64::from(v))
    }
    fn serialize_f64(self, v: f64) -> Result<(), BundleError> {
        self.float(v)
    }
    fn serialize_char(self, _: char) -> Result<(), BundleError> {
        self.value()
    }
    fn serialize_str(self, v: &str) -> Result<(), BundleError> {
        self.text(v.len())
    }
    fn serialize_bytes(self, v: &[u8]) -> Result<(), BundleError> {
        self.text(v.len())
    }
    fn serialize_none(self) -> Result<(), BundleError> {
        self.value()
    }
    fn serialize_some<T: Serialize + ?Sized>(self, v: &T) -> Result<(), BundleError> {
        v.serialize(self)
    }
    fn serialize_unit(self) -> Result<(), BundleError> {
        self.value()
    }
    fn serialize_unit_struct(self, _: &'static str) -> Result<(), BundleError> {
        self.value()
    }
    fn serialize_unit_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
    ) -> Result<(), BundleError> {
        self.value()
    }
    fn serialize_newtype_struct<T: Serialize + ?Sized>(
        self,
        _: &'static str,
        v: &T,
    ) -> Result<(), BundleError> {
        v.serialize(self)
    }
    fn serialize_newtype_variant<T: Serialize + ?Sized>(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        v: &T,
    ) -> Result<(), BundleError> {
        v.serialize(self)
    }
    fn serialize_seq(self, _: Option<usize>) -> Result<Self, BundleError> {
        self.value()?;
        Ok(self)
    }
    fn serialize_tuple(self, _: usize) -> Result<Self, BundleError> {
        self.value()?;
        Ok(self)
    }
    fn serialize_tuple_struct(self, _: &'static str, _: usize) -> Result<Self, BundleError> {
        self.value()?;
        Ok(self)
    }
    fn serialize_tuple_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: usize,
    ) -> Result<Self, BundleError> {
        self.value()?;
        Ok(self)
    }
    fn serialize_map(self, _: Option<usize>) -> Result<Self, BundleError> {
        self.value()?;
        Ok(self)
    }
    fn serialize_struct(self, _: &'static str, _: usize) -> Result<Self, BundleError> {
        self.value()?;
        Ok(self)
    }
    fn serialize_struct_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: usize,
    ) -> Result<Self, BundleError> {
        self.value()?;
        Ok(self)
    }
}
macro_rules! finite_compound {
    ($trait:ident,$method:ident) => {
        impl serde::ser::$trait for &mut FiniteVisitor {
            type Ok = ();
            type Error = BundleError;
            fn $method<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), BundleError> {
                value.serialize(&mut **self)
            }
            fn end(self) -> Result<(), BundleError> {
                Ok(())
            }
        }
    };
}
finite_compound!(SerializeSeq, serialize_element);
finite_compound!(SerializeTuple, serialize_element);
finite_compound!(SerializeTupleStruct, serialize_field);
finite_compound!(SerializeTupleVariant, serialize_field);
impl serde::ser::SerializeMap for &mut FiniteVisitor {
    type Ok = ();
    type Error = BundleError;
    fn serialize_key<T: Serialize + ?Sized>(&mut self, v: &T) -> Result<(), BundleError> {
        v.serialize(&mut **self)
    }
    fn serialize_value<T: Serialize + ?Sized>(&mut self, v: &T) -> Result<(), BundleError> {
        v.serialize(&mut **self)
    }
    fn end(self) -> Result<(), BundleError> {
        Ok(())
    }
}
impl serde::ser::SerializeStruct for &mut FiniteVisitor {
    type Ok = ();
    type Error = BundleError;
    fn serialize_field<T: Serialize + ?Sized>(
        &mut self,
        _: &'static str,
        v: &T,
    ) -> Result<(), BundleError> {
        v.serialize(&mut **self)
    }
    fn end(self) -> Result<(), BundleError> {
        Ok(())
    }
}
impl serde::ser::SerializeStructVariant for &mut FiniteVisitor {
    type Ok = ();
    type Error = BundleError;
    fn serialize_field<T: Serialize + ?Sized>(
        &mut self,
        _: &'static str,
        v: &T,
    ) -> Result<(), BundleError> {
        v.serialize(&mut **self)
    }
    fn end(self) -> Result<(), BundleError> {
        Ok(())
    }
}
fn finite_graphics<T: Serialize>(
    value: &T,
    scene: bool,
    limits: BundleLimits,
) -> Result<(), BundleError> {
    value
        .serialize(&mut FiniteVisitor {
            values: 0,
            bytes: 0,
            limits,
        })
        .map_err(|e| {
            if !scene && matches!(e, BundleError::InvalidScene { .. }) {
                BundleError::InvalidEnvelope {
                    command: None,
                    reason: "non-finite graphics value",
                }
            } else {
                e
            }
        })
}

fn work_add(total: &mut usize, count: usize, limits: BundleLimits) -> Result<(), BundleError> {
    *total = total.checked_add(count).ok_or(BundleError::Budget {
        resource: "aggregate structure",
        requested: usize::MAX,
        limit: limits.max_ops,
    })?;
    bound("aggregate structure", *total, limits.max_ops)
}
fn scene_work(scene: &Scene, limits: BundleLimits) -> Result<usize, BundleError> {
    let mut total = 0;
    for n in [
        scene.ops.len(),
        scene.transforms.len(),
        scene.fonts.len(),
        scene.image_sources.len(),
    ] {
        work_add(&mut total, n, limits)?;
    }
    for op in &scene.ops {
        match op {
            SceneOp::Shape(v) => {
                work_add(&mut total, v.path.ops.len(), limits)?;
                if let Some(st) = &v.stroke {
                    work_add(&mut total, st.dash_pattern.len(), limits)?;
                }
            }
            SceneOp::Stroke(v) => work_add(&mut total, v.dash_pattern.len(), limits)?,
            SceneOp::Gradient(v) => work_add(&mut total, v.stops.len(), limits)?,
            SceneOp::GlyphRun(v) => {
                work_add(&mut total, v.glyphs.len(), limits)?;
                work_add(&mut total, v.font_axis_values.len(), limits)?;
            }
            SceneOp::PushLayer(v) => {
                work_add(&mut total, v.filters.len(), limits)?;
                if let netrender::SceneClip::Path(path) = &v.clip {
                    work_add(&mut total, path.ops.len(), limits)?;
                }
            }
            _ => {}
        }
    }
    Ok(total)
}
fn capture_work(
    capture: &PortablePaintCapture,
    limits: BundleLimits,
) -> Result<usize, BundleError> {
    let mut total = 0;
    for n in [
        capture.envelope.commands.len(),
        capture.envelope.fonts.len(),
        capture.envelope.images.len(),
        capture.producers.len(),
    ] {
        work_add(&mut total, n, limits)?;
    }
    for cmd in &capture.envelope.commands {
        match cmd {
            PaintCmd::DrawStroke(v) => {
                work_add(&mut total, v.path.commands.len(), limits)?;
                if let Some(d) = &v.dash {
                    work_add(&mut total, d.intervals.len(), limits)?;
                }
            }
            PaintCmd::DrawPath(v) => {
                work_add(&mut total, v.path.commands.len(), limits)?;
                if let Some(st) = &v.stroke {
                    if let Some(d) = &st.dash {
                        work_add(&mut total, d.intervals.len(), limits)?;
                    }
                }
            }
            PaintCmd::DrawLinearGradient(v) => {
                work_add(&mut total, v.gradient.stops.len(), limits)?
            }
            PaintCmd::DrawRadialGradient(v) => {
                work_add(&mut total, v.gradient.stops.len(), limits)?
            }
            PaintCmd::DrawConicGradient(v) => work_add(&mut total, v.gradient.stops.len(), limits)?,
            PaintCmd::DrawText(v) => work_add(&mut total, v.glyphs.len(), limits)?,
            PaintCmd::PushLayer(v) => work_add(&mut total, v.filters.len(), limits)?,
            PaintCmd::PushClip(v) => {
                if let paint_list_api::ClipKind::Path(path) = &v.kind {
                    work_add(&mut total, path.commands.len(), limits)?;
                }
            }
            _ => {}
        }
    }
    for source in &capture.producers {
        work_add(&mut total, scene_work(&source.scene, limits)?, limits)?;
    }
    Ok(total)
}
fn gradient_tile(
    placement: &paint_list_api::CommonPlacement,
    size: paint_list_api::LayoutSize,
    spacing: paint_list_api::LayoutSize,
) -> bool {
    spacing.width == 0.0 && spacing.height == 0.0 && size == placement.bounds.size()
}

fn affine_2d(m: &[f32; 16]) -> bool {
    [2, 3, 6, 7, 8, 9, 11, 14].iter().all(|i| m[*i] == 0.0) && m[10] == 1.0 && m[15] == 1.0
}

fn validate_scene(
    scene: &Scene,
    limits: BundleLimits,
    total: &mut usize,
    allow_missing: &BTreeSet<u64>,
) -> Result<(), BundleError> {
    scene_work(scene, limits)?;
    if scene.transforms.iter().any(|t| !affine_2d(&t.m)) {
        return Err(BundleError::InvalidScene {
            reason: "non-2D affine transform is not supported",
        });
    }
    if scene
        .fonts
        .first()
        .is_none_or(|f| !f.data.as_ref().is_empty() || f.index != 0)
    {
        return Err(BundleError::InvalidScene {
            reason: "font sentinel must be empty index zero",
        });
    }
    extent(scene.viewport_width, scene.viewport_height, limits)?;
    bound("scene ops", scene.ops.len(), limits.max_ops)?;
    netrender::vello_backends::validate_scene_for_backend(
        netrender::vello_backends::VelloBackend::Classic,
        scene,
    )
    .map_err(|_| BundleError::InvalidScene {
        reason: "invalid transform or layer structure",
    })?;
    if !scene.compositor_surfaces.is_empty() {
        return Err(BundleError::InvalidScene {
            reason: "native compositor resources are not portable",
        });
    }
    if !scene.root_alpha.is_finite() || !(0.0..=1.0).contains(&scene.root_alpha) {
        return Err(BundleError::InvalidScene {
            reason: "invalid root alpha",
        });
    }
    for font in scene.fonts.iter().skip(1) {
        add(total, font.data.as_ref().len(), limits)?;
        skrifa::FontRef::from_index(font.data.as_ref(), font.index).map_err(|_| {
            BundleError::InvalidScene {
                reason: "invalid source font",
            }
        })?;
    }
    for (&key, image) in &scene.image_sources {
        if allow_missing.contains(&key) {
            return Err(BundleError::InvalidScene {
                reason: "side-channel image already present in translated sources",
            });
        }
        if extent(image.width, image.height, limits)? != image.data.as_ref().len() {
            return Err(BundleError::InvalidScene {
                reason: "invalid image byte extent",
            });
        }
        add(total, image.data.as_ref().len(), limits)?;
    }
    let mut depth = 0usize;
    for op in &scene.ops {
        match op {
            SceneOp::Image(image) => {
                if !scene.image_sources.contains_key(&image.key)
                    && !allow_missing.contains(&image.key)
                {
                    return Err(BundleError::InvalidScene {
                        reason: "missing scene image",
                    });
                }
            }
            SceneOp::Pattern(pattern) => {
                if !scene.image_sources.contains_key(&pattern.tile) {
                    return Err(BundleError::InvalidScene {
                        reason: "missing scene pattern",
                    });
                }
            }
            SceneOp::Fragment(_) => {
                return Err(BundleError::InvalidScene {
                    reason: "retained registry is not serialized in producer scene",
                });
            }
            SceneOp::GlyphRun(run) => {
                let font = scene
                    .fonts
                    .get(run.font_id as usize)
                    .filter(|_| run.font_id != 0)
                    .ok_or(BundleError::InvalidScene {
                        reason: "missing scene font",
                    })?;
                let face =
                    skrifa::FontRef::from_index(font.data.as_ref(), font.index).map_err(|_| {
                        BundleError::InvalidScene {
                            reason: "invalid scene font",
                        }
                    })?;
                let count = face
                    .maxp()
                    .map_err(|_| BundleError::InvalidScene {
                        reason: "font has no glyph count",
                    })?
                    .num_glyphs();
                if run.glyphs.iter().any(|g| g.id >= u32::from(count)) {
                    return Err(BundleError::InvalidScene {
                        reason: "invalid scene glyph",
                    });
                }
                bound("glyphs", run.glyphs.len(), limits.max_ops)?;
            }
            SceneOp::PushLayer(layer) => {
                depth += 1;
                bound("layers", depth, limits.max_depth)?;
                if !layer.alpha.is_finite() || !(0.0..=1.0).contains(&layer.alpha) {
                    return Err(BundleError::InvalidScene {
                        reason: "invalid layer alpha",
                    });
                }
            }
            SceneOp::PopLayer => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    let mut graphics = scene.clone();
    graphics.fonts.clear();
    graphics.image_sources.clear();
    finite_graphics(&graphics, true, limits)?;
    Ok(())
}

/// Resource and graphics preflight for a standalone bound callback, before a
/// caller hashes it or updates source/stamp caches. No external state changes.
pub fn validate_source_scene(scene: &Scene, limits: BundleLimits) -> Result<(), BundleError> {
    validate_scene(scene, limits, &mut 0, &BTreeSet::new())
}
/// Preflight an entire borrowed source plan before any digest/cache work.
/// All aggregate bytes/structure are checked first; semantic validation and
/// finite traversal happen only after the complete plan fits its limits.
pub fn validate_source_scenes<'a>(
    scenes: impl IntoIterator<Item = &'a Scene>,
    limits: BundleLimits,
) -> Result<(), BundleError> {
    let mut refs = Vec::new();
    for scene in scenes {
        bound(
            "producer sources",
            refs.len().saturating_add(1),
            limits.max_sources,
        )?;
        refs.push(scene);
    }
    let mut work = 0;
    let mut bytes = 0;
    for scene in &refs {
        work_add(&mut work, scene_work(scene, limits)?, limits)?;
        for font in &scene.fonts {
            add(&mut bytes, font.data.as_ref().len(), limits)?;
        }
        for image in scene.image_sources.values() {
            add(&mut bytes, image.data.as_ref().len(), limits)?;
        }
    }
    for scene in refs {
        validate_source_scene(scene, limits)?;
    }
    Ok(())
}
/// Validate source limits before the canonical digest allocates font/image
/// copies. Source callbacks may use revision zero; their output still binds
/// the digest and must not change under a reused content stamp.
pub fn bounded_source_scene_digest(
    scene: &Scene,
    limits: BundleLimits,
) -> Result<[u8; 32], BundleError> {
    validate_source_scene(scene, limits)?;
    scene_digest(scene)
}

pub fn validate_capture(capture: &PortablePaintCapture) -> Result<(), BundleError> {
    validate_capture_with_limits(capture, BundleLimits::default())
}
pub fn validate_capture_with_limits(
    capture: &PortablePaintCapture,
    limits: BundleLimits,
) -> Result<(), BundleError> {
    validate_capture_bytes(capture, limits).map(|_| ())
}
fn validate_capture_bytes(
    capture: &PortablePaintCapture,
    limits: BundleLimits,
) -> Result<usize, BundleError> {
    capture_work(capture, limits)?;
    let env = &capture.envelope;
    if env.viewport.width <= 0 || env.viewport.height <= 0 {
        return Err(BundleError::InvalidEnvelope {
            command: None,
            reason: "empty viewport",
        });
    }
    extent(
        env.viewport.width as u32,
        env.viewport.height as u32,
        limits,
    )?;
    bound("commands", env.commands.len(), limits.max_ops)?;
    bound(
        "producer sources",
        capture.producers.len(),
        limits.max_sources,
    )?;
    let mut total = 0;
    let mut fonts = HashMap::new();
    let mut images = HashSet::new();
    for font in &env.fonts {
        add(&mut total, font.data.len(), limits)?;
        let face = skrifa::FontRef::from_index(&font.data, font.index).map_err(|_| {
            BundleError::InvalidEnvelope {
                command: None,
                reason: "invalid font",
            }
        })?;
        let count = face
            .maxp()
            .map_err(|_| BundleError::InvalidEnvelope {
                command: None,
                reason: "font has no glyph count",
            })?
            .num_glyphs();
        if fonts.insert(font.key, count).is_some() {
            return Err(BundleError::InvalidEnvelope {
                command: None,
                reason: "duplicate font",
            });
        }
    }
    for image in &env.images {
        if extent(image.width, image.height, limits)? != image.data.len() {
            return Err(BundleError::InvalidEnvelope {
                command: None,
                reason: "invalid image byte extent",
            });
        }
        add(&mut total, image.data.len(), limits)?;
        if !images.insert(image.key) {
            return Err(BundleError::InvalidEnvelope {
                command: None,
                reason: "duplicate image",
            });
        }
    }
    let mut producers = BTreeMap::new();
    for source in &capture.producers {
        if producers.insert(source.texture_key, source).is_some() {
            return Err(BundleError::DuplicateProducer(source.texture_key));
        }
        let bad = |reason| BundleError::InvalidProducer {
            key: source.texture_key,
            reason,
        };
        if source.texture_key >= (1u64 << 62) {
            return Err(bad("texture key exceeds external image namespace"));
        }
        if source.source_identity == 0 {
            return Err(bad("unbound source identity"));
        }
        if !source.layout_scale.is_finite()
            || source.layout_scale <= 0.0
            || source
                .logical_size
                .iter()
                .any(|v| !v.is_finite() || *v <= 0.0)
        {
            return Err(bad("invalid logical geometry/scale"));
        }
        let physical = source
            .logical_size
            .map(|v| (f64::from(v) * f64::from(source.layout_scale)).ceil() as u32);
        let logical = source.logical_size.map(|v| v.round().max(1.0) as u32);
        if physical != source.physical_size
            || logical != [source.scene.viewport_width, source.scene.viewport_height]
        {
            return Err(bad("source extent/scale differs from callback contract"));
        }
        extent(physical[0], physical[1], limits)?;
        if source.encoding != ResourceEncoding::Srgb {
            return Err(bad("only sRGB producer replay is supported"));
        }
        validate_scene(&source.scene, limits, &mut total, &BTreeSet::new())?;
    }
    let mut used = BTreeSet::new();
    let mut transforms = 0usize;
    let mut active_transforms: Vec<netrender::Transform> = Vec::new();
    let mut layers = Vec::new();
    for (i, cmd) in env.commands.iter().enumerate() {
        let bad = |reason| BundleError::InvalidEnvelope {
            command: Some(i),
            reason,
        };
        match cmd {
            PaintCmd::PushTransform(spec) => {
                if spec.kind != paint_list_api::TransformKind::Standard
                    || !affine_2d(&crate::convert::layout_transform_to_scene(&spec.transform).m)
                {
                    return Err(bad("non-2D transform is not portable"));
                }
                transforms += 1;
                bound("transform depth", transforms, limits.max_depth)?;
                let local = crate::emit::compose_with_origin(
                    &crate::convert::layout_transform_to_scene(&spec.transform),
                    spec.origin.x,
                    spec.origin.y,
                );
                let parent = active_transforms
                    .last()
                    .copied()
                    .unwrap_or(netrender::Transform::IDENTITY);
                let composed = netrender::Transform {
                    m: crate::emit::mat_mul(&parent.m, &local.m),
                };
                if composed.m.iter().any(|v| !v.is_finite()) {
                    return Err(bad("non-finite composed transform"));
                }
                active_transforms.push(composed);
            }
            PaintCmd::PopTransform => {
                transforms = transforms
                    .checked_sub(1)
                    .ok_or_else(|| bad("unbalanced transform"))?;
                active_transforms.pop();
            }
            PaintCmd::PushClip(spec) => {
                if let paint_list_api::ClipKind::RoundedRect {
                    radius, clip_out, ..
                } = &spec.kind
                {
                    if *clip_out
                        || [
                            radius.top_left,
                            radius.top_right,
                            radius.bottom_left,
                            radius.bottom_right,
                        ]
                        .iter()
                        .any(|r| r.width != r.height || r.width < 0.0)
                    {
                        return Err(bad("inverted or elliptical rounded clip is not lowered"));
                    }
                }
                layers.push(false);
                bound("layers", layers.len(), limits.max_depth)?;
            }
            PaintCmd::PushLayer(spec) => {
                if !matches!(
                    spec.mix_blend_mode,
                    paint_list_api::MixBlendMode::Normal
                        | paint_list_api::MixBlendMode::Multiply
                        | paint_list_api::MixBlendMode::Screen
                        | paint_list_api::MixBlendMode::Overlay
                        | paint_list_api::MixBlendMode::Darken
                        | paint_list_api::MixBlendMode::Lighten
                ) {
                    return Err(bad("blend mode is not lowered"));
                }
                if spec.origin.x != 0.0
                    || spec.origin.y != 0.0
                    || spec.raster_space != paint_list_api::RasterSpace::Screen
                {
                    return Err(bad("layer origin or raster space is not lowered"));
                }
                if !spec.filters.is_empty() {
                    return Err(bad("filter translation/parity is not admitted"));
                }
                if !(0.0..=1.0).contains(&spec.opacity) {
                    return Err(bad("invalid opacity"));
                }
                if spec.mask.is_some() {
                    return Err(bad("layer mask translation is unavailable"));
                }
                if spec.flags.0 != 0 {
                    return Err(bad("layer flags are not lowered"));
                }
                layers.push(true);
                bound("layers", layers.len(), limits.max_depth)?;
            }
            PaintCmd::PopClip => {
                if layers.pop() != Some(false) {
                    return Err(bad("unbalanced clip"));
                }
            }
            PaintCmd::PopLayer => {
                if layers.pop() != Some(true) {
                    return Err(bad("unbalanced layer"));
                }
            }
            PaintCmd::DrawShadow(item) => {
                if !item.border_radius.is_zero() {
                    return Err(bad("rounded shadow geometry is not lowered"));
                }
                if item.blur_radius < 0.0 {
                    return Err(bad("negative shadow blur"));
                }
                if item.blur_radius > 0.0 {
                    let active = active_transforms
                        .last()
                        .copied()
                        .unwrap_or(netrender::Transform::IDENTITY);
                    if active.m[1] != 0.0
                        || active.m[4] != 0.0
                        || active.m[0].abs() != 1.0
                        || active.m[5].abs() != 1.0
                    {
                        return Err(bad("blurred shadow transform geometry is not lowered"));
                    }
                }
            }
            PaintCmd::DrawExternalTexture(item) => {
                if item.texture_key >= (1u64 << 62) {
                    return Err(bad("texture key exceeds external image namespace"));
                }
                let source = producers
                    .get(&item.texture_key)
                    .ok_or(BundleError::MissingProducer(item.texture_key))?;
                if item.content_generation != Some(source.content_generation) {
                    return Err(BundleError::StaleProducer(item.texture_key));
                }
                if !(0.0..=1.0).contains(&item.opacity) {
                    return Err(bad("invalid external opacity"));
                }
                used.insert(item.texture_key);
            }
            PaintCmd::DrawText(run) => {
                let count = fonts
                    .get(&run.font_instance)
                    .ok_or_else(|| bad("missing font"))?;
                if run.font_size <= 0.0 || run.glyphs.iter().any(|g| g.index >= u32::from(*count)) {
                    return Err(bad("invalid glyph/font size"));
                }
                bound("positioned glyphs", run.glyphs.len(), limits.max_ops)?;
            }
            PaintCmd::DrawImage(item) => {
                if !images.contains(&item.image_key) {
                    return Err(bad("missing image"));
                }
            }
            PaintCmd::DrawRepeatingImage(item) => {
                if !images.contains(&item.image_key) {
                    return Err(bad("missing pattern image"));
                }
                if item.tile_spacing.width != 0.0
                    || item.tile_spacing.height != 0.0
                    || item.stretch_size.width <= 0.0
                    || item.stretch_size.height <= 0.0
                {
                    return Err(bad("pattern spacing/fallback is not admitted"));
                }
            }
            PaintCmd::DrawLinearGradient(v) => {
                if !gradient_tile(&v.placement, v.tile_size, v.tile_spacing) {
                    return Err(bad("gradient tile geometry is not lowered"));
                }
            }
            PaintCmd::DrawRadialGradient(v) => {
                if !gradient_tile(&v.placement, v.tile_size, v.tile_spacing) {
                    return Err(bad("gradient tile geometry is not lowered"));
                }
            }
            PaintCmd::DrawConicGradient(v) => {
                if !gradient_tile(&v.placement, v.tile_size, v.tile_spacing) {
                    return Err(bad("gradient tile geometry is not lowered"));
                }
            }
            PaintCmd::DrawLine(item) => {
                if item.style != paint_list_api::LineStyle::Solid {
                    return Err(bad("decorated-line fallback is not admitted"));
                }
            }
            PaintCmd::DrawBorder(item) => match &item.details {
                paint_list_api::BorderDetails::Normal(b) => {
                    crate::emit::admit_normal_border(item, b).map_err(bad)?;
                }
                paint_list_api::BorderDetails::NinePatch(_) => {
                    return Err(bad("nine-patch portable replay is not admitted"));
                }
            },
            PaintCmd::PushShadow(_) | PaintCmd::PopAllShadows => {
                return Err(bad("text-shadow stack is not implemented"));
            }
            PaintCmd::PlaceRetainedFragment(_) => {
                return Err(bad("retained registry is not serialized"));
            }
            _ => {}
        }
    }
    if transforms != 0 || !layers.is_empty() {
        return Err(BundleError::InvalidEnvelope {
            command: None,
            reason: "unterminated scopes",
        });
    }
    for key in producers.keys() {
        if !used.contains(key) {
            return Err(BundleError::ForeignProducer(*key));
        }
    }
    finite_graphics(&env.commands, false, limits)?;
    Ok(total)
}

pub fn translate_capture(
    capture: &PortablePaintCapture,
) -> Result<TranslatedResourceScene, BundleError> {
    translate_capture_with_limits(capture, BundleLimits::default())
}
fn translate_capture_with_limits(
    capture: &PortablePaintCapture,
    limits: BundleLimits,
) -> Result<TranslatedResourceScene, BundleError> {
    validate_capture_with_limits(capture, limits)?;
    let output =
        crate::translate_envelope_with_external_textures(&capture.envelope).map_err(|e| {
            BundleError::InvalidEnvelope {
                command: Some(e.command),
                reason: e.reason,
            }
        })?;
    Ok(TranslatedResourceScene {
        scene: output.scene,
        external_textures: output
            .external_textures
            .iter()
            .map(|e| ExternalResourceDraw {
                texture_key: e.texture_key,
                content_generation: e.content_generation,
                image_key: e.image_key,
                dest_rect: e.placement.dest_rect,
                uv: e.placement.uv,
                opacity: e.placement.opacity,
                scene_op_boundary: e.scene_op_boundary,
            })
            .collect(),
        box_shadow_masks: output
            .box_shadow_masks
            .iter()
            .map(|s| ShadowResourceRequest {
                key: s.key,
                dim: s.dim,
                bounds: s.bounds,
                corner_radius: s.corner_radius,
                blur_radius_px: s.blur_radius_px,
                invert: s.invert,
            })
            .collect(),
    })
}
impl ProducerSource {
    pub fn binding(&self) -> Result<ResourceBinding, BundleError> {
        Ok(ResourceBinding::Producer {
            content_generation: self.content_generation,
            source_identity: self.source_identity,
            source_revision: self.source_revision,
            logical_size: self.logical_size,
            physical_size: self.physical_size,
            layout_scale: self.layout_scale,
            source_scene_digest: scene_digest(&self.scene)?,
        })
    }
}
impl ShadowResourceRequest {
    pub fn binding(&self) -> Result<ResourceBinding, BundleError> {
        Ok(ResourceBinding::Shadow {
            request_digest: digest(self)?,
        })
    }
}

/// Explicit encoded-sRGB alpha conversion. RGB is never matted against black.
/// Fully transparent premultiplied pixels must carry zero RGB. Straight input
/// may carry hidden RGB, which remains straight until normal source-over math.
pub fn payload_to_straight(payload: &ResourcePayload) -> Result<Vec<u8>, BundleError> {
    if extent(payload.width, payload.height, BundleLimits::default())? != payload.rgba.len() {
        return Err(BundleError::InvalidPayload {
            resource: payload.resource,
            reason: "RGBA byte extent mismatch",
        });
    }
    if payload.encoding != ResourceEncoding::Srgb {
        return Err(BundleError::InvalidPayload {
            resource: payload.resource,
            reason: "only encoded-sRGB RGBA8 is supported",
        });
    }
    if payload.alpha == ResourceAlpha::Straight {
        return Ok(payload.rgba.clone());
    }
    let mut output = payload.rgba.clone();
    for pixel in output.chunks_exact_mut(4) {
        let a = u32::from(pixel[3]);
        if pixel[..3].iter().any(|c| u32::from(*c) > a) {
            return Err(BundleError::InvalidPayload {
                resource: payload.resource,
                reason: "premultiplied RGB exceeds alpha",
            });
        }
        for c in &mut pixel[..3] {
            *c = if a == 0 {
                0
            } else {
                ((u32::from(*c) * 255 + a / 2) / a).min(255) as u8
            };
        }
    }
    Ok(output)
}
/// Construct source-declared alpha bytes from a CPU renderer's premultiplied
/// sRGB output. This conversion is explicit at capture materialization time.
pub fn cpu_payload(
    resource: ResourceRef,
    binding: ResourceBinding,
    size: [u32; 2],
    alpha: ResourceAlpha,
    rgba: Vec<u8>,
) -> Result<ResourcePayload, BundleError> {
    let mut p = ResourcePayload {
        resource,
        binding,
        width: size[0],
        height: size[1],
        alpha: ResourceAlpha::Premultiplied,
        encoding: ResourceEncoding::Srgb,
        rgba,
    };
    if p.rgba.len() != extent(size[0], size[1], BundleLimits::default())? {
        return Err(BundleError::InvalidPayload {
            resource,
            reason: "invalid CPU pixel extent",
        });
    }
    let straight = payload_to_straight(&p)?;
    if alpha == ResourceAlpha::Straight {
        p.rgba = straight;
    }
    p.alpha = alpha;
    Ok(p)
}

/// Validate all resources before cloning/materializing an image-bearing scene.
/// `expected_capture_digest` must come from the caller's selected capture,
/// not from an untrusted incoming bundle's claimed digest.
pub fn admit_bundle(
    bundle: &CompleteResourceBundle,
    expected_capture_digest: [u8; 32],
    limits: BundleLimits,
) -> Result<Scene, BundleError> {
    if bundle.version != BUNDLE_VERSION {
        return Err(BundleError::Version(bundle.version));
    }
    let mut total = validate_capture_bytes(&bundle.capture, limits)?;
    let actual = capture_digest(&bundle.capture)?;
    if actual != expected_capture_digest || actual != bundle.capture_digest {
        return Err(BundleError::CaptureIdentity);
    }
    let expected = translate_capture_with_limits(&bundle.capture, limits)?;
    if expected.external_textures != bundle.translated.external_textures
        || expected.box_shadow_masks != bundle.translated.box_shadow_masks
    {
        return Err(BundleError::SideChannelIdentity);
    }
    let mut required = BTreeMap::new();
    for source in &bundle.capture.producers {
        required.insert(
            ResourceRef::ExternalTexture(source.texture_key),
            (
                source.binding()?,
                source.physical_size,
                source.alpha,
                source.encoding,
                external_image_key(source.texture_key),
            ),
        );
    }
    for shadow in &expected.box_shadow_masks {
        if required
            .insert(
                ResourceRef::ShadowMask(shadow.key),
                (
                    shadow.binding()?,
                    [shadow.dim, shadow.dim],
                    ResourceAlpha::Straight,
                    ResourceEncoding::Srgb,
                    shadow.key,
                ),
            )
            .is_some()
        {
            return Err(BundleError::DuplicateResource(ResourceRef::ShadowMask(
                shadow.key,
            )));
        }
    }
    let allow_missing: BTreeSet<_> = required.values().map(|r| r.4).collect();
    if allow_missing.len() != required.len() {
        return Err(BundleError::InvalidScene {
            reason: "external/shadow image namespaces collide",
        });
    }
    let mut work = capture_work(&bundle.capture, limits)?;
    work_add(
        &mut work,
        scene_work(&bundle.translated.scene, limits)?,
        limits,
    )?;
    validate_scene(&bundle.translated.scene, limits, &mut total, &allow_missing)?;
    if scene_digest(&expected.scene)? != scene_digest(&bundle.translated.scene)? {
        return Err(BundleError::TranslationIdentity);
    }
    bound(
        "resource entries",
        bundle.resources.len(),
        limits.max_resources,
    )?;
    let mut seen = BTreeSet::new();
    for payload in &bundle.resources {
        if !seen.insert(payload.resource) {
            return Err(BundleError::DuplicateResource(payload.resource));
        }
        let (binding, size, alpha, encoding, _) = required
            .get(&payload.resource)
            .ok_or(BundleError::ForeignResource(payload.resource))?;
        if &payload.binding != binding {
            return Err(BundleError::StaleResource(payload.resource));
        }
        if [payload.width, payload.height] != *size {
            return Err(BundleError::InvalidPayload {
                resource: payload.resource,
                reason: "payload extent differs from bound source",
            });
        }
        if payload.alpha != *alpha {
            return Err(BundleError::InvalidPayload {
                resource: payload.resource,
                reason: "payload alpha differs from bound source",
            });
        }
        if payload.encoding != *encoding || payload.encoding != ResourceEncoding::Srgb {
            return Err(BundleError::InvalidPayload {
                resource: payload.resource,
                reason: "payload encoding differs from source/contract",
            });
        }
        if extent(payload.width, payload.height, limits)? != payload.rgba.len() {
            return Err(BundleError::InvalidPayload {
                resource: payload.resource,
                reason: "RGBA byte extent mismatch",
            });
        }
        add(&mut total, payload.rgba.len(), limits)?;
        // Validate premultiplied bounds before any target/resource mutation.
        if payload.alpha == ResourceAlpha::Premultiplied
            && payload
                .rgba
                .chunks_exact(4)
                .any(|p| p[..3].iter().any(|v| *v > p[3]))
        {
            return Err(BundleError::InvalidPayload {
                resource: payload.resource,
                reason: "premultiplied RGB exceeds alpha",
            });
        }
    }
    for resource in required.keys() {
        if !seen.contains(resource) {
            return Err(BundleError::MissingResource(*resource));
        }
    }
    let mut scene = bundle.translated.scene.clone();
    for payload in &bundle.resources {
        let key = required[&payload.resource].4;
        scene.image_sources.insert(
            key,
            ImageData::from_bytes(payload.width, payload.height, payload_to_straight(payload)?),
        );
    }
    Ok(scene)
}
