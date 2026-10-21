// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! IMG-GPU-REPLACE-STALE receipts: a new `peniko::Blob` under a continuously
//! present `ImageKey` must draw its new pixels through the tile path and the
//! retained-fragment path, rebuilding only the tiles / fragments that draw it.

use netrender::scene::{SceneFragment, Transform};
use netrender::{
    boot, create_netrender_instance, ColorLoad, ImageData, NetrenderOptions, Renderer, Scene,
};

const DIM: u32 = 128;
const TILE: u32 = 64;
const KEY: u64 = 0x1ced;

fn renderer(handles: &netrender::WgpuHandles) -> Renderer {
    create_netrender_instance(
        handles.clone(),
        NetrenderOptions {
            tile_cache_size: Some(TILE),
            enable_vello: true,
            ..Default::default()
        },
    )
    .expect("create_netrender_instance")
}

fn render_bytes(r: &Renderer, handles: &netrender::WgpuHandles, scene: &Scene) -> Vec<u8> {
    let texture = handles.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("img replace target"),
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
    r.render_vello(scene, &view, ColorLoad::Clear(wgpu::Color::BLACK));
    r.wgpu_device.read_rgba8_texture(&texture, DIM, DIM)
}

fn sample(bytes: &[u8], x: usize, y: usize) -> [u8; 4] {
    let offset = (y * DIM as usize + x) * 4;
    <[u8; 4]>::try_from(&bytes[offset..offset + 4]).unwrap()
}

fn solid(rgba: [u8; 4]) -> ImageData {
    ImageData::from_bytes(1, 1, rgba.to_vec())
}

#[test]
fn tile_path_draws_replacement_and_rebuilds_only_dependent_tiles() {
    let handles = boot().expect("wgpu boot");
    let r = renderer(&handles);
    // Image in tile (0,0); an unrelated rect in tile (1,1).
    let red = solid([255, 0, 0, 255]);
    let blue = solid([0, 0, 255, 255]);
    let frame = |image: &ImageData| {
        let mut scene = Scene::new(DIM, DIM);
        scene.push_image(8.0, 8.0, 40.0, 40.0, KEY, image.clone());
        scene.push_rect(72.0, 72.0, 120.0, 120.0, [0.0, 1.0, 0.0, 1.0]);
        scene
    };
    let first = render_bytes(&r, &handles, &frame(&red));
    assert_eq!(sample(&first, 24, 24), [255, 0, 0, 255]);
    let steady = render_bytes(&r, &handles, &frame(&red));
    assert_eq!(sample(&steady, 24, 24), [255, 0, 0, 255]);

    // Same key, same ops, fresh blob: the pixels must follow.
    let replaced = render_bytes(&r, &handles, &frame(&blue));
    assert_eq!(sample(&replaced, 24, 24), [0, 0, 255, 255], "stale pixels");
    assert_eq!(sample(&replaced, 96, 96), [0, 255, 0, 255], "bystander tile");
    let dirty = r.vello_last_dirty_count().unwrap();
    assert_eq!(dirty, 1, "only the image's tile was rebuilt");
}

#[test]
fn retained_fragment_redraws_replaced_image_under_a_reused_key() {
    let handles = boot().expect("wgpu boot");
    let r = renderer(&handles);
    let mut fragment_scene = Scene::new(DIM, DIM);
    fragment_scene.push_image(0.0, 0.0, 32.0, 32.0, KEY, solid([255, 0, 0, 255]));
    let mut fragment = SceneFragment::from_scene(fragment_scene);
    // The placing scene owns the pixels; the fragment only names the key.
    fragment.image_sources.clear();
    let id = r.register_fragment(fragment).unwrap();
    let red = solid([255, 0, 0, 255]);
    let blue = solid([0, 0, 255, 255]);
    let frame = |image: &ImageData| {
        let mut scene = Scene::new(DIM, DIM);
        scene.set_image_source(KEY, image.clone());
        scene.place_fragment(id, Transform::translate_2d(32.0, 32.0));
        scene
    };
    let first = render_bytes(&r, &handles, &frame(&red));
    assert_eq!(sample(&first, 40, 40), [255, 0, 0, 255]);
    let lowers = r.fragment_lower_count().unwrap();
    let again = render_bytes(&r, &handles, &frame(&red));
    assert_eq!(sample(&again, 40, 40), [255, 0, 0, 255]);
    assert_eq!(r.fragment_lower_count(), Some(lowers), "same blob: cached");

    let replaced = render_bytes(&r, &handles, &frame(&blue));
    assert_eq!(sample(&replaced, 40, 40), [0, 0, 255, 255], "stale fragment");
    assert_eq!(r.fragment_lower_count(), Some(lowers + 1));
}
