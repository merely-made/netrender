// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Element `filter` color-matrix receipt.
//!
//! This is intentionally a Scene-level receipt: a layer carrying a non-blur
//! CSS filter must travel through the renderer's element-filter preprocessing
//! and produce observable pixels, rather than only exercising the graph helper
//! in isolation.

use netrender::scene::{Scene, SceneFilter, SceneLayer};
use netrender::{ColorLoad, NetrenderOptions, boot, create_netrender_instance};

const DIM: u32 = 64;

fn make_target(device: &wgpu::Device) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("pd2 element filter target"),
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
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor {
        format: Some(wgpu::TextureFormat::Rgba8Unorm),
        ..Default::default()
    });
    (texture, view)
}

fn pixel(bytes: &[u8], x: u32, y: u32) -> [u8; 4] {
    let i = ((y * DIM + x) * 4) as usize;
    [bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]
}

/// `Invert(1)` is a per-pixel color-matrix filter. A red layer therefore
/// becomes cyan, proving that the Scene layer path invokes the migrated
/// single-pass color-matrix graph and composites its output.
#[test]
fn pd2_element_filter_color_matrix_changes_scene_pixels() {
    let handles = boot().expect("wgpu boot");
    let renderer = create_netrender_instance(
        handles.clone(),
        NetrenderOptions {
            tile_cache_size: Some(DIM),
            enable_vello: true,
            ..Default::default()
        },
    )
    .expect("create_netrender_instance");

    let mut scene = Scene::new(DIM, DIM);
    let mut layer = SceneLayer::alpha(1.0);
    layer.filters.push(SceneFilter::Invert(1.0));
    scene.push_layer(layer);
    scene.push_rect(0.0, 0.0, DIM as f32, DIM as f32, [1.0, 0.0, 0.0, 1.0]);
    scene.pop_layer();

    let (target, view) = make_target(&handles.device);
    renderer.render_vello(&scene, &view, ColorLoad::Clear(wgpu::Color::BLACK));
    let bytes = renderer.wgpu_device.read_rgba8_texture(&target, DIM, DIM);

    for &(x, y) in &[(8, 8), (32, 32), (56, 56)] {
        let p = pixel(&bytes, x, y);
        assert!(
            p[0] < 8 && p[1] > 247 && p[2] > 247 && p[3] > 247,
            "Invert(1) at ({x}, {y}) should turn opaque red into cyan, got {p:?}"
        );
    }
}
