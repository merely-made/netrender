// Copyright 2026 Mark Alan Boykin
// SPDX-License-Identifier: MPL-2.0
//! Complete-resource replay: resource_bundle_parity CAPTURE.postcard OUTDIR cpu|classic|gpu|all
//! Use --bundle for an already-materialized bundle, or --fixture as the input
//! for a finite transparent-overlap control. Enable resource-bundle,replay-cpu;
//! gpu/all additionally require replay-hybrid. CPU selection never boots a GPU.
//! Shadow masks require authoritative production payloads in --bundle input;
//! CPU materialization refuses them rather than inventing an approximation.
#![cfg(feature = "resource-bundle")]
use netrender::{Scene, SceneOp, Transform, NO_CLIP};
use paint_list_render::bundle::*;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    error::Error,
    fs::{self, File},
    path::Path,
    time::Instant,
};
type Result<T> = std::result::Result<T, Box<dyn Error>>;
fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
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
            receipt["session_stats"] = json!({"resources":format!("{:?}",session.stats()),"text":format!("{:?}",session.text_stats())});
            receipt["gpu_initialized"] = json!(false);
            receipt["gpu_completion_wait_ms"] = Value::Null;
            receipt["readback_ms"] = Value::Null;
            return match pixels {
                Ok(pixels) => Ok(pixels.data_as_u8_slice().to_vec()),
                Err(error) => Err(error.into()),
            };
        }
        #[cfg(not(feature = "replay-cpu"))]
        return Err("cpu mode requires feature replay-cpu".into());
    }
    if backend != "classic" && backend != "gpu" {
        return Err("backend must be cpu, gpu or classic".into());
    }
    #[cfg(not(feature = "replay-hybrid"))]
    if backend == "gpu" {
        return Err("hybrid mode requires feature replay-hybrid".into());
    }
    let start = Instant::now();
    let handles = netrender::boot()?;
    receipt["gpu_initialized"] = json!(true);
    let adapter = handles.adapter.get_info();
    receipt["adapter"] = json!({"name":adapter.name,"backend":format!("{:?}",adapter.backend),
        "device_type":format!("{:?}",adapter.device_type),"driver":adapter.driver,"driver_info":adapter.driver_info});
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
    let mut classic_renderer = None;
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
        receipt["classic_quality_before_readback"] = json!(renderer.vello_curve_quality_status());
        classic_renderer = Some(renderer);
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
            receipt["session_stats"] = json!({"resources":format!("{:?}",session.stats()),"text":format!("{:?}",session.text_stats())});
            if let Err(error) = result {
                return Err(error.into());
            }
            handles.queue.submit([encoder.finish()]);
            receipt["render_encode_submit_ms"] = json!(start.elapsed().as_secs_f64() * 1000.0);
        }
    }
    let start = Instant::now();
    let row_bytes = scene.viewport_width * 4;
    let padded = row_bytes.next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
    let buffer = handles.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("bounded complete-bundle readback"),
        size: u64::from(padded) * u64::from(scene.viewport_height),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = handles.device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
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
                rows_per_image: Some(scene.viewport_height),
            },
        },
        wgpu::Extent3d {
            width: scene.viewport_width,
            height: scene.viewport_height,
            depth_or_array_layers: 1,
        },
    );
    // Dedicated device and this exact submission bound completion to our own work.
    // Queue ordering puts our render before this copy, with one five-second deadline.
    let submission = handles.queue.submit([encoder.finish()]);
    let slice = buffer.slice(..);
    let (sender, receiver) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = sender.send(result);
    });
    let deadline = Instant::now() + std::time::Duration::from_secs(5);
    handles.device.poll(wgpu::PollType::Wait {
        submission_index: Some(submission),
        timeout: Some(deadline.saturating_duration_since(Instant::now())),
    })?;
    receiver.recv_timeout(deadline.saturating_duration_since(Instant::now()))??;
    if let Some(renderer) = classic_renderer {
        let status = renderer
            .poll_vello_curve_quality()?
            .ok_or("Classic quality counters unavailable")?;
        receipt["classic_quality_after_readback"] =
            json!({"pending_frames":status.0,"pending_bytes":status.1});
        if status != (0, 0) {
            return Err(
                "bounded Classic readback completed with unobserved quality counters".into(),
            );
        }
    }
    let mapped = slice.get_mapped_range()?;
    let mut pixels = Vec::with_capacity(row_bytes as usize * scene.viewport_height as usize);
    for row in mapped.chunks_exact(padded as usize) {
        pixels.extend_from_slice(&row[..row_bytes as usize]);
    }
    drop(mapped);
    buffer.unmap();
    receipt["bounded_submit_readback_ms"] = json!(start.elapsed().as_secs_f64() * 1000.0);
    receipt["deadline_seconds"] = json!(5);
    Ok(pixels)
}

