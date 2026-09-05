// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! RG3a receipts for the public opaque-tenant frame envelope.

use crate::{
    Compositor, ExternalTextureComposite, ExternalTexturePlacement, NetrenderOptions,
    OpaqueTenantInput, OpaqueTenantMetadata, Scene, boot, create_netrender_instance,
};

const MASTER_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const TENANT_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
const WIDTH: u32 = 16;
const HEIGHT: u32 = 16;

struct CaptureCompositor {
    master: Option<wgpu::Texture>,
}

impl Compositor for CaptureCompositor {
    fn declare_surface(&mut self, _key: crate::SurfaceKey, _world_bounds: [f32; 4]) {}

    fn destroy_surface(&mut self, _key: crate::SurfaceKey) {}

    fn present_frame(&mut self, frame: netrender_device::PresentedFrame<'_>) {
        self.master = Some(frame.master.clone());
    }
}

fn upload_tenant(device: &wgpu::Device, queue: &wgpu::Queue) -> wgpu::Texture {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("rg3a tenant texture"),
        size: wgpu::Extent3d {
            width: WIDTH,
            height: HEIGHT,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: TENANT_FORMAT,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let bytes: Vec<u8> = (0..WIDTH * HEIGHT)
        .flat_map(|_| [0u8, 128, 255, 180])
        .collect();
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &bytes,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(WIDTH * 4),
            rows_per_image: Some(HEIGHT),
        },
        wgpu::Extent3d {
            width: WIDTH,
            height: HEIGHT,
            depth_or_array_layers: 1,
        },
    );
    texture
}

fn scene() -> Scene {
    let mut scene = Scene::new(WIDTH, HEIGHT);
    scene.push_rect(0.0, 0.0, WIDTH as f32, HEIGHT as f32, [1.0, 0.0, 0.0, 1.0]);
    scene
}

fn options() -> NetrenderOptions {
    NetrenderOptions {
        enable_vello: true,
        tile_cache_size: Some(64),
        ..Default::default()
    }
}

#[test]
fn rg3a_receipt_has_stable_opaque_boundary_fields() {
    let _gpu_guard = super::gpu_test_guard();
    let renderer = create_netrender_instance(boot().expect("wgpu boot"), options())
        .expect("create netrender instance");
    let device = renderer.wgpu_device.core.device.clone();
    let queue = renderer.wgpu_device.core.queue.clone();
    let tenant = upload_tenant(&device, &queue);
    let metadata = OpaqueTenantMetadata::new(
        "room-alpha",
        "paredros-opaque-submit",
        2,
        0,
        ExternalTexturePlacement::new([0.0, 0.0, WIDTH as f32, HEIGHT as f32]),
    );
    let input = OpaqueTenantInput::new(&tenant, metadata);
    let mut compositor = CaptureCompositor { master: None };
    let receipt = renderer.render_with_opaque_tenant(
        &scene(),
        MASTER_FORMAT,
        &mut compositor,
        vello::peniko::Color::BLACK,
        &input,
    );

    assert_eq!(receipt.tenant_name, "room-alpha");
    assert_eq!(receipt.producer_path, "paredros-opaque-submit");
    assert_eq!(receipt.fallback_count, 2);
    assert_eq!(receipt.scene_op_boundary, 0);
    assert_eq!(receipt.caller_reported_physical_submission_count, None);
    assert_eq!(receipt.logical_opaque_producer_boundaries, 1);
    assert_eq!(receipt.graph_encoder_batches, 1);
    assert_eq!(receipt.graph_submission_boundaries, 1);
    assert!(receipt
        .logical_plan_dump
        .contains("tenant_name=\"room-alpha\""));
    assert!(receipt
        .logical_plan_dump
        .contains("caller_reported_physical_submission_count=None"));
    assert!(receipt
        .logical_plan_dump
        .contains("graph_submission_boundaries: 1 (submit)"));
    assert!(receipt
        .logical_plan_dump
        .contains("rasterizer=Classic execution_boundary=opaque_submission"));
    assert!(compositor.master.is_some());
}

#[test]
fn rg3a_boundary_zero_matches_legacy_final_master() {
    let _gpu_guard = super::gpu_test_guard();
    let handles = boot().expect("wgpu boot");
    let candidate_renderer = create_netrender_instance(handles.clone(), options())
        .expect("create candidate netrender instance");
    let legacy_renderer =
        create_netrender_instance(handles, options()).expect("create legacy netrender instance");
    let candidate_device = candidate_renderer.wgpu_device.core.device.clone();
    let candidate_queue = candidate_renderer.wgpu_device.core.queue.clone();
    let legacy_device = legacy_renderer.wgpu_device.core.device.clone();
    let legacy_queue = legacy_renderer.wgpu_device.core.queue.clone();
    let candidate_tenant = upload_tenant(&candidate_device, &candidate_queue);
    let legacy_tenant = upload_tenant(&legacy_device, &legacy_queue);
    let legacy_tenant_view = legacy_tenant.create_view(&wgpu::TextureViewDescriptor::default());
    let placement = ExternalTexturePlacement::new([0.0, 0.0, WIDTH as f32, HEIGHT as f32]);
    let input = OpaqueTenantInput::new(
        &candidate_tenant,
        OpaqueTenantMetadata::new("room-alpha", "paredros-opaque-submit", 0, 0, placement)
            .with_reported_physical_submission_count(1),
    );

    let mut candidate_compositor = CaptureCompositor { master: None };
    candidate_renderer.render_with_opaque_tenant(
        &scene(),
        MASTER_FORMAT,
        &mut candidate_compositor,
        vello::peniko::Color::BLACK,
        &input,
    );
    let candidate_master = candidate_compositor
        .master
        .take()
        .expect("candidate master");
    let candidate =
        candidate_renderer
            .wgpu_device
            .read_rgba8_texture(&candidate_master, WIDTH, HEIGHT);

    let mut legacy_compositor = CaptureCompositor { master: None };
    legacy_renderer.render_with_compositor_and_external_textures(
        &scene(),
        MASTER_FORMAT,
        &mut legacy_compositor,
        vello::peniko::Color::BLACK,
        &[ExternalTextureComposite::new(&legacy_tenant_view, placement).with_scene_op_boundary(0)],
    );
    let legacy_master = legacy_compositor.master.take().expect("legacy master");
    let legacy = legacy_renderer
        .wgpu_device
        .read_rgba8_texture(&legacy_master, WIDTH, HEIGHT);

    assert_eq!(candidate, legacy);
}
