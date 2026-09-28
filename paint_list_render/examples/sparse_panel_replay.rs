// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Intact, caller-shaped PaintEnvelope replay. No layout, shaping, or font lookup.
//! Usage: sparse_panel_replay PACKET REFERENCE.png OUTDIR cpu|hybrid|classic [--scale N]
//! Enable replay-cpu / replay-hybrid for those modes. CPU never boots a GPU.
//! This is a single cold replay receipt, not an interactive performance benchmark.

use netrender::{Scene, SceneOp, Transform, NO_CLIP};
use paint_list_api::{PaintCmd, PaintEnvelope};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use skrifa::raw::TableProvider;
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    error::Error,
    fs::{self, File},
    io::BufReader,
    path::{Path, PathBuf},
    time::Instant,
};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn command_name(command: &PaintCmd) -> &'static str {
    match command {
        PaintCmd::PushClip(_) => "PushClip",
        PaintCmd::PopClip => "PopClip",
        PaintCmd::PushTransform(_) => "PushTransform",
        PaintCmd::PopTransform => "PopTransform",
        PaintCmd::PushLayer(_) => "PushLayer",
        PaintCmd::PopLayer => "PopLayer",
        PaintCmd::DrawRect(_) => "DrawRect",
        PaintCmd::DrawStroke(_) => "DrawStroke",
        PaintCmd::DrawLine(_) => "DrawLine",
        PaintCmd::DrawPath(_) => "DrawPath",
        PaintCmd::DrawBorder(_) => "DrawBorder",
        PaintCmd::DrawLinearGradient(_) => "DrawLinearGradient",
        PaintCmd::DrawRadialGradient(_) => "DrawRadialGradient",
        PaintCmd::DrawConicGradient(_) => "DrawConicGradient",
        PaintCmd::DrawText(_) => "DrawText",
        PaintCmd::DrawImage(_) => "DrawImage",
        PaintCmd::DrawRepeatingImage(_) => "DrawRepeatingImage",
        PaintCmd::DrawExternalTexture(_) => "DrawExternalTexture",
        PaintCmd::DrawShadow(_) => "DrawShadow",
        PaintCmd::PushShadow(_) => "PushShadow",
        PaintCmd::PopAllShadows => "PopAllShadows",
        PaintCmd::HitTest(_) => "HitTest",
        PaintCmd::PlaceRetainedFragment(_) => "PlaceRetainedFragment",
    }
}

fn op_name(op: &SceneOp) -> &'static str {
    match op {
        SceneOp::Rect(_) => "Rect",
        SceneOp::Stroke(_) => "Stroke",
        SceneOp::Gradient(_) => "Gradient",
        SceneOp::Image(_) => "Image",
        SceneOp::Pattern(_) => "Pattern",
        SceneOp::Shape(_) => "Shape",
        SceneOp::GlyphRun(_) => "GlyphRun",
        SceneOp::PushLayer(_) => "PushLayer",
        SceneOp::PopLayer => "PopLayer",
        SceneOp::Fragment(_) => "Fragment",
    }
}

