// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Roadmap D1 — backdrop-filter receipts.
//!
//! Two receipt classes:
//!
//! 1. **Pure CPU**: API surface — `SceneFilter` enum, the
//!    `backdrop_filter` field on `SceneLayer`, and tile-cache hash
//!    distinguishes filtered from unfiltered layers.
//! 2. **GPU smoke**: render a scene with sharp-edged content
//!    underneath a layer carrying `Blur(8.0)` backdrop filter; read
//!    pixels back; verify the post-filter region is **smoother**
//!    (lower local variance) than the same region without the
//!    filter. The receipt the roadmap calls for: "frosted-glass nav
//!    bar over a busy background."

use netrender::scene::{Scene, SceneClip, SceneFilter, SceneLayer, SceneOp};
use netrender::tile_cache::TileCache;

const TILE: u32 = 32;

#[test]
fn pd1_backdrop_filter_default_is_none() {
    let layer = SceneLayer::clip(SceneClip::None);
    assert!(
        layer.backdrop_filter.is_none(),
        "no backdrop filter by default"
    );
    let layer = SceneLayer::alpha(0.5);
    assert!(layer.backdrop_filter.is_none());
}

#[test]
fn pd1_setting_backdrop_filter_invalidates_tile_hash() {
    let mut scene = Scene::new(64, 64);
    scene.push_layer(SceneLayer::clip(SceneClip::Rect {
        rect: [0.0, 0.0, 32.0, 32.0],
        radii: [0.0; 4],
    }));
    scene.push_rect(0.0, 0.0, 32.0, 32.0, [1.0, 0.0, 0.0, 1.0]);
    scene.ops.push(SceneOp::PopLayer);

    let mut cache = TileCache::new(TILE);
    let _ = cache.invalidate(&scene);
    let _ = cache.invalidate(&scene);

    if let SceneOp::PushLayer(l) = &mut scene.ops[0] {
        l.backdrop_filter = Some(SceneFilter::Blur(8.0));
    }
    let dirty = cache.invalidate(&scene);
    assert!(
        !dirty.is_empty(),
        "setting backdrop_filter invalidates: {} dirty",
        dirty.len()
    );
}

#[test]
fn pd1_changing_blur_radius_invalidates_tile_hash() {
    let mut scene = Scene::new(64, 64);
    let mut l = SceneLayer::clip(SceneClip::Rect {
        rect: [0.0, 0.0, 32.0, 32.0],
        radii: [0.0; 4],
    });
    l.backdrop_filter = Some(SceneFilter::Blur(4.0));
    scene.push_layer(l);
    scene.push_rect(0.0, 0.0, 32.0, 32.0, [1.0, 0.0, 0.0, 1.0]);
    scene.ops.push(SceneOp::PopLayer);

    let mut cache = TileCache::new(TILE);
    let _ = cache.invalidate(&scene);
    let _ = cache.invalidate(&scene);

    if let SceneOp::PushLayer(l) = &mut scene.ops[0] {
        l.backdrop_filter = Some(SceneFilter::Blur(16.0));
    }
    let dirty = cache.invalidate(&scene);
    assert!(!dirty.is_empty(), "blur radius change invalidates");
}

// ── GPU smoke ─────────────────────────────────────────────────────────

mod gpu_smoke {
    use netrender::scene::{Scene, SceneClip, SceneFilter, SceneLayer, SceneOp};
    use netrender::{boot, create_netrender_instance, ColorLoad, NetrenderOptions};

    const DIM: u32 = 256;
    const TILE_SIZE: u32 = 64;

    fn make_target(device: &wgpu::Device) -> (wgpu::Texture, wgpu::TextureView) {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("pd1 target"),
            size: wgpu::Extent3d {
                width: DIM,
                height: DIM,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::STORAGE_BINDING
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[wgpu::TextureFormat::Rgba8UnormSrgb],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor {
            format: Some(wgpu::TextureFormat::Rgba8Unorm),
            ..Default::default()
        });
        (texture, view)
    }

    /// Build a "busy" scene: alternating colored stripes covering
    /// the full canvas. This produces sharp edges that any blur
    /// will visibly smooth.
    fn busy_scene() -> Scene {
        let mut scene = Scene::new(DIM, DIM);
        for i in 0..16 {
            let x0 = i as f32 * (DIM as f32 / 16.0);
            let x1 = x0 + DIM as f32 / 16.0;
            let color = if i % 2 == 0 {
                [1.0, 0.0, 0.0, 1.0]
            } else {
                [0.0, 0.0, 1.0, 1.0]
            };
            scene.push_rect(x0, 0.0, x1, DIM as f32, color);
        }
        scene
    }

