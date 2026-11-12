//! L0a — netrender face-throughput ceiling.
//!
//! Bodies as sets of axis-aligned rectangles under one per-body affine
//! transform, rendered headless at 1920x1080 RGBA8. Three modes:
//!
//!   static   — one scene built once, re-rendered every frame
//!   live     — scene rebuilt every frame, every body's transform changes
//!   fragment — one retained `SceneFragment` per body, registered once,
//!              placed every frame under a changing transform
//!
//! Reports medians over N measured frames after warmup, plus netrender's
//! own `last_frame_timings()` spans.

use std::time::{Duration, Instant};

use netrender::scene::{SceneFragment, Transform};
use netrender::{ColorLoad, NetrenderOptions, Renderer, Scene, boot_with, create_netrender_instance};

const W: u32 = 1920;
const H: u32 = 1080;
const TILE: u32 = 256;

const BODY_W: f32 = 48.0;
const BODY_H: f32 = 96.0;

const BODIES: [usize; 4] = [10, 100, 1000, 3000];
const RECTS: [usize; 2] = [50, 200];

const WARMUP: usize = 5;
const FRAMES: usize = 60;
/// If a warmup frame is slower than this, drop to `SLOW_FRAMES` measured
/// frames so the whole grid still finishes in a sane wall time.
const SLOW_FRAME_MS: f64 = 150.0;
const SLOW_FRAMES: usize = 10;

// ── tiny deterministic PRNG ───────────────────────────────────────────

struct Rng(u64);
impl Rng {
    fn next_u32(&mut self) -> u32 {
        // xorshift64*
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        (x.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 32) as u32
    }
    fn f32(&mut self) -> f32 {
        self.next_u32() as f32 / u32::MAX as f32
    }
    fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + self.f32() * (hi - lo)
    }
}

// ── body geometry ─────────────────────────────────────────────────────

#[derive(Clone, Copy)]
struct Face {
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    color: [f32; 4],
}

/// One body: `n` axis-aligned rects of 2..24 px inside a 48x96 box,
/// each with a flat colour. Body-local coordinates, origin at 0,0.
fn make_body(n: usize, rng: &mut Rng) -> Vec<Face> {
    (0..n)
        .map(|_| {
            let w = rng.range(2.0, 24.0);
            let h = rng.range(2.0, 24.0);
            let x0 = rng.range(0.0, BODY_W - w);
            let y0 = rng.range(0.0, BODY_H - h);
            Face {
                x0,
                y0,
                x1: x0 + w,
                y1: y0 + h,
                color: [rng.f32(), rng.f32(), rng.f32(), 1.0],
            }
        })
        .collect()
}

/// Where body `i` sits on screen, and its base rotation.
fn body_placement(i: usize, bodies: usize) -> (f32, f32, f32) {
    let cols = (bodies as f32).sqrt().ceil().max(1.0) as usize;
    let rows = bodies.div_ceil(cols);
    let sx = (W as f32 - BODY_W) / cols.max(1) as f32;
    let sy = (H as f32 - BODY_H) / rows.max(1) as f32;
    let cx = (i % cols) as f32 * sx;
    let cy = (i / cols) as f32 * sy;
    (cx, cy, (i % 17) as f32 * 0.03)
}

/// Per-body affine: rotate about the body centre, then translate into
/// place. `t` is the frame phase; 0 for the static scene.
fn body_transform(i: usize, bodies: usize, t: f32) -> Transform {
    let (cx, cy, base) = body_placement(i, bodies);
    let ang = base + t * 0.05 * ((i % 7) as f32 + 1.0);
    let dx = cx + t * 0.7 * ((i % 5) as f32 - 2.0);
    let dy = cy + t * 0.7 * ((i % 3) as f32 - 1.0);
    Transform::translate_2d(-BODY_W * 0.5, -BODY_H * 0.5)
        .then(&Transform::rotate_2d(ang))
        .then(&Transform::translate_2d(
            dx + BODY_W * 0.5,
            dy + BODY_H * 0.5,
        ))
}

