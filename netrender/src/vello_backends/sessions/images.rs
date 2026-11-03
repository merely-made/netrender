// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) struct VariantKey {
    pub(super) key: ImageKey,
    pub(super) blob: u64,
    pub(super) source_size: [u32; 2],
    pub(super) crop: [u32; 4],
    pub(super) tint: [u32; 4],
}

impl VariantKey {
    pub(super) fn width(self) -> u16 {
        (self.crop[2] - self.crop[0]) as u16
    }
    pub(super) fn height(self) -> u16 {
        (self.crop[3] - self.crop[1]) as u16
    }
    pub(super) fn bytes(self) -> usize {
        self.width() as usize * self.height() as usize * 4
    }
}

pub(super) struct CachedImage {
    pub(super) pixmap: Arc<Pixmap>,
    #[cfg(feature = "vello-hybrid")]
    pub(super) texture: Option<wgpu::Texture>,
}

pub(super) struct ImageState {
    pub(super) limits: SparseResourceLimits,
    pub(super) max_dimension: u32,
    pub(super) sources: HashMap<ImageKey, ImageData>,
    pub(super) cache: HashMap<VariantKey, CachedImage>,
    pub(super) hydrated: u64,
    pub(super) uploaded: u64,
}

pub(super) struct FramePlan {
    pub(super) sources: HashMap<ImageKey, ImageData>,
    pub(super) variants: HashMap<VariantKey, ImageData>,
    pub(super) ops: HashMap<usize, VariantKey>,
}

impl ImageState {
    pub(super) fn new(limits: SparseResourceLimits, max_dimension: u32) -> Self {
        Self {
            limits,
            max_dimension,
            sources: HashMap::new(),
            cache: HashMap::new(),
            hydrated: 0,
            uploaded: 0,
        }
    }

    pub(super) fn updated_sources<'a>(
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

    pub(super) fn set_image_source(
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

    pub(super) fn remove_image_source(&mut self, key: ImageKey) -> bool {
        self.cache.retain(|variant, _| variant.key != key);
        self.sources.remove(&key).is_some()
    }

    pub(super) fn clear_images(&mut self) {
        self.cache.clear();
        self.sources.clear();
    }

    pub(super) fn stats(&self) -> SparseResourceStats {
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

    pub(super) fn plan(
        &self,
        scene: &Scene,
        backend: VelloBackend,
    ) -> Result<FramePlan, SparseSessionError> {
        super::super::validate_scene_operations(backend, scene, true)?;
        validate::scene(scene, self.limits)?;
        if matches!(backend, VelloBackend::Hybrid | VelloBackend::WebGl2) {
            for (index, op) in scene.ops.iter().enumerate() {
                if matches!(op, SceneOp::Pattern(pattern) if !pattern.nearest) {
                    return Err(super::super::unsupported(
                        backend,
                        index,
                        "Pattern",
                        "pinned GPU bilinear repeat clamps filter taps at tile seams; only explicit nearest sampling is admitted",
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

    pub(super) fn hydrate(&mut self, plan: &FramePlan) {
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