    /// Sample local horizontal variance on a scanline through `y`,
    /// across the column range `[x0, x1]`. Each pixel is compared
    /// to its left neighbor; large per-pixel deltas mean sharp
    /// edges (high variance), small deltas mean smooth.
    fn local_variance(bytes: &[u8], y: u32, x0: u32, x1: u32) -> f64 {
        let row_start = (y * DIM * 4) as usize;
        let scanline = &bytes[row_start..row_start + (DIM * 4) as usize];
        let mut total: f64 = 0.0;
        let mut count: u32 = 0;
        for x in x0..x1 {
            if x == 0 {
                continue;
            }
            let curr = (x * 4) as usize;
            let prev = ((x - 1) * 4) as usize;
            let dr = scanline[curr] as i32 - scanline[prev] as i32;
            let dg = scanline[curr + 1] as i32 - scanline[prev + 1] as i32;
            let db = scanline[curr + 2] as i32 - scanline[prev + 2] as i32;
            total += (dr * dr + dg * dg + db * db) as f64;
            count += 1;
        }
        if count == 0 {
            0.0
        } else {
            total / count as f64
        }
    }

    #[test]
    fn pd1_backdrop_blur_smooths_busy_background() {
        let handles = boot().expect("wgpu boot");
        let renderer = create_netrender_instance(
            handles.clone(),
            NetrenderOptions {
                tile_cache_size: Some(TILE_SIZE),
                enable_vello: true,
                ..Default::default()
            },
        )
        .expect("create_netrender_instance");

        // Reference: busy stripes, no backdrop filter.
        let reference_scene = busy_scene();
        let (ref_target, ref_view) = make_target(&handles.device);
        renderer.render_vello(
            &reference_scene,
            &ref_view,
            ColorLoad::Clear(wgpu::Color::BLACK),
        );
        let ref_bytes = renderer
            .wgpu_device
            .read_rgba8_texture(&ref_target, DIM, DIM);

        // Filtered: same stripes, with a layer carrying backdrop
        // Blur(12) covering a horizontal band. The band should
        // appear smoother (lower local variance) than the same
        // x-range in the reference.
        let mut filtered = busy_scene();
        let mut layer = SceneLayer::clip(SceneClip::Rect {
            rect: [16.0, 100.0, (DIM - 16) as f32, 156.0],
            radii: [0.0; 4],
        });
        layer.backdrop_filter = Some(SceneFilter::Blur(12.0));
        filtered.push_layer(layer);
        // No content inside the layer — just want the blurred
        // backdrop to show through.
        filtered.ops.push(SceneOp::PopLayer);

        let (filt_target, filt_view) = make_target(&handles.device);
        renderer.render_vello(&filtered, &filt_view, ColorLoad::Clear(wgpu::Color::BLACK));
        let filt_bytes = renderer
            .wgpu_device
            .read_rgba8_texture(&filt_target, DIM, DIM);

        let band_y = 128_u32;
        let ref_var = local_variance(&ref_bytes, band_y, 32, DIM - 32);
        let filt_var = local_variance(&filt_bytes, band_y, 32, DIM - 32);

        eprintln!(
            "pd1: reference variance={:.1}, filtered variance={:.1}",
            ref_var, filt_var
        );

        assert!(
            ref_var > 1000.0,
            "reference busy stripes should have high variance, got {ref_var}"
        );
        assert!(
            filt_var < ref_var * 0.5,
            "backdrop-filtered band should have <50% the variance of the reference: ref={ref_var:.1}, filt={filt_var:.1}"
        );
    }