fn base_scene() -> Scene {
    let mut s = Scene::new(W, H);
    s.push_rect(0.0, 0.0, W as f32, H as f32, [0.06, 0.06, 0.08, 1.0]);
    s
}

/// Flat scene: every face as its own `push_rect_transformed` under the
/// body's transform (one palette entry per body).
fn build_flat(bodies: &[Vec<Face>], t: f32) -> Scene {
    let mut s = base_scene();
    let n = bodies.len();
    for (i, body) in bodies.iter().enumerate() {
        let xf = s.push_transform(body_transform(i, n, t));
        for f in body {
            s.push_rect_transformed(f.x0, f.y0, f.x1, f.y1, f.color, xf);
        }
    }
    s
}

/// Fragment scene: one `place_fragment` per body.
fn build_fragment_scene(ids: &[u64], t: f32) -> Scene {
    let mut s = base_scene();
    let n = ids.len();
    for (i, id) in ids.iter().enumerate() {
        s.place_fragment(*id, body_transform(i, n, t));
    }
    s
}

// ── peak RSS (Windows) ────────────────────────────────────────────────

#[cfg(windows)]
#[repr(C)]
#[derive(Default)]
struct ProcessMemoryCounters {
    cb: u32,
    page_fault_count: u32,
    peak_working_set_size: usize,
    working_set_size: usize,
    quota_peak_paged_pool: usize,
    quota_paged_pool: usize,
    quota_peak_non_paged_pool: usize,
    quota_non_paged_pool: usize,
    pagefile_usage: usize,
    peak_pagefile_usage: usize,
}

#[cfg(windows)]
unsafe extern "system" {
    fn K32GetProcessMemoryInfo(
        process: isize,
        counters: *mut ProcessMemoryCounters,
        cb: u32,
    ) -> i32;
    fn GetCurrentProcess() -> isize;
}

#[cfg(windows)]
fn peak_rss_mb() -> f64 {
    let mut c = ProcessMemoryCounters {
        cb: size_of::<ProcessMemoryCounters>() as u32,
        ..Default::default()
    };
    let ok = unsafe { K32GetProcessMemoryInfo(GetCurrentProcess(), &mut c, c.cb) };
    if ok == 0 {
        return f64::NAN;
    }
    c.peak_working_set_size as f64 / (1024.0 * 1024.0)
}

#[cfg(not(windows))]
fn peak_rss_mb() -> f64 {
    f64::NAN
}

// ── stats ─────────────────────────────────────────────────────────────

