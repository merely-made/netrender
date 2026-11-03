// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! RG2a: the same two semantic scenes are admitted by all three Vello adapters.
//!
//! This corpus deliberately stops at geometry, transforms, clips, gradients,
//! and nested layers. Images, text, and retained fragments are refusal cases
//! until the sparse adapters have real resource hydration/registry seams.

#![cfg(all(feature = "vello-all", not(target_arch = "wasm32")))]

use netrender::vello_backends::{
    BackendAdmissionError, VelloBackend, scene_to_vello_cpu, scene_to_vello_hybrid,
    validate_scene_for_backend,
};
use netrender::{
    ColorLoad, NetrenderOptions, Scene, SceneClip, SceneFilter, SceneLayer, ScenePath, SceneShape,
    ScenePathStroke, SceneStrokeCap, SceneStrokeJoin, Transform, boot, create_netrender_instance,
};

const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

fn fixture_geometry_transform_clip() -> Scene {
    let mut scene = Scene::new(96, 96);
    let transform_id = scene.push_transform(Transform::translate_2d(16.0, 16.0));
    scene.push_rect_clipped(
        0.0,
        0.0,
        48.0,
        48.0,
        [1.0, 0.0, 0.0, 1.0],
        transform_id,
        [24.0, 24.0, 56.0, 56.0],
    );
    let shape_transform = scene.push_transform(Transform::translate_2d(64.0, 16.0));
    let mut path = ScenePath::new();
    path.move_to(0.0, 0.0)
        .line_to(24.0, 0.0)
        .line_to(24.0, 24.0)
        .line_to(0.0, 24.0)
        .close();
    scene.push_shape(SceneShape {
        path,
        fill_color: Some([0.0, 1.0, 0.0, 1.0]),
        stroke: Some(ScenePathStroke {
            color: [1.0, 1.0, 1.0, 1.0],
            width: 4.0,
            cap: SceneStrokeCap::Butt,
            join: SceneStrokeJoin::Miter,
            dash_pattern: Vec::new(),
            dash_offset: 0.0,
        }),
        transform_id: shape_transform,
        clip_rect: [60.0, 20.0, 92.0, 36.0],
        clip_corner_radii: [0.0; 4],
    });
    let stroke_transform = scene.push_transform(Transform::translate_2d(16.0, 64.0));
    scene.push_stroke_full(
        0.0,
        0.0,
        32.0,
        24.0,
        [0.0, 0.0, 1.0, 1.0],
        4.0,
        [0.0; 4],
        stroke_transform,
        [12.0, 60.0, 36.0, 92.0],
        [0.0; 4],
    );
    scene
}

fn fixture_gradients_nested_layers() -> Scene {
    let mut scene = Scene::new(96, 96);
    let mut outer = SceneLayer::alpha(0.8);
    outer.clip = SceneClip::Rect {
        rect: [4.0, 4.0, 92.0, 92.0],
        radii: [6.0; 4],
    };
    scene.push_layer(outer);
    let linear_transform = scene.push_transform(Transform::translate_2d(8.0, 0.0));
    scene.push_linear_gradient_full(
        0.0,
        8.0,
        80.0,
        32.0,
        [0.0, 0.0],
        [80.0, 0.0],
        [1.0, 0.0, 0.0, 1.0],
        [0.0, 0.0, 1.0, 1.0],
        linear_transform,
        [16.0, 8.0, 80.0, 32.0],
    );
    scene.push_radial_gradient(
        8.0,
        32.0,
        88.0,
        64.0,
        [48.0, 48.0],
        [24.0, 24.0],
        [0.0, 1.0, 0.0, 1.0],
        [0.0, 0.0, 1.0, 1.0],
    );
    scene.push_conic_gradient(
        8.0,
        64.0,
        88.0,
        88.0,
        [48.0, 76.0],
        0.0,
        [1.0, 0.0, 0.0, 1.0],
        [0.0, 0.0, 1.0, 1.0],
    );
    scene.push_layer_alpha(0.5);
    scene.push_rect(32.0, 12.0, 64.0, 28.0, [0.0, 1.0, 0.0, 1.0]);
    scene.pop_layer();
    scene.pop_layer();
    scene
}

fn target(device: &wgpu::Device, width: u32, height: u32) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("rg2a corpus target"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUsages::STORAGE_BINDING
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}

fn classic_pixels(renderer: &netrender::Renderer, scene: &Scene) -> Vec<u8> {
    let (texture, view) = target(
        &renderer.wgpu_device.core.device,
        scene.viewport_width,
        scene.viewport_height,
    );
    renderer.render_vello(scene, &view, ColorLoad::Clear(wgpu::Color::TRANSPARENT));
    renderer
        .wgpu_device
        .read_rgba8_texture(&texture, scene.viewport_width, scene.viewport_height)
}

