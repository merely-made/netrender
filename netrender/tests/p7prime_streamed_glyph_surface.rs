// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

use std::sync::Arc;

use netrender::{
    ColorLoad, FontBlob, Glyph, NetrenderOptions, Scene, boot, create_netrender_instance,
    peniko::Blob,
};

const DIM: u32 = 256;

fn system_font() -> Option<Arc<Vec<u8>>> {
    [
        r"C:\Windows\Fonts\arial.ttf",
        r"C:\Windows\Fonts\segoeui.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
    ]
    .into_iter()
    .find_map(|path| std::fs::read(path).ok().map(Arc::new))
}

fn scene(font: Arc<Vec<u8>>, baselines: &[f32]) -> Scene {
    let mut scene = Scene::new(DIM, DIM);
    let font = scene.push_font(FontBlob {
        data: Blob::new(font),
        index: 0,
    });
    for &y in baselines {
        scene.push_glyph_run(
            font,
            24.0,
            (0..5)
                .map(|i| Glyph {
                    id: 36 + i,
                    x: 20.0 + i as f32 * 22.0,
                    y,
                })
                .collect(),
            [1.0, 1.0, 1.0, 1.0],
        );
    }
    scene
}

fn target(device: &wgpu::Device) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("streamed glyph surface"),
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

fn painted_in_band(bytes: &[u8], top: u32, bottom: u32) -> usize {
    (top..bottom)
        .flat_map(|y| (0..DIM).map(move |x| ((y * DIM + x) * 4) as usize))
        .filter(|&i| bytes[i] > 16 || bytes[i + 1] > 16 || bytes[i + 2] > 16)
        .count()
}

#[test]
fn keyed_surface_rebuild_keeps_prefix_glyphs_when_tail_is_added() {
    let Some(font) = system_font() else {
        eprintln!("no known system font; skipping streamed glyph GPU receipt");
        return;
    };
    let handles = boot().expect("wgpu boot");
    let renderer = create_netrender_instance(
        handles.clone(),
        NetrenderOptions {
            tile_cache_size: Some(DIM),
            enable_vello: true,
            ..Default::default()
        },
    )
    .expect("renderer");
    let surface = 17;

    let prefix = scene(font.clone(), &[48.0, 88.0]);
    let (prefix_target, prefix_view) = target(&handles.device);
    renderer.render_vello_scaled_for(
        surface,
        &prefix,
        &prefix_view,
        ColorLoad::Clear(wgpu::Color::BLACK),
        1.0,
    );
    let prefix_bytes = renderer
        .wgpu_device
        .read_rgba8_texture(&prefix_target, DIM, DIM);
    assert!(painted_in_band(&prefix_bytes, 20, 120) > 100);

    assert!(renderer.invalidate_surface_tiles(surface));
    // A document relayout rebuilds its font sidecar, so the complete frame
    // carries equivalent bytes under a fresh blob identity.
    let complete = scene(Arc::new((*font).clone()), &[48.0, 88.0, 158.0, 208.0]);
    let (complete_target, complete_view) = target(&handles.device);
    renderer.render_vello_scaled_for(
        surface,
        &complete,
        &complete_view,
        ColorLoad::Clear(wgpu::Color::BLACK),
        1.0,
    );
    let complete_bytes = renderer
        .wgpu_device
        .read_rgba8_texture(&complete_target, DIM, DIM);

    assert!(
        painted_in_band(&complete_bytes, 20, 120) > 100,
        "the full replacement lost its unchanged prefix glyphs"
    );
    assert!(
        painted_in_band(&complete_bytes, 130, 235) > 100,
        "the full replacement did not paint its new tail glyphs"
    );
}
