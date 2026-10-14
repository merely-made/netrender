// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Replay an intact retained scene and its captured native producer image.
//! Usage: retained_scene_replay SCENE_JSON SOURCE_PNG IMAGE_KEY GENERATION
//! straight|premultiplied OUTDIR. The image is uploaded, staged and sampled
//! through the same device as Classic rendering; unresolved resources refuse.
use std::{
    error::Error,
    fs,
    path::Path,
    time::{Duration, Instant},
};
use netrender::{ColorLoad, NetrenderOptions, Scene, SceneOp, SourceAlpha};
type Result<T> = std::result::Result<T, Box<dyn Error>>;

struct ScopeLogger;
impl log::Log for ScopeLogger {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        metadata.target() == "netrender::tile_scopes"
    }
    fn log(&self, record: &log::Record<'_>) {
        if self.enabled(record.metadata()) {
            println!("{}", record.args());
        }
    }
    fn flush(&self) {}
}
static SCOPE_LOGGER: ScopeLogger = ScopeLogger;

fn read_png(path: &str) -> Result<(u32, u32, Vec<u8>)> {
    let decoder = png::Decoder::new(fs::File::open(path)?);
    let mut reader = decoder.read_info()?;
    let mut pixels = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut pixels)?;
    if info.color_type != png::ColorType::Rgba || info.bit_depth != png::BitDepth::Eight {
        return Err("producer resource must be captured RGBA8".into());
    }
    pixels.truncate(info.buffer_size());
    Ok((info.width, info.height, pixels))
}

fn write_png(path: &Path, width: u32, height: u32, pixels: &[u8]) -> Result<()> {
    let mut encoder = png::Encoder::new(fs::File::create(path)?, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(pixels)?;
    Ok(())
}

fn texture(device: &wgpu::Device, width: u32, height: u32) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("retained replay same-device texture"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::STORAGE_BINDING
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC
            | wgpu::TextureUsages::COPY_DST
            | wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    })
}

fn readback(
    handles: &netrender_device::WgpuHandles,
    texture: &wgpu::Texture,
    width: u32,
    height: u32,
) -> Result<Vec<u8>> {
    let row = width * 4;
    let padded = row.next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
    let buffer = handles.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("bounded retained scene readback"),
        size: u64::from(padded) * u64::from(height),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = handles.device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture,
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
    let submission = handles.queue.submit([encoder.finish()]);
    let slice = buffer.slice(..);
    let (sender, receiver) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = sender.send(result);
    });
    let deadline = Instant::now() + Duration::from_secs(10);
    handles.device.poll(wgpu::PollType::Wait {
        submission_index: Some(submission),
        timeout: Some(deadline.saturating_duration_since(Instant::now())),
    })?;
    receiver.recv_timeout(deadline.saturating_duration_since(Instant::now()))??;
    let mapped = slice.get_mapped_range()?;
    let pixels = mapped
        .chunks_exact(padded as usize)
        .flat_map(|row_pixels| row_pixels[..row as usize].iter().copied())
        .collect();
    drop(mapped);
    buffer.unmap();
    Ok(pixels)
}

fn main() -> Result<()> {
    log::set_logger(&SCOPE_LOGGER).map_err(|error| error.to_string())?;
    log::set_max_level(log::LevelFilter::Debug);
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 6 {
        return Err(
            "expected SCENE_JSON SOURCE_PNG IMAGE_KEY GENERATION straight|premultiplied OUTDIR"
                .into(),
        );
    }
    let scene = Scene::replay_json(&fs::read_to_string(&args[0])?)?;
    let key: u64 = args[2].parse()?;
    let generation: u64 = args[3].parse()?;
    let alpha = match args[4].as_str() {
        "straight" => SourceAlpha::Straight,
        "premultiplied" => SourceAlpha::Premultiplied,
        _ => return Err("resource alpha contract is required".into()),
    };
    if !scene.compositor_surfaces.is_empty()
        || scene.ops.iter().any(|op| matches!(op, SceneOp::GlyphRun(run) if run.font_id == 0 || run.font_id as usize >= scene.fonts.len()))
        || scene
        .ops
        .iter()
        .filter(|op| matches!(op, SceneOp::Image(_)))
        .count()
        != 1
        || !scene
            .ops
            .iter()
            .any(|op| matches!(op, SceneOp::Image(image) if image.key == key))
        || scene
            .ops
            .iter()
            .any(|op| matches!(op, SceneOp::Pattern(_) | SceneOp::Fragment(_)))
    {
        return Err("fixture must resolve exactly one actual image and no uncaptured pattern/fragment resources".into());
    }
    let (width, height, source_pixels) = read_png(&args[1])?;
    let handles = netrender::boot()?;
    let adapter = handles.adapter.get_info();
    let renderer = netrender::create_netrender_instance(
        handles.clone(),
        NetrenderOptions {
            enable_vello: true,
            tile_cache_size: Some(64),
            ..Default::default()
        },
    )
    .map_err(|error| format!("renderer initialization: {error:?}"))?;
    let source = texture(&handles.device, width, height);
    handles.queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &source,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &source_pixels,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(width * 4),
            rows_per_image: Some(height),
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    renderer.stage_external_image(
        key,
        &source.create_view(&Default::default()),
        [width, height],
        alpha,
        generation,
    );
    let target = texture(&handles.device, scene.viewport_width, scene.viewport_height);
    fs::create_dir_all(&args[5])?;
    let out = Path::new(&args[5]);
    for frame in ["cold", "warm"] {
        renderer.render_vello_checked(
            &scene,
            &target.create_view(&Default::default()),
            ColorLoad::Clear(wgpu::Color::TRANSPARENT),
        )?;
        let pixels = readback(
            &handles,
            &target,
            scene.viewport_width,
            scene.viewport_height,
        )?;
        if renderer.poll_vello_curve_quality()? != Some((0, 0)) {
            return Err("unobserved quality counter failure".into());
        }
        write_png(
            &out.join(format!("{frame}.png")),
            scene.viewport_width,
            scene.viewport_height,
            &pixels,
        )?;
    }
    fs::write(
        out.join("replay.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "scene": args[0], "resource": args[1], "image_key": key, "generation": generation,
            "source_alpha": args[4], "source_size": [width, height], "viewport": [scene.viewport_width, scene.viewport_height],
            "scene_ops": scene.ops.len(), "tile_size": 64, "same_device_stage_and_render": true,
        "adapter": { "name": adapter.name, "backend": format!("{:?}", adapter.backend), "vendor": adapter.vendor, "device": adapter.device, "device_type": format!("{:?}", adapter.device_type), "driver": adapter.driver, "driver_info": adapter.driver_info },
            "readback_timeout_seconds": 10
        }))?,
    )?;
    println!(
        "rendered intact scene: {} ops, {}x{}, same-device resource {}x{}",
        scene.ops.len(),
        scene.viewport_width,
        scene.viewport_height,
        width,
        height
    );
    Ok(())
}