/// Validate before translation, whose missing-resource path otherwise skips draws.
fn validate_packet(envelope: &PaintEnvelope) -> Result<()> {
    if envelope.viewport.width <= 0 || envelope.viewport.height <= 0 {
        return Err("packet has empty viewport".into());
    }
    let mut fonts = HashMap::new();
    for font in &envelope.fonts {
        let face = skrifa::FontRef::from_index(&font.data, font.index)
            .map_err(|e| format!("font {:?} face {}: {e:?}", font.key, font.index))?;
        let glyph_count = face.maxp()?.num_glyphs();
        if fonts.insert(font.key, glyph_count).is_some() {
            return Err(format!("duplicate font resource {:?}", font.key).into());
        }
    }
    let mut images = HashSet::new();
    for image in &envelope.images {
        let bytes = (image.width as usize)
            .checked_mul(image.height as usize)
            .and_then(|n| n.checked_mul(4));
        if image.width == 0 || image.height == 0 || bytes != Some(image.data.len()) {
            return Err(format!("invalid RGBA image resource {:?}", image.key).into());
        }
        if !images.insert(image.key) {
            return Err(format!("duplicate image resource {:?}", image.key).into());
        }
    }
    let mut transforms = 0usize;
    let mut layers = Vec::new();
    for (index, command) in envelope.commands.iter().enumerate() {
        let fail = |reason: &str| -> Box<dyn Error> {
            format!(
                "packet command {index} ({}): {reason}",
                command_name(command)
            )
            .into()
        };
        match command {
            PaintCmd::PushTransform(_) => transforms += 1,
            PaintCmd::PopTransform => {
                transforms = transforms
                    .checked_sub(1)
                    .ok_or_else(|| fail("unbalanced transform"))?;
            }
            PaintCmd::PushClip(_) => layers.push("clip"),
            PaintCmd::PushLayer(_) => layers.push("layer"),
            PaintCmd::PopClip | PaintCmd::PopLayer => {
                let expected = if matches!(command, PaintCmd::PopClip) {
                    "clip"
                } else {
                    "layer"
                };
                if layers.pop() != Some(expected) {
                    return Err(fail("unbalanced clip/layer nesting"));
                }
            }
            PaintCmd::DrawText(run) => {
                let count = fonts
                    .get(&run.font_instance)
                    .ok_or_else(|| fail("missing font resource"))?;
                if !run.font_size.is_finite() || run.font_size <= 0.0 {
                    return Err(fail("invalid font size"));
                }
                for glyph in &run.glyphs {
                    if glyph.index >= u32::from(*count)
                        || !glyph.point.x.is_finite()
                        || !glyph.point.y.is_finite()
                    {
                        return Err(fail("invalid positioned glyph"));
                    }
                }
            }
            PaintCmd::DrawImage(image) if !images.contains(&image.image_key) => {
                return Err(fail("missing image resource"))
            }
            PaintCmd::DrawRepeatingImage(image) if !images.contains(&image.image_key) => {
                return Err(fail("missing tile resource"))
            }
            PaintCmd::DrawBorder(border) => {
                if let paint_list_api::BorderDetails::NinePatch(patch) = &border.details {
                    if let paint_list_api::NinePatchSource::Image(key, _) = &patch.source {
                        if !images.contains(key) {
                            return Err(fail("missing nine-patch image"));
                        }
                    }
                }
            }
            PaintCmd::DrawExternalTexture(_) => {
                return Err(fail(
                    "external texture producer is absent from serialized capture",
                ))
            }
            PaintCmd::PlaceRetainedFragment(_) => {
                return Err(fail(
                    "retained fragment registry is absent from serialized capture",
                ))
            }
            PaintCmd::PushShadow(_) => {
                return Err(fail("translator does not implement text-shadow stack"))
            }
            _ => {}
        }
    }
    if transforms != 0 || !layers.is_empty() {
        return Err("packet has unterminated transform/clip/layer scopes".into());
    }
    Ok(())
}

fn verify_identity(envelope: &PaintEnvelope, scene: &Scene) -> Result<()> {
    // Empty shaped runs are explicit no-ops in the shared translator.
    let original: Vec<_> = envelope
        .commands
        .iter()
        .filter_map(|command| match command {
            PaintCmd::DrawText(run) if !run.glyphs.is_empty() => Some(run),
            _ => None,
        })
        .collect();
    let replay: Vec<_> = scene.iter_glyph_runs().collect();
    if original.len() != replay.len() {
        return Err("translation changed nonempty text run count".into());
    }
    for (original, replay) in original.into_iter().zip(replay) {
        let font = envelope
            .fonts
            .iter()
            .find(|font| font.key == original.font_instance)
            .ok_or("missing original font")?;
        let translated = scene
            .fonts
            .get(replay.font_id as usize)
            .ok_or("missing translated font")?;
        if font.index != translated.index
            || font.data.as_slice() != translated.data.as_ref()
            || original.font_size != replay.font_size
            || original.glyphs.len() != replay.glyphs.len()
        {
            return Err("font bytes/index or shaped run changed in translation".into());
        }
        for (a, b) in original.glyphs.iter().zip(&replay.glyphs) {
            if a.index != b.id || a.point.x != b.x || a.point.y != b.y {
                return Err("positioned glyph changed in translation".into());
            }
        }
    }
    if scene.fonts.len() != envelope.fonts.len() + 1
        || scene.image_sources.len() != envelope.images.len()
    {
        return Err("translation changed resource palette counts".into());
    }
    Ok(())
}

