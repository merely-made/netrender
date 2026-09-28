// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Owned, image-bearing sparse sessions. The legacy free lowerers deliberately
//! remain resource-free. These sessions admit a narrower, validated parameter
//! subset and never import an external producer's GPU image implicitly.

use super::{BackendAdmissionError, VelloBackend, sparse};
use crate::scene::{ImageData, ImageKey, Scene, SceneImage, SceneOp, ScenePattern};
use std::{collections::HashMap, sync::Arc};
use vello_sparse_common::{
    kurbo::Affine,
    paint::{Image, ImageSource},
    peniko::{Extend, ImageQuality},
    pixmap::Pixmap,
};

/// Configurable admission limits, applied before allocating image resources.
/// These bound session image payloads and scene inputs, not total process/GPU
/// memory, backend scratch allocations, or commands retained by the host.
#[derive(Debug, Clone, Copy)]
pub struct SparseResourceLimits {
    pub max_sources: usize,
    pub max_source_bytes: usize,
    /// Distinct source/crop/tint variants used by a single frame.
    pub max_cached_images: usize,
    /// Premultiplied pixel bytes; Hybrid additionally retains the same number
    /// of GPU texture payload bytes, reported separately in stats.
    pub max_cached_bytes: usize,
    pub max_viewport_pixels: usize,
    /// Sum of draw operations, palette entries, path segments and gradient stops.
    pub max_ops: usize,
    pub max_layers: usize,
}

impl Default for SparseResourceLimits {
    fn default() -> Self {
        Self {
            max_sources: 256,
            max_source_bytes: 64 * 1024 * 1024,
            max_cached_images: 256,
            max_cached_bytes: 64 * 1024 * 1024,
            max_viewport_pixels: 16 * 1024 * 1024,
            max_ops: 100_000,
            max_layers: 64,
        }
    }
}

/// Current logical residency and cumulative work. GPU bytes count image
/// texture payloads only, excluding driver alignment, renderer scratch, target
/// textures and resources kept alive by previously encoded/in-flight commands.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SparseResourceStats {
    pub source_count: usize,
    pub source_bytes: usize,
    pub cached_images: usize,
    pub cached_bytes: usize,
    pub gpu_texture_bytes: usize,
    pub hydrated_images: u64,
    pub uploaded_images: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SparseSessionError {
    Admission(BackendAdmissionError),
    MissingImage {
        key: ImageKey,
    },
    InvalidImage {
        key: ImageKey,
        reason: &'static str,
    },
    InvalidFont {
        op_index: usize,
        font_id: u32,
        reason: &'static str,
    },
    InvalidGlyph {
        op_index: usize,
        glyph_id: u32,
        reason: &'static str,
    },
    InvalidScene {
        op_index: Option<usize>,
        reason: &'static str,
    },
    ResourceBudgetExceeded {
        resource: &'static str,
        limit: usize,
        requested: usize,
    },
    InvalidTarget {
        reason: &'static str,
    },
    HybridRender(String),
}

impl std::fmt::Display for SparseSessionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "sparse session: {self:?}")
    }
}
impl std::error::Error for SparseSessionError {}
impl From<BackendAdmissionError> for SparseSessionError {
    fn from(value: BackendAdmissionError) -> Self {
        Self::Admission(value)
    }
}

fn budget(
    resource: &'static str,
    requested: usize,
    limit: usize,
) -> Result<(), SparseSessionError> {
    if requested > limit {
        return Err(SparseSessionError::ResourceBudgetExceeded {
            resource,
            limit,
            requested,
        });
    }
    Ok(())
}

