// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

use super::*;
use netrender::vello_backends::HybridSession;
struct Gpu {
    session: HybridSession,
    handles: netrender::WgpuHandles,
    readback: netrender::WgpuDevice,
    texture: wgpu::Texture,
}
impl Gpu {
    fn new(limits: SparseTextLimits) -> Self {
        let handles = netrender::boot().expect("VB3 existing device");
        let info = handles.adapter.get_info();
        eprintln!(
            "VB3 text adapter={:?} backend={:?} driver={:?}",
            info.name, info.backend, info.driver
        );
        let session = HybridSession::new_with_text_limits(
            &handles.device,
            &handles.queue,
            SparseResourceLimits::default(),
            limits,
        )
        .unwrap();
        let readback = netrender::WgpuDevice::with_external(handles.clone()).unwrap();
        let texture = handles.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("VB3 text acceptance target"),
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
    fn render(&mut self, scene: &Scene) -> Result<Vec<u8>, SparseSessionError> {
        let before = self.readback.read_rgba8_texture(&self.texture, DIM, DIM);
        let mut encoder =
            self.handles
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("VB3 text frame"),
                });
        let result = self.session.render(
            scene,
            &self.handles.device,
            &self.handles.queue,
            &mut encoder,
            &self.texture,
        );
        self.handles.queue.submit([encoder.finish()]);
        let after = self.readback.read_rgba8_texture(&self.texture, DIM, DIM);
        if result.is_err() {
            assert_eq!(after, before, "text refusal must preserve visible target");
        }
        result.map(|()| after)
    }
    fn text_stats(&self) -> SparseTextStats {
        self.session.text_stats()
    }
    fn resource_stats(&self) -> SparseResourceStats {
        self.session.stats()
    }
    fn clear_text_cache(&mut self) {
        self.session.clear_text_cache();
    }
}
#[test]
fn hybrid_positions_composition_on_existing_device() {
    positioned_and_composed(&mut Gpu::new(SparseTextLimits::default()));
}
#[test]
fn hybrid_variations_fallback_on_existing_device() {
    variations_and_caller_fallback(&mut Gpu::new(SparseTextLimits::default()));
}
#[test]
fn hybrid_invalid_text_on_existing_device() {
    invalid_text(&mut Gpu::new(SparseTextLimits::default()));
}
#[test]
fn hybrid_bounded_text_lifecycle_on_existing_device() {
    text_lifecycle(&mut Gpu::new(bounded_text_limits()));
}
#[test]
fn hybrid_repeated_outline_budget_on_existing_device() {
    repeated_outline_budget(&mut Gpu::new(repeated_outline_limits()));
}