fn hybrid_pixels(handles: &netrender_device::WgpuHandles, scene: &Scene) -> Vec<u8> {
    let width = scene.viewport_width;
    let height = scene.viewport_height;
    let sparse_scene = scene_to_vello_hybrid(scene).expect("Hybrid fixture admission");
    let sparse_width = u16::try_from(width).expect("admitted Hybrid fixture width fits u16");
    let sparse_height = u16::try_from(height).expect("admitted Hybrid fixture height fits u16");
    let render_size = vello_hybrid::RenderSize {
        width: sparse_width,
        height: sparse_height,
    };
    let (texture, view) = target(&handles.device, width, height);
    let (mut renderer, mut resources) = vello_hybrid::Renderer::new(
        &handles.device,
        &vello_hybrid::RenderTargetConfig {
            format: FORMAT,
            width: sparse_width,
            height: sparse_height,
        },
    );
    let depth = vello_hybrid::Renderer::create_depth_texture_view(&handles.device, &render_size);
    let mut encoder = handles
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("rg2a Hybrid corpus"),
        });
    renderer
        .render(
            &sparse_scene,
            &mut resources,
            &handles.device,
            &handles.queue,
            &mut encoder,
            &render_size,
            &view,
            Some(&depth),
            &vello_hybrid::TextureBindings::new(),
            vello_hybrid::TargetInit::Clear(vello_hybrid::ClearSettings::default()),
        )
        .expect("Hybrid fixture render");
    handles.queue.submit([encoder.finish()]);
    let bytes_per_row = width * 4;
    // Reuse Netrender's readback path through a temporary renderer would
    // change the ownership under test, so issue a small explicit map here.
    let padded = bytes_per_row.next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
    let buffer = handles.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("rg2a Hybrid readback"),
        size: padded as u64 * height as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut copy = handles
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("rg2a Hybrid readback copy"),
        });
    copy.copy_texture_to_buffer(
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
                bytes_per_row: Some(padded),
                rows_per_image: Some(height),
            },
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    let submission = handles.queue.submit([copy.finish()]);
    let slice = buffer.slice(..);
    let (sender, receiver) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = sender.send(result);
    });
    handles
        .device
        // This copy follows the producer render on the same queue. Waiting
        // for our copy submission covers both without an unbounded first wait.
        .poll(wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: Some(std::time::Duration::from_secs(5)),
        })
        .expect("Hybrid readback completion");
    receiver
        .try_recv()
        .expect("Hybrid readback sender")
        .expect("Hybrid readback map");
    let mapped = slice.get_mapped_range().expect("Hybrid mapped range");
    let mut out = Vec::with_capacity((width * height * 4) as usize);
    for row in 0..height as usize {
        let start = row * padded as usize;
        out.extend_from_slice(&mapped[start..start + bytes_per_row as usize]);
    }
    drop(mapped);
    buffer.unmap();
    out
}

fn pixel(bytes: &[u8], width: u32, x: u32, y: u32) -> [u8; 4] {
    let start = ((y * width + x) * 4) as usize;
    bytes[start..start + 4].try_into().expect("pixel")
}

fn assert_geometry_semantics(backend: VelloBackend, bytes: &[u8]) {
    let clipped_fill = pixel(bytes, 96, 32, 32);
    assert!(
        clipped_fill[0] > 180,
        "{backend:?} transformed clipped fill: {clipped_fill:?}"
    );
    assert_eq!(
        pixel(bytes, 96, 20, 32)[3],
        0,
        "device-space clip removes the transformed shape's left edge"
    );
    assert_eq!(
        pixel(bytes, 96, 60, 32)[3],
        0,
        "device-space clip removes the transformed shape's right edge"
    );
    let path_fill = pixel(bytes, 96, 76, 28);
    assert!(path_fill[1] > 180, "filled path anchor: {path_fill:?}");
    let path_stroke = pixel(bytes, 96, 65, 28);
    assert!(
        path_stroke[0] > 180 && path_stroke[1] > 180 && path_stroke[2] > 180,
        "stroked path anchor: {path_stroke:?}"
    );
    assert_eq!(
        pixel(bytes, 96, 76, 18)[3],
        0,
        "device-space clip removes the transformed path's top edge"
    );
    let rect_stroke = pixel(bytes, 96, 17, 76);
    assert!(
        rect_stroke[2] > 180,
        "transformed clipped stroke anchor: {rect_stroke:?}"
    );
    assert_eq!(
        pixel(bytes, 96, 47, 76)[3],
        0,
        "device-space clip removes the transformed stroke's right edge"
    );
    assert_eq!(
        pixel(bytes, 96, 10, 10)[3],
        0,
        "outside clip remains transparent"
    );
}

