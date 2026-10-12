// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Pixel readback for tests.

use crate::{core, frame};
use std::time::{Duration, Instant};

/// Read an RGBA8 texture back to CPU memory as tightly-packed rows.
/// wgpu requires COPY_BYTES_PER_ROW_ALIGNMENT padding on the GPU copy;
/// this helper hides that staging detail and returns width * height * 4
/// bytes to callers. Completion is bounded to this copy submission with one
/// five-second deadline shared by device polling and callback receipt.
pub(crate) fn read_rgba8_texture(
    dev: &core::WgpuHandles,
    target: &wgpu::Texture,
    width: u32,
    height: u32,
) -> Vec<u8> {
    let row_bytes = width * 4;
    let padded = row_bytes.next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
    let buffer = dev.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("wgpu readback"),
        size: padded as u64 * height as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let mut encoder = frame::create_encoder(&dev.device, "wgpu readback encoder");
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: target,
            mip_level: 0,
            origin: wgpu::Origin3d { x: 0, y: 0, z: 0 },
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
    let submission = frame::submit_index(&dev.queue, encoder);
    let deadline = Instant::now() + Duration::from_secs(5);

    let slice = buffer.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = tx.send(result);
    });
    // Cancel mapping and unmap on every result path. A timed-out late callback
    // safely sees a dropped receiver; no sender panic can outlive the test.
    let result = (|| -> Result<Vec<u8>, String> {
        dev.device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(submission),
                timeout: Some(deadline.saturating_duration_since(Instant::now())),
            })
            .map_err(|error| format!("bounded readback poll failed: {error}"))?;
        rx.recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .map_err(|error| format!("bounded readback callback failed: {error}"))?
            .map_err(|error| format!("readback mapping failed: {error}"))?;
        let mapped = slice
            .get_mapped_range()
            .map_err(|error| format!("readback map range failed: {error}"))?;

        let mut out = Vec::with_capacity((row_bytes * height) as usize);
        for row in 0..height as usize {
            let src = row * padded as usize;
            let bytes = mapped
                .get(src..src + row_bytes as usize)
                .ok_or_else(|| "readback mapped range is shorter than its row layout".to_owned())?;
            out.extend_from_slice(bytes);
        }
        drop(mapped);
        Ok(out)
    })();
    buffer.unmap();
    result.expect("bounded RGBA8 readback failed")
}