fn fixture() -> PortablePaintCapture {
    use paint_list_api::*;
    let rect = |a: f32, b: f32, c: f32, d: f32| {
        LayoutRect::new(LayoutPoint::new(a, b), LayoutPoint::new(c, d))
    };
    let mut source = Scene::new(16, 16);
    source.push_rect(1.0, 1.0, 11.0, 12.0, [0.45, 0.075, 0.025, 0.5]);
    source.push_rect(6.0, 4.0, 15.0, 15.0, [0.02, 0.08, 0.38, 0.4]);
    PortablePaintCapture {
        envelope: PaintEnvelope {
            engine: EngineId::GENET,
            viewport: DeviceIntSize::new(24, 24),
            generation: 1,
            commands: vec![
                PaintCmd::DrawBorder(BorderItem {
                    placement: CommonPlacement::new(rect(0.0, 0.0, 24.0, 24.0)),
                    widths: LayoutSideOffsets::new(1.0, 1.0, 1.0, 7.0),
                    details: BorderDetails::Normal(NormalBorder {
                        left: BorderSide {
                            style: BorderStyle::Solid,
                            color: ColorF::new(0.2, 0.4, 0.78431374, 0.2863),
                        },
                        top: BorderSide {
                            style: BorderStyle::Solid,
                            color: ColorF::new(0.41960785, 0.5529412, 0.6666667, 1.0),
                        },
                        right: BorderSide {
                            style: BorderStyle::Solid,
                            color: ColorF::new(0.41960785, 0.5529412, 0.6666667, 1.0),
                        },
                        bottom: BorderSide {
                            style: BorderStyle::Solid,
                            color: ColorF::new(0.41960785, 0.5529412, 0.6666667, 1.0),
                        },
                        radius: BorderRadius {
                            top_left: LayoutSize::new(5.0, 3.0),
                            top_right: LayoutSize::new(5.0, 3.0),
                            bottom_left: LayoutSize::new(5.0, 3.0),
                            bottom_right: LayoutSize::new(5.0, 3.0),
                        },
                        do_aa: true,
                    }),
                }),
                PaintCmd::DrawRect(RectItem {
                    placement: CommonPlacement::new(rect(3.0, 3.0, 19.0, 20.0)),
                    color: ColorF::new(0.1, 0.8, 0.2, 0.35),
                }),
                PaintCmd::DrawExternalTexture(ExternalTextureItem {
                    placement: CommonPlacement::new(rect(4.0, 4.0, 20.0, 20.0)),
                    texture_key: 1,
                    opacity: 0.7,
                    content_generation: Some(1),
                }),
                PaintCmd::DrawRect(RectItem {
                    placement: CommonPlacement::new(rect(10.0, 9.0, 22.0, 22.0)),
                    color: ColorF::new(1.0, 0.7, 0.1, 0.25),
                }),
            ],
            fonts: vec![],
            images: vec![],
        },
        producers: vec![ProducerSource {
            texture_key: 1,
            content_generation: 1,
            source_identity: 1,
            source_revision: 0,
            logical_size: [16.0, 16.0],
            physical_size: [16, 16],
            layout_scale: 1.0,
            alpha: ResourceAlpha::Straight,
            encoding: ResourceEncoding::Srgb,
            scene: source,
        }],
    }
}
fn materialize(capture: PortablePaintCapture) -> Result<CompleteResourceBundle> {
    validate_capture(&capture)?;
    let translated = translate_capture(&capture)?;
    if let Some(shadow) = translated.box_shadow_masks.first() {
        return Err(BundleError::MissingResource(ResourceRef::ShadowMask(shadow.key)).into());
    }
    let mut resources = Vec::new();
    for source in &capture.producers {
        let physical = physical_scene(
            &source.scene,
            source.physical_size[0],
            source.physical_size[1],
            source.layout_scale,
        )?;
        let pixels = render(&physical, "cpu", &mut json!({}))?;
        resources.push(cpu_payload(
            ResourceRef::ExternalTexture(source.texture_key),
            source.binding()?,
            source.physical_size,
            source.alpha,
            pixels,
        )?);
    }
    Ok(CompleteResourceBundle {
        version: BUNDLE_VERSION,
        capture_digest: capture_digest(&capture)?,
        capture,
        translated,
        resources,
    })
}
fn premultiplied(bytes: &[u8], backend: &str) -> Vec<u8> {
    let mut out = bytes.to_vec();
    // Classic fine.wgsl explicitly separates RGB from alpha before textureStore.
    // CPU/GPU sparse pixmaps keep premultiplied bytes. Compare in premultiplied
    // encoded-sRGB space while retaining and comparing every alpha byte.
    if backend == "classic" {
        for p in out.chunks_exact_mut(4) {
            let alpha = u16::from(p[3]);
            for channel in &mut p[..3] {
                *channel = ((u16::from(*channel) * alpha + 127) / 255) as u8;
            }
        }
    }
    out
}
fn straight_png(bytes: &[u8], backend: &str) -> Result<Vec<u8>> {
    if backend == "classic" {
        return Ok(bytes.to_vec());
    }
    let mut out = bytes.to_vec();
    for p in out.chunks_exact_mut(4) {
        let a = u32::from(p[3]);
        for c in &mut p[..3] {
            if u32::from(*c) > a {
                return Err("renderer output violates premultiplied alpha".into());
            }
            *c = if a == 0 {
                0
            } else {
                ((u32::from(*c) * 255 + a / 2) / a).min(255) as u8
            };
        }
    }
    Ok(out)
}
fn write_png(path: &Path, w: u32, h: u32, bytes: &[u8]) -> Result<()> {
    let f = File::options().write(true).create_new(true).open(path)?;
    let mut e = png::Encoder::new(f, w, h);
    e.set_color(png::ColorType::Rgba);
    e.set_depth(png::BitDepth::Eight);
    e.write_header()?.write_image_data(bytes)?;
    Ok(())
}
/// Classify an edge solely from the CPU reference: a one-pixel neighborhood
/// with any RGBA variation >8. Thresholds are recorded separately from exact
/// capture/translation/resource identity. A large error at even one interior
/// pixel refuses acceptance; alpha never becomes opaque for comparison.
fn comparison(reference: &[u8], actual: &[u8], w: u32, h: u32) -> Value {
    let mut edge_pixels = 0;
    let mut interior_pixels = 0;
    let mut edge_max = 0u8;
    let mut interior_max = 0u8;
    let mut alpha_edge = 0u8;
    let mut alpha_interior = 0u8;
    let mut sum = 0u64;
    let mut changed = 0;
    for y in 0..h as usize {
        for x in 0..w as usize {
            let i = (y * w as usize + x) * 4;
            let p = &reference[i..i + 4];
            let mut edge = false;
            for yy in y.saturating_sub(1)..=(y + 1).min(h as usize - 1) {
                for xx in x.saturating_sub(1)..=(x + 1).min(w as usize - 1) {
                    let j = (yy * w as usize + xx) * 4;
                    if (0..4).any(|c| p[c].abs_diff(reference[j + c]) > 8) {
                        edge = true;
                    }
                }
            }
            let delta = (0..4)
                .map(|c| reference[i + c].abs_diff(actual[i + c]))
                .max()
                .unwrap();
            let alpha = reference[i + 3].abs_diff(actual[i + 3]);
            sum += (0..4)
                .map(|c| u64::from(reference[i + c].abs_diff(actual[i + c])))
                .sum::<u64>();
            if delta > 0 {
                changed += 1;
            }
            if edge {
                edge_pixels += 1;
                edge_max = edge_max.max(delta);
                alpha_edge = alpha_edge.max(alpha);
            } else {
                interior_pixels += 1;
                interior_max = interior_max.max(delta);
                alpha_interior = alpha_interior.max(alpha);
            }
        }
    }
    let mean = sum as f64 / reference.len() as f64;
    json!({"exact_pixels":reference==actual,"accepted":edge_max<=32&&interior_max<=3&&alpha_edge<=16&&alpha_interior<=2&&mean<=1.0,
        "edge_pixels":edge_pixels,"interior_pixels":interior_pixels,"edge_max_rgba_delta":edge_max,"interior_max_rgba_delta":interior_max,
        "edge_max_alpha_delta":alpha_edge,"interior_max_alpha_delta":alpha_interior,"mean_channel_delta":mean,"changed_pixels":changed})
}
fn run() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() < 3 {
        return Err("usage: resource_bundle_parity CAPTURE.postcard|--fixture OUTDIR cpu|classic|gpu|all [--bundle]".into());
    }
    let mode = &args[2];
    if !["cpu", "classic", "gpu", "all"].contains(&mode.as_str()) {
        return Err("invalid runtime selector".into());
    }
    let out = Path::new(&args[1]);
    fs::create_dir_all(out)?;
    if out.join("receipt.json").exists() {
        return Err("output already has a receipt".into());
    }
    let bundle = if args[0] == "--fixture" {
        materialize(fixture())?
    } else {
        let file = File::open(&args[0])?;
        if file.metadata()?.len() > 512 * 1024 * 1024 {
            return Err("input exceeds file byte budget".into());
        }
        let bytes = fs::read(&args[0])?;
        if args.iter().any(|a| a == "--bundle") {
            postcard::from_bytes::<CompleteResourceBundle>(&bytes)?
        } else {
            materialize(postcard::from_bytes::<PortablePaintCapture>(&bytes)?)?
        }
    };
    // Bind admission to this explicitly selected local capture, independently
    // of the bundle's claimed identity field.
    validate_capture(&bundle.capture)?;
    let expected = capture_digest(&bundle.capture)?;
    let scene = admit_bundle(&bundle, expected, BundleLimits::default())?;
    let encoded = postcard::to_allocvec(&bundle)?;
    fs::write(out.join("complete-bundle.postcard"), &encoded)?;
    let mut cpu_receipt = json!({});
    let cpu = render(&scene, "cpu", &mut cpu_receipt)?;
    write_png(
        &out.join("cpu.png"),
        scene.viewport_width,
        scene.viewport_height,
        &straight_png(&cpu, "cpu")?,
    )?;
    cpu_receipt["pixels_sha256"] = json!(hash(&cpu));
    cpu_receipt["comparison"] = comparison(&cpu, &cpu, scene.viewport_width, scene.viewport_height);
    let mut renders = json!({"cpu":cpu_receipt});
    let mut passed = true;
    let modes: Vec<&str> = match mode.as_str() {
        "cpu" => vec![],
        "all" => vec!["classic", "gpu"],
        other => vec![other],
    };
    let mut producer_parity = Vec::new();
    for source in &bundle.capture.producers {
        let physical = physical_scene(
            &source.scene,
            source.physical_size[0],
            source.physical_size[1],
            source.layout_scale,
        )?;
        let mut source_cpu_receipt = json!({});
        let source_cpu = render(&physical, "cpu", &mut source_cpu_receipt)?;
        write_png(
            &out.join(format!("producer-{}-cpu.png", source.texture_key)),
            physical.viewport_width,
            physical.viewport_height,
            &straight_png(&source_cpu, "cpu")?,
        )?;
        let mut source_renders = json!({"cpu":source_cpu_receipt});
        for backend in &modes {
            let mut source_receipt = json!({});
            let raw = render(&physical, backend, &mut source_receipt)?;
            let normalized = premultiplied(&raw, backend);
            let diff = comparison(
                &source_cpu,
                &normalized,
                physical.viewport_width,
                physical.viewport_height,
            );
            passed &= diff["accepted"].as_bool().unwrap();
            source_receipt["comparison"] = diff;
            source_receipt["raw_pixels_sha256"] = json!(hash(&raw));
            write_png(
                &out.join(format!("producer-{}-{backend}.png", source.texture_key)),
                physical.viewport_width,
                physical.viewport_height,
                &straight_png(&raw, backend)?,
            )?;
            source_renders[*backend] = source_receipt;
        }
        producer_parity.push(json!({"texture_key":source.texture_key,"binding":source.binding()?,"renders":source_renders}));
    }
    for backend in modes {
        let mut receipt = json!({});
        let raw = render(&scene, backend, &mut receipt)?;
        let normalized = premultiplied(&raw, backend);
        let diff = comparison(
            &cpu,
            &normalized,
            scene.viewport_width,
            scene.viewport_height,
        );
        passed &= diff["accepted"].as_bool().unwrap();
        receipt["comparison"] = diff;
        receipt["raw_pixels_sha256"] = json!(hash(&raw));
        receipt["premultiplied_pixels_sha256"] = json!(hash(&normalized));
        write_png(
            &out.join(format!("{backend}.png")),
            scene.viewport_width,
            scene.viewport_height,
            &straight_png(&raw, backend)?,
        )?;
        renders[backend] = receipt;
    }
    let resources:Vec<_>=bundle.resources.iter().map(|p|json!({"resource":p.resource,"binding":p.binding,"width":p.width,"height":p.height,
        "alpha":p.alpha,"encoding":p.encoding,"rgba_sha256":hash(&p.rgba),"provider":if args.iter().any(|a|a=="--bundle") {"caller-authoritative-complete-bundle"} else {"bound-source-scene-cpu-raster"}})).collect();
    let receipt = json!({"schema_version":1,"selector":mode,"passed":passed,"fixture":args[0]=="--fixture",
        "bundle_sha256":hash(&encoded),"capture_digest":expected,"exact_identity_admitted":true,
        "identity_scope":"intact envelope, callback scene provenance, full translated scene and external/shadow side channels",
        "comparison_space":"premultiplied encoded-sRGB RGBA8 including alpha; no background matte",
        "pixel_thresholds":{"edge_max_rgba":32,"interior_max_rgba":3,"edge_max_alpha":16,"interior_max_alpha":2,"mean_channel_delta":1.0,"edge_reference_neighbor_variation":8},
        "resource_count":resources.len(),"resources":resources,"renders":renders,"producer_source_parity":producer_parity,
        "scope":"finite offscreen renderer/resource replay; host presentation, input, accessibility, and reset recovery are separate gates"});
    fs::write(
        out.join("receipt.json"),
        serde_json::to_vec_pretty(&receipt)?,
    )?;
    if !passed {
        return Err("pixel acceptance thresholds exceeded; receipt preserved".into());
    }
    Ok(())
}
fn main() {
    if let Err(e) = run() {
        eprintln!("resource bundle parity refused: {e}");
        std::process::exit(1);
    }
}