/// Root presentation affine, equivalent to Classic's scaled append. Geometry,
/// glyph positions, font sizes and local layer clips stay unchanged. Primitive
/// device clips scale separately. Keep reserved transform 0 equal to identity.
fn physical_scene(logical: &Scene, width: u32, height: u32, scale: f32) -> Result<Scene> {
    if !scale.is_finite() || scale <= 0.0 {
        return Err("invalid presentation scale".into());
    }
    let mut scene = logical.clone();
    let offset = u32::try_from(scene.transforms.len())?;
    let root = Transform::scale_2d(scale, scale);
    scene
        .transforms
        .extend(logical.transforms.iter().map(|t| t.then(&root)));
    for (index, op) in scene.ops.iter_mut().enumerate() {
        let (id, clip) = match op {
            SceneOp::Rect(v) => (
                &mut v.transform_id,
                Some((&mut v.clip_rect, &mut v.clip_corner_radii)),
            ),
            SceneOp::Stroke(v) => (
                &mut v.transform_id,
                Some((&mut v.clip_rect, &mut v.clip_corner_radii)),
            ),
            SceneOp::Gradient(v) => (
                &mut v.transform_id,
                Some((&mut v.clip_rect, &mut v.clip_corner_radii)),
            ),
            SceneOp::Image(v) => (
                &mut v.transform_id,
                Some((&mut v.clip_rect, &mut v.clip_corner_radii)),
            ),
            SceneOp::Pattern(v) => (
                &mut v.transform_id,
                Some((&mut v.clip_rect, &mut v.clip_corner_radii)),
            ),
            SceneOp::Shape(v) => (
                &mut v.transform_id,
                Some((&mut v.clip_rect, &mut v.clip_corner_radii)),
            ),
            SceneOp::GlyphRun(v) => (
                &mut v.transform_id,
                Some((&mut v.clip_rect, &mut v.clip_corner_radii)),
            ),
            SceneOp::PushLayer(v) => (&mut v.transform_id, None),
            SceneOp::Fragment(v) => (&mut v.transform_id, None),
            SceneOp::PopLayer => continue,
        };
        if *id >= offset {
            return Err(
                format!("scene op {index}: invalid transform before presentation scaling").into(),
            );
        }
        *id = id
            .checked_add(offset)
            .ok_or("presentation transform index overflow")?;
        if let Some((rect, radii)) = clip {
            if *rect != NO_CLIP {
                for v in rect {
                    *v *= scale;
                }
            }
            for v in radii {
                *v *= scale;
            }
        }
    }
    scene.viewport_width = width;
    scene.viewport_height = height;
    if !scene.compositor_surfaces.is_empty() {
        return Err("native compositor surfaces need captured producer pixels".into());
    }
    Ok(scene)
}

fn read_png(path: &Path) -> Result<(u32, u32, Vec<u8>)> {
    let mut reader = png::Decoder::new(BufReader::new(File::open(path)?)).read_info()?;
    let mut data = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut data)?;
    if info.color_type != png::ColorType::Rgba || info.bit_depth != png::BitDepth::Eight {
        return Err("reference must be RGBA8".into());
    }
    data.truncate(info.buffer_size());
    Ok((info.width, info.height, data))
}