fn image_bytes(
    key: ImageKey,
    source: &ImageData,
    max_dimension: u32,
) -> Result<usize, SparseSessionError> {
    let invalid = |reason| SparseSessionError::InvalidImage { key, reason };
    if source.width == 0
        || source.height == 0
        || source.width > max_dimension
        || source.height > max_dimension
    {
        return Err(invalid(
            "image dimensions must be nonzero and within the backend/device limit",
        ));
    }
    let bytes = (source.width as usize)
        .checked_mul(source.height as usize)
        .and_then(|v| v.checked_mul(4))
        .ok_or_else(|| invalid("image byte count overflow"))?;
    if bytes != source.data.as_ref().len() {
        return Err(invalid(
            "RGBA8 payload must contain exactly width * height * 4 bytes",
        ));
    }
    Ok(bytes)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct VariantKey {
    key: ImageKey,
    blob: u64,
    source_size: [u32; 2],
    crop: [u32; 4],
    tint: [u32; 4],
}
impl VariantKey {
    fn width(self) -> u16 {
        (self.crop[2] - self.crop[0]) as u16
    }
    fn height(self) -> u16 {
        (self.crop[3] - self.crop[1]) as u16
    }
    fn bytes(self) -> usize {
        self.width() as usize * self.height() as usize * 4
    }
}

struct CachedImage {
    pixmap: Arc<Pixmap>,
    #[cfg(feature = "vello-hybrid")]
    texture: Option<wgpu::Texture>,
}

struct ImageState {
    limits: SparseResourceLimits,
    max_dimension: u32,
    sources: HashMap<ImageKey, ImageData>,
    cache: HashMap<VariantKey, CachedImage>,
    hydrated: u64,
    uploaded: u64,
}

struct FramePlan {
    sources: HashMap<ImageKey, ImageData>,
    variants: HashMap<VariantKey, ImageData>,
    ops: HashMap<usize, VariantKey>,
}

impl ImageState {
    fn new(limits: SparseResourceLimits, max_dimension: u32) -> Self {
        Self {
            limits,
            max_dimension,
            sources: HashMap::new(),
            cache: HashMap::new(),
            hydrated: 0,
            uploaded: 0,
        }
    }

    fn updated_sources<'a>(
        &self,
        changes: impl IntoIterator<Item = (ImageKey, &'a ImageData)>,
    ) -> Result<HashMap<ImageKey, ImageData>, SparseSessionError> {
        let mut sources = self.sources.clone();
        for (key, source) in changes {
            image_bytes(key, source, self.max_dimension)?;
            // Capture serde can reconstruct arbitrary blob ids. Do not let a
            // changed payload masquerade as a cache hit for the same identity.
            if let Some(old) = sources.get(&key) {
                if old.data.id() == source.data.id() && old.data.as_ref() != source.data.as_ref() {
                    return Err(SparseSessionError::InvalidImage {
                        key,
                        reason: "blob identity reused with different pixel bytes",
                    });
                }
            }
            sources.insert(key, source.clone());
            budget("source count", sources.len(), self.limits.max_sources)?;
        }
        let total = sources
            .values()
            .try_fold(0usize, |total, image| {
                total.checked_add(image.data.as_ref().len())
            })
            .ok_or(SparseSessionError::InvalidScene {
                op_index: None,
                reason: "source byte accounting overflow",
            })?;
        budget("source bytes", total, self.limits.max_source_bytes)?;
        Ok(sources)
    }

    fn set_image_source(
        &mut self,
        key: ImageKey,
        data: ImageData,
    ) -> Result<(), SparseSessionError> {
        let sources = self.updated_sources([(key, &data)])?;
        self.cache.retain(|variant, _| {
            variant.key != key
                || (variant.blob == data.data.id()
                    && variant.source_size == [data.width, data.height])
        });
        self.sources = sources;
        Ok(())
    }

    fn remove_image_source(&mut self, key: ImageKey) -> bool {
        self.cache.retain(|variant, _| variant.key != key);
        self.sources.remove(&key).is_some()
    }

    fn clear_images(&mut self) {
        self.cache.clear();
        self.sources.clear();
    }

    fn stats(&self) -> SparseResourceStats {
        SparseResourceStats {
            source_count: self.sources.len(),
            source_bytes: self
                .sources
                .values()
                .map(|source| source.data.as_ref().len())
                .sum(),
            cached_images: self.cache.len(),
            cached_bytes: self.cache.keys().map(|key| key.bytes()).sum(),
            gpu_texture_bytes: {
                #[cfg(feature = "vello-hybrid")]
                {
                    self.cache
                        .iter()
                        .filter(|(_, value)| value.texture.is_some())
                        .map(|(key, _)| key.bytes())
                        .sum()
                }
                #[cfg(not(feature = "vello-hybrid"))]
                {
                    0
                }
            },
            hydrated_images: self.hydrated,
            uploaded_images: self.uploaded,
        }
    }

    fn plan(&self, scene: &Scene, backend: VelloBackend) -> Result<FramePlan, SparseSessionError> {
        super::validate_scene_operations(backend, scene, true)?;
        validate::scene(scene, self.limits)?;
        if backend == VelloBackend::Hybrid {
            for (index, op) in scene.ops.iter().enumerate() {
                if matches!(op, SceneOp::Pattern(pattern) if !pattern.nearest) {
                    return Err(super::unsupported(
                        backend,
                        index,
                        "Pattern",
                        "pinned Hybrid bilinear repeat clamps filter taps at tile seams; only explicit nearest sampling is admitted",
                    ).into());
                }
            }
        }
        let sources =
            self.updated_sources(scene.image_sources.iter().map(|(key, value)| (*key, value)))?;
        let mut variants = HashMap::new();
        let mut ops = HashMap::new();
        let mut bytes = 0usize;
        for (index, op) in scene.ops.iter().enumerate() {
            if let SceneOp::Pattern(pattern) = op {
                let source = sources
                    .get(&pattern.tile)
                    .ok_or(SparseSessionError::MissingImage { key: pattern.tile })?;
                let key = VariantKey {
                    key: pattern.tile,
                    blob: source.data.id(),
                    source_size: [source.width, source.height],
                    crop: [0, 0, source.width, source.height],
                    tint: [1.0_f32.to_bits(); 4],
                };
                if let std::collections::hash_map::Entry::Vacant(entry) = variants.entry(key) {
                    bytes =
                        bytes
                            .checked_add(key.bytes())
                            .ok_or(SparseSessionError::InvalidImage {
                                key: pattern.tile,
                                reason: "derived image byte accounting overflow",
                            })?;
                    budget("cached image bytes", bytes, self.limits.max_cached_bytes)?;
                    entry.insert(source.clone());
                    budget(
                        "cached image count",
                        variants.len(),
                        self.limits.max_cached_images,
                    )?;
                }
                ops.insert(index, key);
            }
            if let SceneOp::Image(image) = op {
                let source = sources
                    .get(&image.key)
                    .ok_or(SparseSessionError::MissingImage { key: image.key })?;
                let crop = if image.clamp_to_uv {
                    let px = |value: f32, dim: u32| (value * dim as f32).round() as u32;
                    let x0 = px(image.uv[0], source.width);
                    let y0 = px(image.uv[1], source.height);
                    let x1 = px(image.uv[2], source.width).max(x0 + 1).min(source.width);
                    let y1 = px(image.uv[3], source.height)
                        .max(y0 + 1)
                        .min(source.height);
                    if x0 >= x1 || y0 >= y1 {
                        return Err(SparseSessionError::InvalidImage {
                            key: image.key,
                            reason: "UV crop rounds to an empty source region",
                        });
                    }
                    [x0, y0, x1, y1]
                } else {
                    [0, 0, source.width, source.height]
                };
                let key = VariantKey {
                    key: image.key,
                    blob: source.data.id(),
                    source_size: [source.width, source.height],
                    crop,
                    tint: image.color.map(f32::to_bits),
                };
                let m = &scene.transforms[image.transform_id as usize].m;
                let world = Affine::new([
                    m[0] as f64,
                    m[1] as f64,
                    m[4] as f64,
                    m[5] as f64,
                    m[12] as f64,
                    m[13] as f64,
                ]);
                let brush = paint_transform(image, key);
                let corners_valid = [
                    (image.x0, image.y0),
                    (image.x0, image.y1),
                    (image.x1, image.y0),
                    (image.x1, image.y1),
                ]
                .into_iter()
                .all(|(x, y)| {
                    let point = world * vello_sparse_common::kurbo::Point::new(x as f64, y as f64);
                    (point.x as f32).is_finite() && (point.y as f32).is_finite()
                });
                if !validate::safe_affine(brush)
                    || !validate::safe_affine(world * brush)
                    || !corners_valid
                {
                    return Err(SparseSessionError::InvalidScene {
                        op_index: Some(index),
                        reason: "derived image mapping exceeds sparse f32 precision/range",
                    });
                }
                if let std::collections::hash_map::Entry::Vacant(entry) = variants.entry(key) {
                    bytes =
                        bytes
                            .checked_add(key.bytes())
                            .ok_or(SparseSessionError::InvalidImage {
                                key: image.key,
                                reason: "derived image byte accounting overflow",
                            })?;
                    budget("cached image bytes", bytes, self.limits.max_cached_bytes)?;
                    entry.insert(source.clone());
                    budget(
                        "cached image count",
                        variants.len(),
                        self.limits.max_cached_images,
                    )?;
                }
                ops.insert(index, key);
            }
        }
        Ok(FramePlan {
            sources,
            variants,
            ops,
        })
    }

    fn hydrate(&mut self, plan: &FramePlan) {
        self.cache.retain(|key, _| plan.variants.contains_key(key));
        for (key, source) in &plan.variants {
            if self.cache.contains_key(key) {
                continue;
            }
            // Allocate through Pixmap rather than casting caller-owned Vec
            // storage (upstream from_parts requires capacity divisible by 4).
            let mut pixmap = Pixmap::new(key.width(), key.height());
            let tint = key.tint.map(f32::from_bits);
            for row in 0..key.height() as usize {
                for col in 0..key.width() as usize {
                    let offset = ((row + key.crop[1] as usize) * source.width as usize
                        + col
                        + key.crop[0] as usize)
                        * 4;
                    let rgba = &source.data.as_ref()[offset..offset + 4];
                    let alpha = rgba[3] as f32 / 255.0;
                    let target = &mut pixmap.data_as_u8_slice_mut()
                        [(row * key.width() as usize + col) * 4..][..4];
                    for channel in 0..3 {
                        target[channel] =
                            (rgba[channel] as f32 * alpha * tint[channel]).round() as u8;
                    }
                    target[3] = (rgba[3] as f32 * tint[3]).round() as u8;
                }
            }
            pixmap.recompute_may_have_transparency();
            self.cache.insert(
                *key,
                CachedImage {
                    pixmap: Arc::new(pixmap),
                    #[cfg(feature = "vello-hybrid")]
                    texture: None,
                },
            );
            self.hydrated = self.hydrated.saturating_add(1);
        }
        self.sources = plan.sources.clone();
    }
}

fn paint_transform(image: &SceneImage, key: VariantKey) -> Affine {
    let uv = if image.clamp_to_uv {
        [0.0, 0.0, 1.0, 1.0]
    } else {
        image.uv
    };
    let sx =
        (image.x1 as f64 - image.x0 as f64) / ((uv[2] as f64 - uv[0] as f64) * key.width() as f64);
    let sy =
        (image.y1 as f64 - image.y0 as f64) / ((uv[3] as f64 - uv[1] as f64) * key.height() as f64);
    Affine::translate((
        image.x0 as f64 - uv[0] as f64 * key.width() as f64 * sx,
        image.y0 as f64 - uv[1] as f64 * key.height() as f64 * sy,
    )) * Affine::scale_non_uniform(sx, sy)
}

fn pattern_transform(pattern: &ScenePattern) -> Affine {
    // ScenePattern's existing contract normalizes each nonpositive axis to 1.
    // Nonfinite values are refused during preflight before this helper runs.
    let [sx, sy] = pattern
        .scale
        .map(|value| if value > 0.0 { value as f64 } else { 1.0 });
    Affine::translate((pattern.extent[0] as f64, pattern.extent[1] as f64))
        * Affine::scale_non_uniform(sx, sy)
}

fn paint(op: &SceneOp, key: VariantKey, source: ImageSource) -> sparse::ImagePaint {
    let (extend, nearest, transform) = match op {
        SceneOp::Image(image) => (Extend::Pad, image.nearest, paint_transform(image, key)),
        SceneOp::Pattern(pattern) => (Extend::Repeat, pattern.nearest, pattern_transform(pattern)),
        _ => unreachable!("only admitted image/pattern operations have resource paints"),
    };
    sparse::ImagePaint {
        image: Image {
            image: source,
            sampler: Default::default(),
        }
        .with_extend(extend)
        .with_quality(if nearest {
            ImageQuality::Low
        } else {
            ImageQuality::Medium
        }),
        transform,
    }
}

/// CPU rendering with persistent CPU-owned images. Rendering never requests a
/// GPU device. Scene image entries update this session's source palette;
/// omission retains the previous source until explicit removal/clear.
///
/// Images require nonempty ordered destination bounds and normalized ordered
/// UVs (`0 <= u0 < u1 <= 1`, likewise V). Clamp-to-UV rounds each boundary to
/// source pixels and expands the far edge to at least one pixel, matching
/// Classic's crop for this admitted subset; an empty rounded crop is refused.
/// Unclamped draws sample the whole source with padded edges. Both nearest and
/// bilinear sampling, device-space rounded clips and 2D affine transforms are
/// admitted. RGBA tint must be premultiplied (`0 <= RGB <= A <= 1`) and is
/// multiplied into derived source pixels. Degenerate or overflowing derived
/// transforms, filters and registered fragments are refused.
///
/// Patterns repeat an untinted full source tile on both axes, with phase
/// anchored at the extent's top-left. Extents must be finite, ordered and
/// nonempty. Per-axis scales must be finite; zero and negative values normalize
/// to 1, matching Classic. Nearest/bilinear sampling, affine transforms and
/// device-space rounded clips are supported. Pattern scale/sampling does not
/// create new pixel variants: patterns share the full untinted image cache.
///
/// Outline text consumes caller-shaped glyphs and the current Scene font
/// palette, including collection indices and user-space variations. Text is
/// unhinted and uses solid premultiplied color; color/bitmap/SVG font tables
/// are refused. Unknown printable axis tags are ignored as in Classic.
/// `SparseTextLimits` bounds a conservative cache epoch; a full epoch is
/// replaced only after the complete next frame passes admission. Font assets
/// remain trusted parser inputs, not a font-sanitizer or CPU-time sandbox.
#[cfg(feature = "vello-cpu")]
pub struct CpuSession {
    state: ImageState,
    resources: vello_cpu::Resources,
    text: text::TextState,
}

macro_rules! image_api {
    () => {
        /// Insert or replace a CPU-owned, straight-alpha RGBA8 source. New blob
        /// identity or dimensions invalidate derived pixels. This is atomic on
        /// admission failure. Removing a key from Scene alone does not remove it.
        pub fn set_image_source(
            &mut self,
            key: ImageKey,
            data: ImageData,
        ) -> Result<(), SparseSessionError> {
            self.state.set_image_source(key, data)
        }
        /// Drop this source and every derived image owned by this session.
        pub fn remove_image_source(&mut self, key: ImageKey) -> bool {
            self.state.remove_image_source(key)
        }
        pub fn clear_images(&mut self) {
            self.state.clear_images();
        }
        pub fn stats(&self) -> SparseResourceStats {
            self.state.stats()
        }
        /// Conservative outline-cache epoch accounting, separate from images.
        pub fn text_stats(&self) -> SparseTextStats {
            self.text.stats()
        }
        /// Drop all retained font references and outline preparation storage.
        /// Does not clear image resources or reconstruct GPU pipelines.
        pub fn clear_text_cache(&mut self) {
            self.text.clear();
        }
    };
}

#[cfg(feature = "vello-cpu")]
impl CpuSession {
    pub fn new(limits: SparseResourceLimits) -> Self {
        Self::new_with_text_limits(limits, SparseTextLimits::default())
    }
    pub fn new_with_text_limits(
        limits: SparseResourceLimits,
        text_limits: SparseTextLimits,
    ) -> Self {
        Self {
            state: ImageState::new(limits, u16::MAX as u32),
            resources: vello_cpu::Resources::new(),
            text: text::TextState::new(text_limits),
        }
    }
    image_api!();

    /// Return fresh premultiplied RGBA pixels. Admission failures leave owned
    /// sources and caches unchanged. Unused derived variants are released after
    /// successful admission; original sources persist until explicitly removed.
    pub fn render(&mut self, scene: &Scene) -> Result<Pixmap, SparseSessionError> {
        let plan = self.state.plan(scene, VelloBackend::Cpu)?;
        let text_plan = self.text.plan(scene)?;
        self.state.hydrate(&plan);
        self.text.commit(&text_plan);
        let paints = plan
            .ops
            .iter()
            .map(|(index, key)| {
                (
                    *index,
                    paint(
                        &scene.ops[*index],
                        *key,
                        ImageSource::Pixmap(self.state.cache[key].pixmap.clone()),
                    ),
                )
            })
            .collect();
        let mut context = vello_cpu::RenderContext::new(
            scene.viewport_width as u16,
            scene.viewport_height as u16,
        );
        sparse::lower_admitted(&mut context, scene, &paints, |context, index, run| {
            self.text.draw(context, scene, run, &text_plan.runs[&index]);
        });
        context.flush();
        let mut target = Pixmap::new(scene.viewport_width as u16, scene.viewport_height as u16);
        context.render(&mut target, &mut self.resources);
        self.text.maintain();
        Ok(target)
    }
}

/// Hybrid rendering on the host's existing device. Images are hydrated from
/// CPU-owned sources into session-owned textures, using sparse texture bindings
/// internally; this does not admit externally registered Scene GPU images.
/// Image parameter, crop, tint and source-lifetime semantics match
/// `CpuSession` when that feature is enabled; the same subset is available
/// in Hybrid-only builds. See this module's session admission documentation.
///
/// Patterns require explicit nearest sampling (`nearest = true`). Bilinear
/// patterns are refused before resource/target mutation because the pinned
/// shader clamps filtering taps at repeated tile boundaries. The requested
/// sampler is never silently changed. CPU sessions support bilinear repeat.
/// Admitted patterns repeat a full untinted source, anchored at the extent's
/// top-left, with finite nonpositive scale axes normalized to 1. Extent,
/// affine and device-space clip validation match the CPU pattern subset.
///
/// Outline text uses the same caller-shaped, unhinted solid-color subset and
/// `SparseTextLimits` as CPU. An owned Glifo preparation cache is replaced at
/// epoch limits or explicit clear; GPU pipelines and image textures survive.
/// Color/bitmap/SVG font tables are refused. The Scene font palette is required
/// each frame; fonts are trusted parser inputs, not a security sandbox.
#[cfg(feature = "vello-hybrid")]
pub struct HybridSession {
    state: ImageState,
    device: wgpu::Device,
    queue: wgpu::Queue,
    renderer: vello_hybrid::Renderer,
    resources: vello_hybrid::Resources,
    text: text::TextState,
}

#[cfg(feature = "vello-hybrid")]
impl HybridSession {
    /// The host must supply a device and its associated queue.
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        limits: SparseResourceLimits,
    ) -> Result<Self, SparseSessionError> {
        Self::new_with_text_limits(device, queue, limits, SparseTextLimits::default())
    }
    pub fn new_with_text_limits(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        limits: SparseResourceLimits,
        text_limits: SparseTextLimits,
    ) -> Result<Self, SparseSessionError> {
        let (renderer, resources) = vello_hybrid::Renderer::new(
            device,
            &vello_hybrid::RenderTargetConfig {
                format: wgpu::TextureFormat::Rgba8Unorm,
                width: 1,
                height: 1,
            },
        );
        Ok(Self {
            state: ImageState::new(
                limits,
                device
                    .limits()
                    .max_texture_dimension_2d
                    .min(u16::MAX as u32),
            ),
            device: device.clone(),
            queue: queue.clone(),
            renderer,
            resources,
            text: text::TextState::new(text_limits),
        })
    }
    image_api!();

    /// Encode into a same-device, single-sample Rgba8Unorm target. The host owns
    /// submission/readback and must submit this encoder before the next render
    /// call on this session: upstream updates shared buffers with queue writes.
    /// Encoder and target device identity is not exposed by
    /// wgpu: they MUST originate from this session's device. The checked device
    /// and queue identities do not establish a GPU-object sandbox.
    ///
    /// Scene/resource/observable-target admission failures encode no commands
    /// and change no session state. Device/runtime failures remain host-owned.
    pub fn render(
        &mut self,
        scene: &Scene,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::Texture,
    ) -> Result<(), SparseSessionError> {
        if device != &self.device || queue != &self.queue {
            return Err(SparseSessionError::InvalidTarget {
                reason: "device/queue differs from session identity",
            });
        }
        if target.format() != wgpu::TextureFormat::Rgba8Unorm
            || target.sample_count() != 1
            || target.dimension() != wgpu::TextureDimension::D2
            || target.depth_or_array_layers() != 1
            || target.mip_level_count() != 1
            || target.width() != scene.viewport_width
            || target.height() != scene.viewport_height
            || !target
                .usage()
                .contains(wgpu::TextureUsages::RENDER_ATTACHMENT)
        {
            return Err(SparseSessionError::InvalidTarget {
                reason: "target must match viewport and be a single-mip, single-sample Rgba8Unorm D2 render attachment",
            });
        }
        let plan = self.state.plan(scene, VelloBackend::Hybrid)?;
        let text_plan = self.text.plan(scene)?;
        self.state.hydrate(&plan);
        self.text.commit(&text_plan);
        let mut bindings = vello_hybrid::TextureBindings::new();
        let mut sources = HashMap::new();
        for (index, (key, cached)) in self.state.cache.iter_mut().enumerate() {
            if cached.texture.is_none() {
                let texture = device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("NetRender sparse CPU-owned image"),
                    size: wgpu::Extent3d {
                        width: key.width() as u32,
                        height: key.height() as u32,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                    view_formats: &[],
                });
                queue.write_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: &texture,
                        mip_level: 0,
                        origin: wgpu::Origin3d::ZERO,
                        aspect: wgpu::TextureAspect::All,
                    },
                    cached.pixmap.data_as_u8_slice(),
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(key.width() as u32 * 4),
                        rows_per_image: Some(key.height() as u32),
                    },
                    texture.size(),
                );
                cached.texture = Some(texture);
                self.state.uploaded = self.state.uploaded.saturating_add(1);
            }
            let id = vello_sparse_common::TextureId(index as u64);
            bindings.insert(
                id,
                cached
                    .texture
                    .as_ref()
                    .unwrap()
                    .create_view(&wgpu::TextureViewDescriptor::default()),
            );
            sources.insert(
                *key,
                ImageSource::external_texture(
                    id,
                    vello_sparse_common::geometry::RectU16::new(0, 0, key.width(), key.height()),
                    cached.pixmap.may_have_transparency(),
                ),
            );
        }
        let paints = plan
            .ops
            .iter()
            .map(|(index, key)| {
                (
                    *index,
                    paint(&scene.ops[*index], *key, sources[key].clone()),
                )
            })
            .collect();
        let mut packet =
            vello_hybrid::Scene::new(scene.viewport_width as u16, scene.viewport_height as u16);
        sparse::lower_admitted(&mut packet, scene, &paints, |context, index, run| {
            self.text.draw(context, scene, run, &text_plan.runs[&index]);
        });
        let result = self
            .renderer
            .render(
                &packet,
                &mut self.resources,
                device,
                queue,
                encoder,
                &vello_hybrid::RenderSize {
                    width: scene.viewport_width,
                    height: scene.viewport_height,
                },
                &target.create_view(&wgpu::TextureViewDescriptor::default()),
                None,
                &bindings,
            )
            .map_err(|error| SparseSessionError::HybridRender(error.to_string()));
        self.text.maintain();
        result
    }
}

mod text;
mod validate;
pub use text::{SparseTextLimits, SparseTextStats};
