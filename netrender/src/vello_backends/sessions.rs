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
    /// Geometry exceeded bounded curve-quality work after source admission.
    /// Prepared source/text caches may exist; no target pixels or GPU draw
    /// submission have been produced for the refused recording.
    CurveFlattening {
        reason: String,
    },
    Admission(BackendAdmissionError),
    MissingFragment {
        id: crate::scene::FragmentId,
    },
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
    /// Unexpected glyph execution failure after successful resource admission.
    /// No CPU target is returned and no Hybrid draw commands are encoded.
    GlyphRender {
        op_index: usize,
        reason: String,
    },
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
        let mut glyph_error = None;
        sparse::lower_admitted(&mut context, scene, &paints, |context, index, run| {
            if glyph_error.is_none() {
                if let Err(error) = self.text.draw(context, scene, run, &text_plan.runs[&index]) {
                    glyph_error = Some(SparseSessionError::GlyphRender {
                        op_index: index,
                        reason: error.to_string(),
                    });
                }
            }
        });
        if let Some(error) = glyph_error {
            return Err(error);
        }
        context.flush();
        if let Some(error) = context.flattening_error() {
            return Err(SparseSessionError::CurveFlattening {
                reason: error.to_string(),
            });
        }
        let mut target = Pixmap::new(scene.viewport_width as u16, scene.viewport_height as u16);
        context
            .render_checked(&mut target, &mut self.resources)
            .map_err(|error| SparseSessionError::CurveFlattening {
                reason: error.to_string(),
            })?;
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
        let mut glyph_error = None;
        sparse::lower_admitted(&mut packet, scene, &paints, |context, index, run| {
            if glyph_error.is_none() {
                if let Err(error) = self.text.draw(context, scene, run, &text_plan.runs[&index]) {
                    glyph_error = Some(SparseSessionError::GlyphRender {
                        op_index: index,
                        reason: error.to_string(),
                    });
                }
            }
        });
        if let Some(error) = glyph_error {
            return Err(error);
        }
        if let Some(error) = packet.flattening_error() {
            return Err(SparseSessionError::CurveFlattening {
                reason: error.to_string(),
            });
        }
        let result = self
            .renderer
            .render(
                &packet,
                &mut self.resources,
                device,
                queue,
                encoder,
                &vello_hybrid::RenderSize {
                    // Sparse admission has already bounded both dimensions to u16.
                    width: scene.viewport_width as u16,
                    height: scene.viewport_height as u16,
                },
                &target.create_view(&wgpu::TextureViewDescriptor::default()),
                None,
                &bindings,
                vello_hybrid::TargetInit::Clear(vello_hybrid::ClearSettings::default()),
            )
            .map_err(|error| SparseSessionError::HybridRender(error.to_string()));
        self.text.maintain();
        result
    }
}

mod text;
pub(super) mod validate;
pub use text::{SparseTextLimits, SparseTextStats};

#[cfg(all(target_arch = "wasm32", feature = "vello-webgl"))]
#[path = "webgl.rs"]
mod webgl;
#[cfg(all(target_arch = "wasm32", feature = "vello-webgl"))]
pub use webgl::{WebGlSession, WebGlSessionInit, WebGlSessionError, WebGlPreparedFrame};

mod images;

use images::{ImageState, VariantKey};
