// Copyright 2026 Mark Alan Boykin
// SPDX-License-Identifier: MPL-2.0

use super::Renderer;

impl Renderer {
    pub(super) fn crop_backdrop(
        &self,
        prefix_tex: &wgpu::Texture,
        bounds: [f32; 4],
    ) -> wgpu::Texture {
        let width = (bounds[2] - bounds[0]) as u32;
        let height = (bounds[3] - bounds[1]) as u32;
        let device = &self.wgpu_device.core.device;
        let cropped = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("mirrored backdrop pane"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("crop backdrop pane"),
        });
        encoder.copy_texture_to_texture(
            wgpu::TexelCopyTextureInfo {
                texture: prefix_tex,
                mip_level: 0,
                origin: wgpu::Origin3d {
                    x: bounds[0] as u32,
                    y: bounds[1] as u32,
                    z: 0,
                },
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyTextureInfo {
                texture: &cropped,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            cropped.size(),
        );
        self.wgpu_device.core.queue.submit([encoder.finish()]);
        cropped
    }
}
