// Copyright 2026 Mark Alan Boykin
// SPDX-License-Identifier: MPL-2.0

//! VB2 image/session acceptance. CPU tests never request a GPU device.
//! These synthetic semantic anchors do not constitute specimen-panel acceptance.

#![cfg(any(feature = "vello-cpu", feature = "vello-hybrid"))]

use netrender::vello_backends::{SparseResourceLimits, SparseResourceStats, SparseSessionError};
use netrender::{ImageData, NO_CLIP, Scene, SceneClip, SceneImage, SceneLayer, SceneOp, Transform};

const DIM: u32 = 48;
const KEY: u64 = 71;

trait Session {
    fn set(&mut self, key: u64, image: ImageData) -> Result<(), SparseSessionError>;
    fn remove(&mut self, key: u64) -> bool;
    fn stats(&self) -> SparseResourceStats;
    fn render(&mut self, scene: &Scene) -> Result<Vec<u8>, SparseSessionError>;
}

fn solid(color: [u8; 4]) -> ImageData {
    ImageData::from_bytes(2, 2, color.repeat(4))
}

fn image(scene: &mut Scene) -> &mut SceneImage {
    scene
        .ops
        .iter_mut()
        .find_map(|op| match op {
            SceneOp::Image(image) => Some(image),
            _ => None,
        })
        .expect("image fixture")
}

fn image_scene() -> Scene {
    let mut scene = Scene::new(DIM, DIM);
    scene.push_image_full(
        8.0,
        8.0,
        40.0,
        40.0,
        [0.0, 0.0, 1.0, 1.0],
        [1.0; 4],
        KEY,
        0,
        NO_CLIP,
    );
    scene
}

fn pixel(bytes: &[u8], x: u32, y: u32) -> [u8; 4] {
    bytes[((y * DIM + x) * 4) as usize..][..4]
        .try_into()
        .unwrap()
}

#[track_caller]
fn near(actual: [u8; 4], expected: [u8; 4], tolerance: u8) {
    assert!(
        actual
            .into_iter()
            .zip(expected)
            .all(|(a, b)| a.abs_diff(b) <= tolerance),
        "pixel {actual:?}, expected {expected:?}, tolerance {tolerance}"
    );
}

