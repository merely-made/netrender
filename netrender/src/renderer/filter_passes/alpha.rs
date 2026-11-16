// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

use super::Renderer;

impl Renderer {
    pub(super) fn premultiply_filter_input(&self, texture: &wgpu::Texture) -> wgpu::Texture {
        let device = &self.wgpu_device.core.device;
        let view = texture.create_view(&Default::default());
        // Vello's fine raster writes straight RGB. Filter shaders operate on
        // premultiplied pixels; blend onto an explicitly cleared transparent
        // target to establish that convention before the first filter pass.
        let premultiplied = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("netrender premultiplied filter input"),
            size: texture.size(),
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let output = premultiplied.create_view(&Default::default());
        let pipe = self.external_texture_pipeline(wgpu::TextureFormat::Rgba8Unorm);
        let placement = crate::ExternalTexturePlacement::new([
            0.0,
            0.0,
            texture.width() as f32,
            texture.height() as f32,
        ]);
        let commands = crate::external_texture::external_texture_commands(
            device,
            &pipe,
            &view,
            texture.width(),
            texture.height(),
            placement,
        )
        .expect("nonempty filter viewport");
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("netrender filter alpha conversion"),
        });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("netrender filter alpha conversion"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &output,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            commands.encode(&mut pass);
        }
        self.wgpu_device.core.queue.submit([encoder.finish()]);
        premultiplied
    }
}