fn median(v: &mut Vec<f64>) -> f64 {
    // Missing spans arrive as NaN; drop them rather than poisoning the sort.
    v.retain(|x| x.is_finite());
    if v.is_empty() {
        return f64::NAN;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = v.len();
    if n % 2 == 1 {
        v[n / 2]
    } else {
        (v[n / 2 - 1] + v[n / 2]) / 2.0
    }
}

/// NaN-safe JSON number (serde_json cannot represent NaN).
fn jf(x: f64) -> serde_json::Value {
    if x.is_finite() {
        serde_json::json!(x)
    } else {
        serde_json::Value::Null
    }
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

// ── one cell ──────────────────────────────────────────────────────────

struct Cell {
    mode: &'static str,
    bodies: usize,
    rects: usize,
    elements: usize,
    frames: usize,
    build_ms: f64,
    encode_ms: f64,
    poll_ms: f64,
    frame_ms: f64,
    span_total_ms: f64,
    span_vello_ms: f64,
    span_master_compose_ms: f64,
    span_dirty_tile_rebuild_ms: f64,
    peak_rss_mb: f64,
    notes: String,
}

fn make_target(device: &wgpu::Device) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("l0a target"),
        size: wgpu::Extent3d {
            width: W,
            height: H,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::STORAGE_BINDING
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&Default::default());
    (texture, view)
}

fn new_renderer(handles: &netrender::WgpuHandles) -> Renderer {
    create_netrender_instance(
        handles.clone(),
        NetrenderOptions {
            tile_cache_size: Some(TILE),
            enable_vello: true,
            ..Default::default()
        },
    )
    .expect("create_netrender_instance")
}

fn span(r: &Renderer, name: &str) -> f64 {
    // `total` is a FrameTimings field, not a named span.
    if name == "total" {
        return r.last_frame_timings().map(|t| ms(t.total)).unwrap_or(f64::NAN);
    }
    r.last_frame_timings()
        .and_then(|t| t.spans.iter().find(|s| s.name == name).map(|s| s.duration))
        .map(ms)
        .unwrap_or(f64::NAN)
}

fn run_cell(
    handles: &netrender::WgpuHandles,
    mode: &'static str,
    nbodies: usize,
    nrects: usize,
) -> Cell {
    let mut rng = Rng(0x1234_5678_9ABC_DEF0 ^ (nrects as u64) << 8);
    let bodies: Vec<Vec<Face>> = (0..nbodies).map(|_| make_body(nrects, &mut rng)).collect();

    let renderer = new_renderer(handles);
    let (_target, view) = make_target(&handles.device);

    let mut notes = String::new();
    let mut frag_ids: Vec<u64> = Vec::new();

    if mode == "fragment" {
        for body in &bodies {
            let mut f = SceneFragment::new();
            for fc in body {
                f.push_rect(fc.x0, fc.y0, fc.x1, fc.y1, fc.color);
            }
            match renderer.register_fragment(f) {
                Some(id) => frag_ids.push(id),
                None => {
                    notes.push_str("register_fragment returned None (vello disabled); ");
                    break;
                }
            }
        }
    }

    // Static mode builds its scene once, outside the measured loop.
    let mut build_once = 0.0;
    let static_scene = if mode == "static" {
        let t0 = Instant::now();
        let s = build_flat(&bodies, 0.0);
        build_once = ms(t0.elapsed());
        Some(s)
    } else {
        None
    };

    let mut builds = Vec::new();
    let mut encodes = Vec::new();
    let mut polls = Vec::new();
    let mut totals = Vec::new();
    let mut sp_total = Vec::new();
    let mut sp_vello = Vec::new();
    let mut sp_compose = Vec::new();
    let mut sp_dirty = Vec::new();

    let mut frames = FRAMES;
    let mut measured = 0usize;
    let mut i = 0usize;
    loop {
        if i >= WARMUP + frames {
            break;
        }
        let phase = i as f32;

        let t_build = Instant::now();
        let scene_owned;
        let scene: &Scene = match mode {
            "static" => static_scene.as_ref().unwrap(),
            "live" => {
                scene_owned = build_flat(&bodies, phase);
                &scene_owned
            }
            "fragment" => {
                scene_owned = build_fragment_scene(&frag_ids, phase);
                &scene_owned
            }
            _ => unreachable!(),
        };
        let build = if mode == "static" {
            0.0
        } else {
            ms(t_build.elapsed())
        };

        let t_enc = Instant::now();
        renderer.render_vello(scene, &view, ColorLoad::Clear(wgpu::Color::BLACK));
        let encode = ms(t_enc.elapsed());

        let t_poll = Instant::now();
        handles
            .device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("device poll");
        let poll = ms(t_poll.elapsed());

        let total = build + encode + poll;

        if i == WARMUP - 1 && total > SLOW_FRAME_MS {
            frames = SLOW_FRAMES;
        }
        if i >= WARMUP {
            builds.push(build);
            encodes.push(encode);
            polls.push(poll);
            totals.push(total);
            sp_total.push(span(&renderer, "total"));
            sp_vello.push(span(&renderer, "vello_render"));
            sp_compose.push(span(&renderer, "master_compose"));
            sp_dirty.push(span(&renderer, "dirty_tile_rebuild"));
            measured += 1;
        }
        i += 1;
    }

    if mode == "fragment" {
        let lowers = renderer.fragment_lower_count().unwrap_or(0);
        let hits = renderer.fragment_master_hits().unwrap_or(0);
        notes.push_str(&format!(
            "fragment_lower_count={lowers} master_hits={hits}; "
        ));
    }
    if let Some(n) = renderer.vello_cached_tile_count() {
        notes.push_str(&format!("cached_tiles={n}; "));
    }

    Cell {
        mode,
        bodies: nbodies,
        rects: nrects,
        elements: nbodies * nrects,
        frames: measured,
        build_ms: if mode == "static" {
            build_once
        } else {
            median(&mut builds)
        },
        encode_ms: median(&mut encodes),
        poll_ms: median(&mut polls),
        frame_ms: median(&mut totals),
        span_total_ms: median(&mut sp_total),
        span_vello_ms: median(&mut sp_vello),
        span_master_compose_ms: median(&mut sp_compose),
        span_dirty_tile_rebuild_ms: median(&mut sp_dirty),
        peak_rss_mb: peak_rss_mb(),
        notes,
    }
}

fn main() {
    let backend_env = std::env::var("L0A_BACKEND").unwrap_or_else(|_| "vulkan".into());
    let backends = match backend_env.as_str() {
        "vulkan" => wgpu::Backends::VULKAN,
        "dx12" => wgpu::Backends::DX12,
        _ => wgpu::Backends::all(),
    };
    let handles = match boot_with(backends) {
        Ok(h) => h,
        Err(e) => {
            eprintln!("boot_with({backend_env}) failed: {e}; falling back to default boot()");
            netrender::boot().expect("wgpu boot")
        }
    };
    let info = handles.adapter.get_info();
    eprintln!(
        "adapter: {} ({:?}, {:?}) driver={} {}",
        info.name, info.device_type, info.backend, info.driver, info.driver_info
    );

    let only = std::env::var("L0A_ONLY").ok();
    let mut cells = Vec::new();
    for mode in ["static", "live", "fragment"] {
        if let Some(o) = &only {
            if o != mode {
                continue;
            }
        }
        for &nrects in RECTS.iter() {
            for &nbodies in BODIES.iter() {
                eprint!("running {mode} bodies={nbodies} rects={nrects} ... ");
                let c = run_cell(&handles, mode, nbodies, nrects);
                eprintln!(
                    "build {:.2} encode {:.2} poll {:.2} frame {:.2} ({} frames)",
                    c.build_ms, c.encode_ms, c.poll_ms, c.frame_ms, c.frames
                );
                cells.push(c);
            }
        }
    }

    let json = serde_json::json!({
        "probe": "L0a netrender face ceiling",
        "date": "2026-09-11",
        "viewport": [W, H],
        "format": "Rgba8Unorm",
        "tile_cache_size": TILE,
        "warmup_frames": WARMUP,
        "frames_target": FRAMES,
        "slow_cell_frames": SLOW_FRAMES,
        "adapter": {
            "name": info.name,
            "backend": format!("{:?}", info.backend),
            "device_type": format!("{:?}", info.device_type),
            "driver": info.driver,
            "driver_info": info.driver_info,
            "vendor": info.vendor,
            "device": info.device,
        },
        "cells": cells.iter().map(|c| serde_json::json!({
            "mode": c.mode,
            "bodies": c.bodies,
            "rects_per_body": c.rects,
            "elements": c.elements,
            "frames_measured": c.frames,
            "build_ms": jf(c.build_ms),
            "encode_ms": jf(c.encode_ms),
            "poll_ms": jf(c.poll_ms),
            "frame_ms": jf(c.frame_ms),
            "span_total_ms": jf(c.span_total_ms),
            "span_vello_render_ms": jf(c.span_vello_ms),
            "span_master_compose_ms": jf(c.span_master_compose_ms),
            "span_dirty_tile_rebuild_ms": jf(c.span_dirty_tile_rebuild_ms),
            "peak_rss_mb": jf(c.peak_rss_mb),
            "notes": c.notes,
        })).collect::<Vec<_>>(),
    });
    let out = std::env::var("L0A_OUT")
        .unwrap_or_else(|_| "l0a_raw.json".into());
    std::fs::write(&out, serde_json::to_string_pretty(&json).unwrap()).expect("write json");
    eprintln!("wrote {out}");
}