fn assert_gradient_layer_semantics(backend: VelloBackend, bytes: &[u8]) {
    let left = pixel(bytes, 96, 20, 16);
    let right = pixel(bytes, 96, 76, 16);
    let center = pixel(bytes, 96, 48, 20);
    assert!(
        left[0] > left[2] + 20,
        "{backend:?} gradient left anchor: {left:?}"
    );
    assert!(right[2] > right[0] + 20, "gradient right anchor: {right:?}");
    assert_eq!(
        pixel(bytes, 96, 12, 16)[3],
        0,
        "device-space clip removes the transformed gradient's left edge"
    );
    assert!(
        center[1] > center[0] && center[1] > center[2],
        "nested layer anchor: {center:?}"
    );
    let radial_center = pixel(bytes, 96, 48, 48);
    let radial_edge = pixel(bytes, 96, 72, 48);
    assert!(
        radial_center[1] > radial_center[2] + 20,
        "radial center anchor: {radial_center:?}"
    );
    assert!(
        radial_edge[2] > radial_edge[1] + 20,
        "radial edge anchor: {radial_edge:?}"
    );
    let conic_start = pixel(bytes, 96, 72, 76);
    assert!(
        conic_start[0] > conic_start[2] + 20,
        "conic start anchor: {conic_start:?}"
    );
    assert_eq!(
        pixel(bytes, 96, 0, 0)[3],
        0,
        "outside gradient remains transparent"
    );
}

#[test]
fn rg2a_direct_scene_corpus_is_semantically_consistent() {
    let handles = boot().expect("wgpu device");
    let adapter = handles.adapter.get_info();
    eprintln!(
        "rg2a adapter name={:?} backend={:?} driver={:?} driver_info={:?}; Classic=netrender-vello (resolved version in Cargo.lock); Hybrid/CPU=mark-ik/vello@ca3f40ea; wgpu=30",
        adapter.name, adapter.backend, adapter.driver, adapter.driver_info
    );
    let renderer = create_netrender_instance(
        handles.clone(),
        NetrenderOptions {
            tile_cache_size: Some(64),
            enable_vello: true,
            ..Default::default()
        },
    )
    .expect("Classic renderer");
    let fixtures = [
        ("geometry_transform_clip", fixture_geometry_transform_clip()),
        ("gradients_nested_layers", fixture_gradients_nested_layers()),
    ];
    for (name, scene) in fixtures {
        for backend in [
            VelloBackend::Classic,
            VelloBackend::Hybrid,
            VelloBackend::Cpu,
        ] {
            let capabilities = backend.capabilities();
            validate_scene_for_backend(backend, &scene)
                .expect("corpus fixture semantic validation");
            if name == "geometry_transform_clip" {
                assert!(
                    capabilities.solid_geometry && capabilities.transforms && capabilities.clips,
                    "{backend:?} lacks geometry fixture capability"
                );
            } else {
                assert!(
                    capabilities.gradients
                        && capabilities.layers
                        && capabilities.clips
                        && capabilities.nested_layers,
                    "{backend:?} lacks gradient/layer fixture capability"
                );
            }
        }
        let classic = classic_pixels(&renderer, &scene);
        let hybrid = hybrid_pixels(&handles, &scene);
        let cpu = scene_to_vello_cpu(&scene).expect("CPU fixture admission");
        let mut cpu_target =
            vello_cpu::Pixmap::new(scene.viewport_width as u16, scene.viewport_height as u16);
        let mut resources = vello_cpu::Resources::new();
        cpu.render(&mut cpu_target, &mut resources);
        let cpu_bytes = cpu_target
            .data()
            .iter()
            .flat_map(|pixel| [pixel.r, pixel.g, pixel.b, pixel.a])
            .collect::<Vec<_>>();
        match name {
            "geometry_transform_clip" => {
                assert_geometry_semantics(VelloBackend::Classic, &classic);
                assert_geometry_semantics(VelloBackend::Hybrid, &hybrid);
                assert_geometry_semantics(VelloBackend::Cpu, &cpu_bytes);
            }
            "gradients_nested_layers" => {
                assert_gradient_layer_semantics(VelloBackend::Classic, &classic);
                assert_gradient_layer_semantics(VelloBackend::Hybrid, &hybrid);
                assert_gradient_layer_semantics(VelloBackend::Cpu, &cpu_bytes);
            }
            _ => unreachable!(),
        }
    }
}

#[path = "rg2a_scene_corpus/refusals.rs"]
mod refusals;