fn semantic_images(session: &mut impl Session) {
    // Each quadrant is independent: detects UV orientation as well as affine/clip errors.
    let mut quadrants = Vec::new();
    for y in 0..4 {
        for x in 0..4 {
            quadrants.extend_from_slice(&match (x < 2, y < 2) {
                (true, true) => [255, 0, 0, 255],
                (false, true) => [0, 255, 0, 255],
                (true, false) => [0, 0, 255, 255],
                (false, false) => [255, 255, 255, 255],
            });
        }
    }
    session
        .set(KEY, ImageData::from_bytes(4, 4, quadrants))
        .unwrap();
    let mut scene = image_scene();
    let transform = scene
        .push_transform(Transform::scale_2d(2.0, 2.0).then(&Transform::translate_2d(8.0, 8.0)));
    let img = image(&mut scene);
    (img.x0, img.y0, img.x1, img.y1) = (0.0, 0.0, 16.0, 16.0);
    img.transform_id = transform;
    img.nearest = true;
    img.clip_rect = [16.0, 12.0, 32.0, 36.0];
    let bytes = session.render(&scene).unwrap();
    near(pixel(&bytes, 20, 16), [255, 0, 0, 255], 2);
    near(pixel(&bytes, 28, 16), [0, 255, 0, 255], 2);
    near(pixel(&bytes, 20, 30), [0, 0, 255, 255], 2);
    near(pixel(&bytes, 28, 30), [255; 4], 2);
    for (x, y) in [(12, 16), (34, 16), (20, 10), (20, 38)] {
        assert_eq!(pixel(&bytes, x, y)[3], 0, "device-space clip at {x},{y}");
    }

    // A sprite subrect must not pick up adjacent red/blue texels at its borders.
    session
        .set(
            KEY,
            ImageData::from_bytes(
                4,
                1,
                vec![
                    255, 0, 0, 255, 0, 255, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255,
                ],
            ),
        )
        .unwrap();
    let mut scene = image_scene();
    image(&mut scene).uv = [0.25, 0.0, 0.75, 1.0];
    image(&mut scene).clamp_to_uv = true;
    let clamped = session.render(&scene).unwrap();
    for x in [8, 9, 24, 38, 39] {
        near(pixel(&clamped, x, 24), [0, 255, 0, 255], 3);
    }
    image(&mut scene).clamp_to_uv = false;
    let unclamped = session.render(&scene).unwrap();
    assert!(
        pixel(&unclamped, 8, 24)[0] > 40,
        "negative control: adjacent red texel contributes"
    );
    assert!(
        pixel(&unclamped, 39, 24)[2] > 40,
        "negative control: adjacent blue texel contributes"
    );

    // The center seam distinguishes the two samplers without byte-wide raster comparisons.
    session
        .set(
            KEY,
            ImageData::from_bytes(2, 1, vec![255, 0, 0, 255, 0, 0, 255, 255]),
        )
        .unwrap();
    let mut scene = image_scene();
    image(&mut scene).nearest = true;
    let nearest = session.render(&scene).unwrap();
    near(pixel(&nearest, 23, 24), [255, 0, 0, 255], 2);
    near(pixel(&nearest, 24, 24), [0, 0, 255, 255], 2);
    image(&mut scene).nearest = false;
    let bilinear = session.render(&scene).unwrap();
    let seam = pixel(&bilinear, 23, 24);
    assert!(
        seam[0] > 80 && seam[2] > 80 && seam[3] > 250,
        "bilinear seam {seam:?}"
    );

    // Premultiplied tint and straight source alpha have separate contributions.
    // An opaque black backdrop makes the result independent of output alpha storage convention.
    session.set(KEY, solid([255, 255, 255, 128])).unwrap();
    let mut scene = Scene::new(DIM, DIM);
    scene.push_rect(0.0, 0.0, DIM as f32, DIM as f32, [0.0, 0.0, 0.0, 1.0]);
    scene.push_image_full(
        8.0,
        8.0,
        40.0,
        40.0,
        [0.0, 0.0, 1.0, 1.0],
        [0.25, 0.0, 0.125, 0.5],
        KEY,
        0,
        NO_CLIP,
    );
    scene.push_rect(20.0, 20.0, 28.0, 28.0, [0.0, 1.0, 0.0, 1.0]);
    let tinted = session.render(&scene).unwrap();
    near(pixel(&tinted, 12, 24), [32, 0, 16, 255], 3);
    near(pixel(&tinted, 24, 24), [0, 255, 0, 255], 2);
    near(pixel(&tinted, 4, 24), [0, 0, 0, 255], 2);

    session.set(KEY, solid([255; 4])).unwrap();
    let mut scene = Scene::new(DIM, DIM);
    let mut layer = SceneLayer::alpha(0.5);
    layer.clip = SceneClip::Rect {
        rect: [8.0, 8.0, 40.0, 40.0],
        radii: [12.0; 4],
    };
    scene.push_layer(layer);
    scene.push_image_full(
        0.0,
        0.0,
        48.0,
        48.0,
        [0.0, 0.0, 1.0, 1.0],
        [1.0; 4],
        KEY,
        0,
        NO_CLIP,
    );
    scene.pop_layer();
    let layered = session.render(&scene).unwrap();
    assert!(pixel(&layered, 24, 24)[3].abs_diff(128) <= 3);
    assert_eq!(pixel(&layered, 8, 8)[3], 0, "rounded corner removes image");
    assert_eq!(pixel(&layered, 4, 24)[3], 0, "layer clip removes image");
}