fn write_png(path: &Path, width: u32, height: u32, data: &[u8]) -> Result<()> {
    let output = File::options().write(true).create_new(true).open(path)?;
    let mut encoder = png::Encoder::new(output, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(data)?;
    Ok(())
}

/// Classic stores straight alpha; sparse CPU/Hybrid store premultiplied alpha.
/// Convert both to an opaque-black presentation without changing Scene ops.
fn present_over_black(pixels: &mut [u8], backend: &str) {
    for pixel in pixels.chunks_exact_mut(4) {
        if backend == "classic" {
            let alpha = u16::from(pixel[3]);
            for channel in &mut pixel[..3] {
                *channel = ((u16::from(*channel) * alpha + 127) / 255) as u8;
            }
        }
        pixel[3] = 255;
    }
}

#[cfg(any(feature = "replay-cpu", feature = "replay-hybrid"))]
fn stats(
    images: netrender::vello_backends::SparseResourceStats,
    text: netrender::vello_backends::SparseTextStats,
) -> Value {
    json!({"images": {
        "source_count": images.source_count, "source_bytes": images.source_bytes,
        "cached_images": images.cached_images, "cached_bytes": images.cached_bytes,
        "gpu_texture_bytes": images.gpu_texture_bytes, "hydrated_images": images.hydrated_images,
        "uploaded_images": images.uploaded_images
    }, "text": {
        "font_count": text.font_count, "font_bytes": text.font_bytes,
        "cached_glyphs": text.cached_glyphs, "cached_outline_segments": text.cached_outline_segments,
        "prepared_glyphs": text.prepared_glyphs, "cache_resets": text.cache_resets,
        "cache_frames": text.cache_frames
    }})
}

#[cfg(any(feature = "replay-cpu", feature = "replay-hybrid"))]
fn refusal(
    error: &netrender::vello_backends::SparseSessionError,
    scene: &Scene,
    receipt: &mut Value,
) {
    use netrender::vello_backends::{BackendAdmissionError as A, SparseSessionError as E};
    let index = match error {
        E::InvalidFont { op_index, .. } | E::InvalidGlyph { op_index, .. } => Some(*op_index),
        E::InvalidScene { op_index, .. } => *op_index,
        E::Admission(
            A::UnsupportedOperation { op_index, .. } | A::InvalidTransform { op_index, .. },
        ) => Some(*op_index),
        E::Admission(A::UnbalancedLayers { op_index, .. }) => *op_index,
        E::MissingImage { key } | E::InvalidImage { key, .. } => {
            scene.ops.iter().position(|op| match op {
                SceneOp::Image(image) => image.key == *key,
                SceneOp::Pattern(pattern) => pattern.tile == *key,
                _ => false,
            })
        }
        _ => None,
    };
    receipt["refusal"] = json!({"typed_error": format!("{error:?}"), "scene_op_index": index,
        "scene_operation": index.and_then(|i| scene.ops.get(i)).map(op_name)});
}

fn render(scene: &Scene, backend: &str, receipt: &mut Value) -> Result<Vec<u8>> {
    if backend == "cpu" {
        #[cfg(feature = "replay-cpu")]
        {
            use netrender::vello_backends::{CpuSession, SparseResourceLimits};
            let start = Instant::now();
            let mut session = CpuSession::new(SparseResourceLimits::default());
            receipt["initialize_ms"] = json!(start.elapsed().as_secs_f64() * 1000.0);
            let start = Instant::now();
            let pixels = session.render(scene);
            receipt["render_encode_submit_ms"] = json!(start.elapsed().as_secs_f64() * 1000.0);
            receipt["session_stats"] = stats(session.stats(), session.text_stats());
            receipt["gpu_initialized"] = json!(false);
            receipt["gpu_completion_wait_ms"] = Value::Null;
            receipt["readback_ms"] = Value::Null;
            return match pixels {
                Ok(pixels) => Ok(pixels.data_as_u8_slice().to_vec()),
                Err(error) => {
                    refusal(&error, scene, receipt);
                    Err(error.into())
                }
            };
        }
        #[cfg(not(feature = "replay-cpu"))]
        return Err("cpu mode requires feature replay-cpu".into());
    }
    if backend != "classic" && backend != "hybrid" {
        return Err("backend must be cpu, hybrid or classic".into());
    }
    #[cfg(not(feature = "replay-hybrid"))]
    if backend == "hybrid" {
        return Err("hybrid mode requires feature replay-hybrid".into());
    }
    let start = Instant::now();
    let handles = netrender::boot()?;
    receipt["gpu_initialized"] = json!(true);
    let adapter = handles.adapter.get_info();
    receipt["adapter"] = json!({"name":adapter.name,"backend":format!("{:?}",adapter.backend),
        "device_type":format!("{:?}",adapter.device_type),"driver":adapter.driver,"driver_info":adapter.driver_info});
    let readback = netrender::WgpuDevice::with_external(handles.clone())
        .map_err(|e| format!("readback: {e:?}"))?;
    let texture = handles.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("intact panel replay"),
        size: wgpu::Extent3d {
            width: scene.viewport_width,
            height: scene.viewport_height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::STORAGE_BINDING
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    if backend == "classic" {
        let renderer = netrender::create_netrender_instance(
            handles.clone(),
            netrender::NetrenderOptions {
                enable_vello: true,
                tile_cache_size: Some(256),
                ..Default::default()
            },
        )
        .map_err(|e| format!("Classic initialization: {e:?}"))?;
        receipt["initialize_ms"] = json!(start.elapsed().as_secs_f64() * 1000.0);
        let start = Instant::now();
        renderer.render_vello(
            scene,
            &texture.create_view(&Default::default()),
            netrender::ColorLoad::Clear(wgpu::Color::TRANSPARENT),
        );
        receipt["render_encode_submit_ms"] = json!(start.elapsed().as_secs_f64() * 1000.0);
    } else {
        #[cfg(feature = "replay-hybrid")]
        {
            use netrender::vello_backends::{HybridSession, SparseResourceLimits};
            let mut session = HybridSession::new(
                &handles.device,
                &handles.queue,
                SparseResourceLimits::default(),
            )?;
            receipt["initialize_ms"] = json!(start.elapsed().as_secs_f64() * 1000.0);
            let mut encoder =
                handles
                    .device
                    .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                        label: Some("intact panel Hybrid"),
                    });
            let start = Instant::now();
            let result = session.render(
                scene,
                &handles.device,
                &handles.queue,
                &mut encoder,
                &texture,
            );
            receipt["session_stats"] = stats(session.stats(), session.text_stats());
            if let Err(error) = result {
                refusal(&error, scene, receipt);
                return Err(error.into());
            }
            handles.queue.submit([encoder.finish()]);
            receipt["render_encode_submit_ms"] = json!(start.elapsed().as_secs_f64() * 1000.0);
        }
    }
    let start = Instant::now();
    handles.device.poll(wgpu::PollType::wait_indefinitely())?;
    receipt["gpu_completion_wait_ms"] = json!(start.elapsed().as_secs_f64() * 1000.0);
    let start = Instant::now();
    let pixels = readback.read_rgba8_texture(&texture, scene.viewport_width, scene.viewport_height);
    receipt["readback_ms"] = json!(start.elapsed().as_secs_f64() * 1000.0);
    Ok(pixels)
}

