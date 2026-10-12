// Copyright 2026 Mark Alan Boykin
// SPDX-License-Identifier: MPL-2.0
//! Finite Classic edge isolation: classic_edge_probe OUTDIR cpu|classic|gpu|all
//! [--case NAME] [--bundle PATH --producer KEY --glyph-op INDEX].
//! CPU mode never initializes a GPU. All GPU submissions/readbacks are bounded
//! by their own five-second deadline. Diagnostic receipt flags are authoritative;
//! a completed diagnostic exits zero even when parity remains red.
#![cfg(feature = "resource-bundle")]
#[path = "classic_edge_probe/fixtures.rs"]
mod fixtures;
use netrender::{Scene, SceneOp};
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

fn option<'a>(args: &'a [String], flag: &str) -> Result<Option<&'a str>> {
    match args.iter().position(|v| v == flag) {
        Some(i) => Ok(Some(
            args.get(i + 1).ok_or("option missing value")?.as_str(),
        )),
        None => Ok(None),
    }
}
fn run() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() < 2 {
        return Err("usage: classic_edge_probe OUTDIR cpu|classic|gpu|all [--case NAME] [--bundle PATH --producer KEY --glyph-op INDEX]".into());
    }
    let mode = args[1].as_str();
    if !["cpu", "classic", "gpu", "all"].contains(&mode) {
        return Err("invalid runtime selector".into());
    }
    let out = Path::new(&args[0]);
    fs::create_dir_all(out)?;
    if out.join("receipt.json").exists() {
        return Err("output receipt already exists".into());
    }
    let mut cases = fixtures::geometry_cases();
    let mut input = Value::Null;
    if let Some(path) = option(&args, "--bundle")? {
        let file = File::open(path)?;
        if file.metadata()?.len() > 512 * 1024 * 1024 {
            return Err("bundle exceeds byte limit".into());
        }
        let bytes = fs::read(path)?;
        let bundle: CompleteResourceBundle = postcard::from_bytes(&bytes)?;
        validate_capture(&bundle.capture)?;
        let expected = capture_digest(&bundle.capture)?;
        admit_bundle(&bundle, expected, BundleLimits::default())?;
        let key: u64 = option(&args, "--producer")?
            .ok_or("bundle requires explicit --producer")?
            .parse()?;
        let producer = bundle
            .capture
            .producers
            .iter()
            .find(|s| s.texture_key == key)
            .ok_or("selected producer absent")?;
        let candidates:Vec<_>=producer.scene.ops.iter().enumerate().filter_map(|(i,op)|if let SceneOp::GlyphRun(r)=op {Some(json!({"op_index":i,"font_id":r.font_id,"font_size":r.font_size,"glyph_count":r.glyphs.len(),"first_glyph":r.glyphs.first().map(|g|json!({"id":g.id,"x":g.x,"y":g.y})),"transform_id":r.transform_id}))}else{None}).collect();
        input = json!({"bundle_path":path,"bundle_sha256":hash(&bytes),"producer_key":key,"binding":producer.binding()?,"logical_viewport":[producer.scene.viewport_width,producer.scene.viewport_height],"glyph_candidates":candidates,"font_bytes_written":false});
        if let Some(index) = option(&args, "--glyph-op")? {
            let mut glyphs = fixtures::glyph_cases(&producer.scene, index.parse()?)?;
            for case in &mut glyphs {
                // Record only source font hashes, never font-bearing Scene data.
                case.details["font_sha256"] = json!(producer
                    .scene
                    .fonts
                    .iter()
                    .map(|f| hash(f.data.data()))
                    .collect::<Vec<_>>());
                case.details["presentation_scope"] = json!(
                    "logical callback viewport; producer layout scale is not silently applied"
                );
            }
            cases.extend(glyphs);
        }
    } else if option(&args, "--producer")?.is_some() || option(&args, "--glyph-op")?.is_some() {
        return Err("glyph selection requires --bundle".into());
    }
    if let Some(name) = option(&args, "--case")? {
        cases.retain(|c| c.name == name);
        if cases.is_empty() {
            return Err("unknown case".into());
        }
    }
    if args.iter().any(|a| a == "--list") {
        let listed: Vec<_> = cases.iter().map(|c| json!({"name":c.name,"viewport":[c.scene.viewport_width,c.scene.viewport_height],"details":c.details})).collect();
        fs::write(
            out.join("receipt.json"),
            serde_json::to_vec_pretty(
                &json!({"schema_version":1,"scope":"source-only fixture/glyph inventory; no rasterization","gpu_initialized":false,"input":input,"cases":listed}),
            )?,
        )?;
        println!("source inventory: {}", out.join("receipt.json").display());
        return Ok(());
    }
    let mut receipts = Vec::new();
    let mut samples = std::collections::HashMap::<(String, String), (Vec<u8>, u32, u32)>::new();
    let mut accepted = true;
    for case in cases {
        validate_source_scene(&case.scene, BundleLimits::default())?;
        let case_dir = out.join(&case.name);
        fs::create_dir_all(&case_dir)?;
        let mut cpu_receipt = json!({});
        let cpu = render(&case.scene, "cpu", &mut cpu_receipt)?;
        write_png(
            &case_dir.join("cpu.png"),
            case.scene.viewport_width,
            case.scene.viewport_height,
            &straight_png(&cpu, "cpu")?,
        )?;
        cpu_receipt["raw_pixels_sha256"] = json!(hash(&cpu));
        let mut renders = json!({"cpu":cpu_receipt});
        samples.insert(
            (case.name.clone(), "cpu".into()),
            (
                cpu.clone(),
                case.scene.viewport_width,
                case.scene.viewport_height,
            ),
        );
        for backend in ["classic", "gpu"] {
            if mode != backend && mode != "all" {
                continue;
            }
            let mut receipt = json!({});
            let raw = render(&case.scene, backend, &mut receipt)?;
            let premul = premultiplied(&raw, backend);
            let metrics = comparison(
                &cpu,
                &premul,
                case.scene.viewport_width,
                case.scene.viewport_height,
            );
            accepted &= metrics["accepted"] == true;
            receipt["comparison"] = metrics;
            receipt["raw_pixels_sha256"] = json!(hash(&raw));
            receipt["premultiplied_pixels_sha256"] = json!(hash(&premul));
            let x = 21u32.min(case.scene.viewport_width - 1);
            let i = (x * 4) as usize;
            receipt["premultiplied_probe_21_0"] = json!(&premul[i..i + 4]);
            write_png(
                &case_dir.join(format!("{backend}.png")),
                case.scene.viewport_width,
                case.scene.viewport_height,
                &straight_png(&raw, backend)?,
            )?;
            renders[backend] = receipt;
            samples.insert(
                (case.name.clone(), backend.into()),
                (
                    premul,
                    case.scene.viewport_width,
                    case.scene.viewport_height,
                ),
            );
        }
        let i = (21u32.min(case.scene.viewport_width - 1) * 4) as usize;
        receipts.push(json!({"name":case.name,"viewport":[case.scene.viewport_width,case.scene.viewport_height],"scene_digest":scene_digest(&case.scene)?,"details":case.details,"cpu_premultiplied_probe_21_0":&cpu[i..i+4],"renders":renders}));
    }
    let mut isolations = Vec::new();
    for backend in ["cpu", "classic", "gpu"] {
        for (reference, actual, meaning) in [
            (
                "curve-fill",
                "curve-clip-only",
                "fill versus one curve clip over covering paint",
            ),
            (
                "polyline-fill",
                "polyline-clip-only",
                "identical diagnostic line endpoints as fill versus single clip",
            ),
            (
                "curve-fill",
                "polyline-fill",
                "canonical curve versus diagnostic fine polyline approximation",
            ),
            (
                "curve-clip-only",
                "curve-fill-and-coincident-clip",
                "additional coincident fill coverage",
            ),
            (
                "curve-clip-only",
                "curve-double-coincident-clip",
                "additional coincident clip coverage",
            ),
            (
                "glyph-original-run",
                "glyph-skrifa-draw-size",
                "caller glyph API versus explicit unhinted draw-size paths",
            ),
            (
                "glyph-original-run",
                "glyph-skrifa-upem-scaled",
                "caller glyph API versus explicit unhinted UPEM paths and scale",
            ),
            (
                "glyph-skrifa-draw-size",
                "glyph-skrifa-upem-scaled",
                "outline size and transform representation only",
            ),
        ] {
            if let (Some((a, w, h)), Some((b, bw, bh))) = (
                samples.get(&(reference.into(), backend.into())),
                samples.get(&(actual.into(), backend.into())),
            ) {
                if w != bw || h != bh {
                    return Err("control extent mismatch".into());
                }
                isolations.push(json!({"backend":backend,"reference":reference,"actual":actual,"meaning":meaning,"comparison":comparison(a,b,*w,*h),"scope":"diagnostic comparison; repeated antialiased clip coverage is not an identity gate"}));
            }
        }
    }
    let receipt = json!({"schema_version":1,"scope":"finite curve/clip/rounded-border/unhinted-glyph isolation; no production or provider changes", "selector":mode,"all_selected_cross_renderer_pixel_acceptance":accepted,"comparison_space":"premultiplied encoded-sRGB RGBA8 including alpha; no matte","pixel_thresholds":{"edge_max_rgba":32,"interior_max_rgba":3,"edge_max_alpha":16,"interior_max_alpha":2,"mean_channel_delta":1.0,"edge_reference_neighbor_variation":8},"input":input,"cases":receipts,"within_backend_isolations":isolations,"completion":"exit zero means diagnostic completed; pixel acceptance remains a separate field"});
    fs::write(
        out.join("receipt.json"),
        serde_json::to_vec_pretty(&receipt)?,
    )?;
    println!(
        "diagnostic complete; selected pixel acceptance={accepted}; {}",
        out.join("receipt.json").display()
    );
    Ok(())
}
fn main() {
    if let Err(e) = run() {
        eprintln!("Classic edge probe refused: {e}");
        std::process::exit(1)
    }
}