fn source_lifecycle(session: &mut impl Session) {
    let original = solid([255, 0, 0, 255]);
    let mut scene = image_scene();
    scene.image_sources.insert(KEY, original.clone());
    near(
        pixel(&session.render(&scene).unwrap(), 24, 24),
        [255, 0, 0, 255],
        2,
    );
    let first = session.stats();
    session.set(KEY, original).unwrap();
    scene.image_sources.clear();
    // Omission is not deletion, and the same blob does not hydrate/upload again.
    near(
        pixel(&session.render(&scene).unwrap(), 24, 24),
        [255, 0, 0, 255],
        2,
    );
    let repeated = session.stats();
    assert_eq!(repeated.hydrated_images, first.hydrated_images);
    assert_eq!(repeated.uploaded_images, first.uploaded_images);

    for index in 0..12 {
        let color = if index % 2 == 0 {
            [0, 0, 255, 255]
        } else {
            [0, 255, 0, 255]
        };
        session.set(KEY, solid(color)).unwrap();
        near(pixel(&session.render(&scene).unwrap(), 24, 24), color, 2);
        let stats = session.stats();
        assert_eq!(stats.source_count, 1);
        assert_eq!(stats.source_bytes, 16);
        assert_eq!(stats.cached_images, 1);
        assert_eq!(stats.cached_bytes, 16);
        assert!(stats.gpu_texture_bytes <= 16);
    }
    assert!(session.remove(KEY));
    assert!(!session.remove(KEY));
    let removed = session.stats();
    assert_eq!(
        (
            removed.source_count,
            removed.cached_images,
            removed.cached_bytes,
            removed.gpu_texture_bytes
        ),
        (0, 0, 0, 0)
    );
    assert!(matches!(
        session.render(&scene),
        Err(SparseSessionError::MissingImage { key: KEY, .. })
    ));
    session.set(KEY, solid([255, 255, 0, 255])).unwrap();
    near(
        pixel(&session.render(&scene).unwrap(), 24, 24),
        [255, 255, 0, 255],
        2,
    );

    // Failed replacements and malformed scene overrides must preserve the prior source.
    assert!(matches!(
        session.set(KEY, ImageData::from_bytes(2, 2, vec![0; 15])),
        Err(SparseSessionError::InvalidImage { key: KEY, .. })
    ));
    let before = session.stats();
    scene
        .image_sources
        .insert(KEY, ImageData::from_bytes(0, 1, vec![]));
    assert!(matches!(
        session.render(&scene),
        Err(SparseSessionError::InvalidImage { key: KEY, .. })
    ));
    let after = session.stats();
    assert_eq!(
        (
            after.source_count,
            after.cached_images,
            after.hydrated_images,
            after.uploaded_images
        ),
        (
            before.source_count,
            before.cached_images,
            before.hydrated_images,
            before.uploaded_images
        )
    );
    scene.image_sources.clear();
    near(
        pixel(&session.render(&scene).unwrap(), 24, 24),
        [255, 255, 0, 255],
        2,
    );

    let mut projective = scene.clone();
    let mut matrix = Transform::IDENTITY;
    matrix.m[3] = 0.1;
    image(&mut projective).transform_id = projective.push_transform(matrix);
    assert!(matches!(
        session.render(&projective),
        Err(SparseSessionError::InvalidScene { .. })
    ));
    let mut invalid_uv = scene.clone();
    image(&mut invalid_uv).uv[2] = f32::NAN;
    assert!(matches!(
        session.render(&invalid_uv),
        Err(SparseSessionError::InvalidScene { .. })
    ));
    let mut filtered = Scene::new(DIM, DIM);
    let mut layer = SceneLayer::alpha(1.0);
    layer.filters.push(netrender::SceneFilter::Blur(2.0));
    filtered.push_layer(layer);
    filtered.pop_layer();
    assert!(matches!(
        session.render(&filtered),
        Err(SparseSessionError::Admission(_))
    ));
}

fn source_budget(session: &mut impl Session) {
    session.set(KEY, solid([255, 0, 0, 255])).unwrap();
    assert!(matches!(
        session.set(KEY + 1, solid([0, 255, 0, 255])),
        Err(SparseSessionError::ResourceBudgetExceeded { .. })
    ));
    let scene = image_scene();
    near(
        pixel(&session.render(&scene).unwrap(), 24, 24),
        [255, 0, 0, 255],
        2,
    );
    // Replacing at capacity must release the previous derived image, not require a spare slot.
    session.set(KEY, solid([0, 0, 255, 255])).unwrap();
    near(
        pixel(&session.render(&scene).unwrap(), 24, 24),
        [0, 0, 255, 255],
        2,
    );
    assert_eq!(session.stats().cached_images, 1);
    let mut two_variants = scene.clone();
    two_variants.push_image_full(
        8.0,
        8.0,
        20.0,
        20.0,
        [0.0, 0.0, 1.0, 1.0],
        [0.5, 0.5, 0.5, 0.5],
        KEY,
        0,
        NO_CLIP,
    );
    assert!(matches!(
        session.render(&two_variants),
        Err(SparseSessionError::ResourceBudgetExceeded { .. })
    ));
    assert_eq!(session.stats().cached_images, 1);
    near(
        pixel(&session.render(&scene).unwrap(), 24, 24),
        [0, 0, 255, 255],
        2,
    );
}