fn replay(
    packet: &Path,
    reference_path: &Path,
    outdir: &Path,
    backend: &str,
    scale: Option<f32>,
    receipt: &mut Value,
) -> Result<()> {
    let bytes = fs::read(packet)?;
    receipt["packet_sha256"] = json!(hash(&bytes));
    receipt["packet_bytes"] = json!(bytes.len());
    receipt["reference_sha256"] = json!(hash(&fs::read(reference_path)?));
    let start = Instant::now();
    let envelope: PaintEnvelope = postcard::from_bytes(&bytes)?;
    if postcard::to_allocvec(&envelope)? != bytes {
        return Err("packet failed lossless postcard roundtrip".into());
    }
    receipt["lossless_round_trip"] = json!(true);
    let mut histogram = BTreeMap::<_, usize>::new();
    for command in &envelope.commands {
        *histogram.entry(command_name(command)).or_default() += 1;
    }
    receipt["commands"] = json!(envelope.commands.len());
    receipt["command_counts"] = json!(histogram);
    receipt["engine"] = json!(envelope.engine.0);
    receipt["generation"] = json!(envelope.generation);
    receipt["fonts"] = json!(envelope.fonts.iter().map(|f| json!({"key":format!("{:?}", f.key),"index":f.index,"bytes":f.data.len(),"sha256":hash(&f.data)})).collect::<Vec<_>>());
    receipt["images"] = json!(envelope.images.iter().map(|i| json!({"key":format!("{:?}",i.key),"size":[i.width,i.height],"bytes":i.data.len(),"sha256":hash(&i.data)})).collect::<Vec<_>>());
    let runs: Vec<_> = envelope
        .commands
        .iter()
        .filter_map(|c| {
            if let PaintCmd::DrawText(t) = c {
                Some(t)
            } else {
                None
            }
        })
        .collect();
    receipt["text_runs"] = json!(runs.len());
    receipt["glyphs"] = json!(runs.iter().map(|r| r.glyphs.len()).sum::<usize>());
    receipt["positioned_text_sha256"] = json!(hash(&postcard::to_allocvec(&runs)?));
    validate_packet(&envelope)?;
    if !runs.is_empty() {
        let mut broken = envelope.clone();
        broken.fonts.clear();
        if validate_packet(&broken).is_ok() {
            return Err("missing-font negative control unexpectedly passed".into());
        }
        receipt["missing_font_control_rejected"] = json!(true);
    }
    let translated = paint_list_render::translate_envelope_with_external_textures(&envelope);
    receipt["box_shadow_masks"] = json!(translated.box_shadow_masks.len());
    receipt["external_textures"] = json!(translated.external_textures.len());
    if !translated.external_textures.is_empty() || !translated.box_shadow_masks.is_empty() {
        return Err("replay requires unavailable external producer or generated shadow-mask resources; scene was not stripped".into());
    }
    let logical = translated.scene;
    verify_identity(&envelope, &logical)?;
    receipt["font_and_glyph_identity"] = json!(true);
    let (width, height, reference) = read_png(reference_path)?;
    let scale = scale.unwrap_or(width as f32 / logical.viewport_width as f32);
    if !scale.is_finite()
        || scale <= 0.0
        || (f64::from(logical.viewport_width) * f64::from(scale) - f64::from(width)).abs() > 0.01
        || (f64::from(logical.viewport_height) * f64::from(scale) - f64::from(height)).abs() > 0.01
    {
        return Err("reference dimensions require exact uniform presentation scale (or explicit matching --scale)".into());
    }
    receipt["logical_viewport"] = json!([logical.viewport_width, logical.viewport_height]);
    receipt["physical_viewport"] = json!([width, height]);
    receipt["presentation_scale"] = json!(scale);
    let scene = physical_scene(&logical, width, height, scale)?;
    let mut histogram = BTreeMap::<_, usize>::new();
    for op in &scene.ops {
        *histogram.entry(op_name(op)).or_default() += 1;
    }
    receipt["scene_ops"] = json!(scene.ops.len());
    receipt["scene_op_counts"] = json!(histogram);
    receipt["prepare_ms"] = json!(start.elapsed().as_secs_f64() * 1000.0);
    let mut pixels = render(&scene, backend, receipt)?;
    if pixels.len() != reference.len() {
        return Err("output size mismatch".into());
    }
    receipt["raw_rgba_sha256"] = json!(hash(&pixels));
    receipt["raw_alpha_convention"] = json!(if backend == "classic" {
        "straight"
    } else {
        "premultiplied"
    });
    receipt["raw_nonopaque_pixels"] = json!(pixels
        .chunks_exact(4)
        .filter(|pixel| pixel[3] != 255)
        .count());
    present_over_black(&mut pixels, backend);
    let changed = pixels
        .chunks_exact(4)
        .zip(reference.chunks_exact(4))
        .filter(|(a, b)| a != b)
        .count();
    let max_delta = pixels
        .iter()
        .zip(&reference)
        .map(|(a, b)| a.abs_diff(*b))
        .max()
        .unwrap_or(0);
    let total_delta: u64 = pixels
        .iter()
        .zip(&reference)
        .map(|(a, b)| u64::from(a.abs_diff(*b)))
        .sum();
    receipt["comparison"] = json!({"changed_pixels":changed,"max_channel_delta":max_delta,"mean_absolute_channel_delta":total_delta as f64 / pixels.len() as f64,
        "policy":"measure only; anti-aliasing equality is not assumed across renderers"});
    receipt["presented_rgba_sha256"] = json!(hash(&pixels));
    let output = outdir.join(format!("{backend}.png"));
    write_png(&output, width, height, &pixels)?;
    receipt["output"] = json!(output);
    Ok(())
}

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 4 && args.len() != 6 {
        return Err(
            "usage: sparse_panel_replay PACKET REFERENCE.png OUTDIR cpu|hybrid|classic [--scale N]"
                .into(),
        );
    }
    let backend = args[3].to_str().ok_or("invalid backend string")?;
    if !["cpu", "hybrid", "classic"].contains(&backend) {
        return Err("unknown backend".into());
    }
    let scale = if args.len() == 6 {
        if args[4] != "--scale" {
            return Err("expected --scale".into());
        }
        Some(
            args[5]
                .to_str()
                .ok_or("invalid scale string")?
                .parse::<f32>()?,
        )
    } else {
        None
    };
    let outdir = PathBuf::from(&args[2]);
    fs::create_dir_all(&outdir)?;
    let receipt_path = outdir.join(format!("{backend}.json"));
    // Refuse accidental replacement of earlier replay evidence.
    let receipt_file = File::options()
        .write(true)
        .create_new(true)
        .open(receipt_path)?;
    let mut receipt = json!({"kind":"intact-paint-envelope-replay","backend":backend,"packet":PathBuf::from(&args[0]),"reference":PathBuf::from(&args[1]),
        "gpu_initialized":false,"clear":"transparent; backend alpha convention converted to opaque-black presentation after readback",
        "scope":"one cold frame; render encoding/submission, GPU completion wait, and readback measured separately; not interactive performance",
        "session_limits":"unchanged SparseResourceLimits and SparseTextLimits defaults",
        "working_tree_revision":std::process::Command::new("git").args(["rev-parse","HEAD"]).output().ok().filter(|o|o.status.success()).map(|o|String::from_utf8_lossy(&o.stdout).trim().to_owned())});
    let result = replay(
        Path::new(&args[0]),
        Path::new(&args[1]),
        &outdir,
        backend,
        scale,
        &mut receipt,
    );
    receipt["status"] = json!(if result.is_ok() {
        "rendered"
    } else {
        "refused_or_failed"
    });
    if let Err(error) = &result {
        receipt["error"] = json!(error.to_string());
    }
    serde_json::to_writer_pretty(receipt_file, &receipt)?;
    println!("{}", serde_json::to_string_pretty(&receipt)?);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn black_presentation_respects_backend_alpha_conventions() {
        let mut straight = [200, 100, 40, 128, 255, 255, 255, 0];
        let mut premul = [100, 50, 20, 128, 0, 0, 0, 0];
        present_over_black(&mut straight, "classic");
        present_over_black(&mut premul, "hybrid");
        assert_eq!(straight, [100, 50, 20, 255, 0, 0, 0, 255]);
        assert_eq!(straight, premul);
    }

    #[test]
    fn presentation_scales_world_and_device_clip_once() {
        let mut scene = Scene::new(20, 20);
        scene.push_rect(1.0, 2.0, 3.0, 4.0, [1.0; 4]);
        let SceneOp::Rect(rect) = &mut scene.ops[0] else {
            unreachable!()
        };
        rect.clip_rect = [2.0, 3.0, 5.0, 7.0];
        rect.clip_corner_radii = [1.0; 4];
        let scaled = physical_scene(&scene, 40, 40, 2.0).unwrap();
        assert_eq!(scaled.transforms[0].m, Transform::IDENTITY.m);
        let SceneOp::Rect(rect) = &scaled.ops[0] else {
            unreachable!()
        };
        assert_eq!([rect.x0, rect.y0, rect.x1, rect.y1], [1.0, 2.0, 3.0, 4.0]);
        assert_eq!(rect.clip_rect, [4.0, 6.0, 10.0, 14.0]);
        assert_eq!(rect.clip_corner_radii, [2.0; 4]);
        assert_eq!(
            scaled.transforms[rect.transform_id as usize].m,
            Transform::scale_2d(2.0, 2.0).m
        );
    }
}