    #[test]
    fn staged_canvas_backdrop_blurs_and_tracks_generation() {
        const HEIGHT: u32 = 192;
        use netrender::{external_image_key, SourceAlpha, NO_CLIP};

        let handles = boot().expect("wgpu boot");
        let renderer = create_netrender_instance(
            handles.clone(),
            NetrenderOptions {
                tile_cache_size: Some(TILE_SIZE),
                enable_vello: true,
                ..Default::default()
            },
        )
        .expect("classic renderer");
        let source = handles.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("canvas producer"),
            size: wgpu::Extent3d {
                width: DIM,
                height: HEIGHT,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let source_view = source.create_view(&Default::default());
        let key = external_image_key(817);
        let mut scene = Scene::new(DIM, HEIGHT);
        scene.push_image_full(
            0.0,
            0.0,
            DIM as f32,
            HEIGHT as f32,
            [0.0, 0.0, 1.0, 1.0],
            [1.0; 4],
            key,
            0,
            NO_CLIP,
        );
        let mut pane = SceneLayer::clip(SceneClip::Rect {
            rect: [32.0, 64.0, 224.0, 160.0],
            radii: [0.0; 4],
        });
        pane.backdrop_filter = Some(SceneFilter::BackdropBlurMirrored(40.0));
        scene.push_layer(pane);
        scene.pop_layer();
        // Later foreground crosses the pane and must retain its sharp edge.
        scene.push_rect(120.0, 112.0, 136.0, 144.0, [0.0, 1.0, 0.0, 1.0]);

        let render = |generation: u64, swap: bool| {
            let mut bytes = Vec::with_capacity((DIM * HEIGHT * 4) as usize);
            for _y in 0..HEIGHT {
                for x in 0..DIM {
                    let red = ((x / 16) % 2 == 0) ^ swap;
                    bytes.extend_from_slice(if red {
                        &[255, 0, 0, 255]
                    } else {
                        &[0, 0, 255, 255]
                    });
                }
            }
            handles.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &source,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &bytes,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(DIM * 4),
                    rows_per_image: Some(HEIGHT),
                },
                wgpu::Extent3d {
                    width: DIM,
                    height: HEIGHT,
                    depth_or_array_layers: 1,
                },
            );
            renderer.stage_external_image(
                key,
                &source_view,
                [DIM, HEIGHT],
                SourceAlpha::Premultiplied,
                generation,
            );
            let target = handles.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("non-square backdrop target"),
                size: wgpu::Extent3d {
                    width: DIM,
                    height: HEIGHT,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::STORAGE_BINDING
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let view = target.create_view(&Default::default());
            renderer.render_vello(&scene, &view, ColorLoad::Clear(wgpu::Color::BLACK));
            renderer
                .wgpu_device
                .read_rgba8_texture(&target, DIM, HEIGHT)
        };
        let first = render(1, false);
        let second = render(2, true);
        let pixel = |bytes: &[u8], x: u32, y: u32| -> [u8; 4] {
            let i = ((y * DIM + x) * 4) as usize;
            bytes[i..i + 4].try_into().unwrap()
        };
        for (generation, bytes) in [(1, &first), (2, &second)] {
            let outside = local_variance(bytes, 32, 48, 208);
            let inside = local_variance(bytes, 96, 48, 208);
            assert!(
                outside > 1000.0 && inside < outside * 0.5,
                "generation {generation}: outside variance {outside}, inside {inside}"
            );
            assert_eq!(pixel(bytes, 120, 128), [0, 255, 0, 255], "foreground edge");
            assert_ne!(
                pixel(bytes, 119, 128),
                [0, 255, 0, 255],
                "foreground must start sharply"
            );
        }
        assert_ne!(
            pixel(&first, 40, 32),
            pixel(&second, 40, 32),
            "producer changed outside pane"
        );
        assert_ne!(
            pixel(&first, 40, 96),
            pixel(&second, 40, 96),
            "backdrop must repaint when producer generation changes"
        );
    }

    #[test]
    fn mirrored_backdrop_does_not_sample_outside_pane() {
        let handles = boot().expect("wgpu boot");
        let renderer = create_netrender_instance(
            handles.clone(),
            NetrenderOptions {
                tile_cache_size: Some(TILE_SIZE),
                enable_vello: true,
                ..Default::default()
            },
        )
        .expect("classic renderer");
        let render = |filter, patterned| {
            let mut scene = Scene::new(DIM, DIM);
            if patterned {
                scene.push_rect(0.0, 0.0, DIM as f32, DIM as f32, [1.0, 0.0, 0.0, 1.0]);
                scene.push_rect(68.0, 64.0, 192.0, 192.0, [0.0, 0.0, 1.0, 1.0]);
            } else {
                scene.push_rect(0.0, 0.0, DIM as f32, DIM as f32, [0.0, 0.0, 1.0, 1.0]);
                scene.push_rect(64.0, 64.0, 192.0, 192.0, [1.0, 0.0, 0.0, 1.0]);
            }
            let mut pane = SceneLayer::clip(SceneClip::Rect {
                rect: [64.0, 64.0, 192.0, 192.0],
                radii: [0.0; 4],
            });
            pane.backdrop_filter = Some(filter);
            scene.push_layer(pane);
            scene.pop_layer();
            let (target, view) = make_target(&handles.device);
            renderer.render_vello(&scene, &view, ColorLoad::Clear(wgpu::Color::BLACK));
            renderer.wgpu_device.read_rgba8_texture(&target, DIM, DIM)
        };
        let legacy = render(SceneFilter::Blur(12.0), false);
        let mirror = render(SceneFilter::BackdropBlurMirrored(12.0), false);
        let at = |bytes: &[u8], x: u32, y: u32| -> [u8; 4] {
            let i = ((y * DIM + x) * 4) as usize;
            bytes[i..i + 4].try_into().unwrap()
        };
        let old = at(&legacy, 65, 128);
        let new = at(&mirror, 65, 128);
        assert!(
            old[2] > 20,
            "legacy blur samples blue outside pane: {old:?}"
        );
        assert!(
            new[0] > 245 && new[2] < 10,
            "mirror keeps red pane edge: {new:?}"
        );
        let clamp_edge = at(&render(SceneFilter::Blur(12.0), true), 64, 128);
        let reflected_edge = at(
            &render(SceneFilter::BackdropBlurMirrored(12.0), true),
            64,
            128,
        );
        assert!(
            reflected_edge[2] > clamp_edge[2] + 10,
            "reflected interior must contribute at edge: mirror={reflected_edge:?}, legacy={clamp_edge:?}"
        );
        assert_eq!(
            at(&mirror, 63, 128),
            [0, 0, 255, 255],
            "pane leaves outside sharp"
        );
    }
}