#[cfg(feature = "vello-cpu")]
mod cpu {
    use super::*;
    use netrender::vello_backends::CpuSession;

    impl Session for CpuSession {
        fn set(&mut self, key: u64, image: ImageData) -> Result<(), SparseSessionError> {
            self.set_image_source(key, image)
        }
        fn remove(&mut self, key: u64) -> bool {
            self.remove_image_source(key)
        }
        fn stats(&self) -> SparseResourceStats {
            CpuSession::stats(self)
        }
        fn render(&mut self, scene: &Scene) -> Result<Vec<u8>, SparseSessionError> {
            CpuSession::render(self, scene).map(|pixels| pixels.data_as_u8_slice().to_vec())
        }
    }

    #[test]
    fn cpu_image_semantics_without_device() {
        semantic_images(&mut CpuSession::new(SparseResourceLimits::default()));
    }

    #[test]
    fn cpu_source_lifecycle_without_device() {
        source_lifecycle(&mut CpuSession::new(SparseResourceLimits::default()));
    }

    #[test]
    fn cpu_resource_budget_without_device() {
        source_budget(&mut CpuSession::new(SparseResourceLimits {
            max_sources: 1,
            max_source_bytes: 16,
            max_cached_images: 1,
            max_cached_bytes: 16,
            ..Default::default()
        }));
    }
}

#[cfg(all(feature = "vello-hybrid", not(target_arch = "wasm32")))]
mod hybrid {
    use super::*;
    use netrender::vello_backends::HybridSession;

    struct Gpu {
        session: HybridSession,
        handles: netrender::WgpuHandles,
        readback: netrender::WgpuDevice,
        texture: wgpu::Texture,
    }

    impl Gpu {
        fn new(limits: SparseResourceLimits) -> Self {
            let handles = netrender::boot().expect("VB2 shared device");
            let info = handles.adapter.get_info();
            eprintln!(
                "VB2 Hybrid adapter={:?} backend={:?} driver={:?}",
                info.name, info.backend, info.driver
            );
            let session = HybridSession::new(&handles.device, &handles.queue, limits).unwrap();
            let readback = netrender::WgpuDevice::with_external(handles.clone()).unwrap();
            let texture = handles.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("VB2 image acceptance target"),
                size: wgpu::Extent3d {
                    width: DIM,
                    height: DIM,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::COPY_SRC
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            Self {
                session,
                handles,
                readback,
                texture,
            }
        }
    }

    impl Session for Gpu {
        fn set(&mut self, key: u64, image: ImageData) -> Result<(), SparseSessionError> {
            self.session.set_image_source(key, image)
        }
        fn remove(&mut self, key: u64) -> bool {
            self.session.remove_image_source(key)
        }
        fn stats(&self) -> SparseResourceStats {
            self.session.stats()
        }
        fn render(&mut self, scene: &Scene) -> Result<Vec<u8>, SparseSessionError> {
            let before = self.readback.read_rgba8_texture(&self.texture, DIM, DIM);
            let mut encoder =
                self.handles
                    .device
                    .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                        label: Some("VB2 image session render"),
                    });
            let result = self.session.render(
                scene,
                &self.handles.device,
                &self.handles.queue,
                &mut encoder,
                &self.texture,
            );
            // Submit even on refusal: catches clear/draw commands accidentally encoded before preflight.
            self.handles.queue.submit([encoder.finish()]);
            let after = self.readback.read_rgba8_texture(&self.texture, DIM, DIM);
            if result.is_err() {
                assert_eq!(
                    after, before,
                    "typed refusal must leave the visible target unchanged"
                );
            }
            result.map(|()| after)
        }
    }

    #[test]
    fn hybrid_images_on_existing_device() {
        let mut gpu = Gpu::new(SparseResourceLimits::default());
        semantic_images(&mut gpu);
        gpu.session.clear_images();
        source_lifecycle(&mut gpu);
    }

    #[test]
    fn hybrid_image_budget_on_existing_device() {
        source_budget(&mut Gpu::new(SparseResourceLimits {
            max_sources: 1,
            max_source_bytes: 16,
            max_cached_images: 1,
            max_cached_bytes: 16,
            ..Default::default()
        }));
    }
}
