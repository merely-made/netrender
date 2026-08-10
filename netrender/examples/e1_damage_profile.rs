/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! Roadmap E1 evidence — where per-frame cost goes when almost nothing
//! changed.
//!
//! E1 says vello re-encodes the whole scene every frame and defers the
//! fix upstream, with no netrender-side done condition. That entry was
//! written before Phase 7' landed, and it is now imprecise in a way that
//! matters for talking to upstream:
//!
//! - Netrender's tile cache **does** elide re-encode. `dirty_tile_rebuild`
//!   only lowers tiles that `TileCache::invalidate` reported dirty.
//! - What is *not* elided is assembly plus upload. `compose_master`
//!   builds a fresh `vello::Scene` every frame and appends every cached
//!   tile into it, then hands the whole thing to vello. Both are
//!   O(total tiles), not O(dirty tiles).
//!
//! So the question worth measuring is not "does netrender re-encode",
//! it is "how much does a one-tile change cost as the scene grows".
//! This example holds the damage fixed at a single tile and sweeps the
//! total tile count, printing the A4 spans for each.
//!
//! **The answer, measured 2026-08-10, was not the expected one.** Neither
//! vello nor `compose` dominates. `TileCache::invalidate` does, at ~87% of
//! a 4096² frame, because `hash_tile_deps` walks the entire op list once
//! per tile. Dirty detection, not rasterization, is the cost of a
//! mostly-static frame. Roadmap E3 tracks the fix; E1 stays upstream-gated
//! but is no longer the interesting number.
//!
//! ```text
//! cargo run -p netrender --release --example e1_damage_profile
//! ```
//!
//! Release matters. Debug numbers are dominated by unoptimised scene
//! lowering and say nothing useful about upstream.
//!
//! **Reading the numbers honestly.** These are CPU-side wall-clock
//! spans from `std::time::Instant`. `vello_render` covers encode plus
//! submit, not GPU execution; there are no timestamp queries here. The
//! load-bearing column is `master_compose`, which is pure CPU work
//! netrender performs and could avoid if vello scenes were retained
//! across frames.

use std::time::Duration;

use netrender::{boot, create_netrender_instance, ColorLoad, NetrenderOptions, Scene};

const TILE: u32 = 256;
/// Frames measured per configuration, after a warm-up frame.
const FRAMES: usize = 30;

fn make_target(device: &wgpu::Device, dim: u32) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("e1 profile target"),
        size: wgpu::Extent3d {
            width: dim,
            height: dim,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::STORAGE_BINDING
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[wgpu::TextureFormat::Rgba8UnormSrgb],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor {
        format: Some(wgpu::TextureFormat::Rgba8Unorm),
        ..Default::default()
    });
    (texture, view)
}

/// A document-shaped scene: white page, a grid of text-like line boxes
/// covering the whole viewport, plus one small caret rect whose color
/// changes per frame. The caret sits inside a single tile, so exactly
/// one tile should go dirty per frame after the first.
fn page_scene(dim: u32, frame: usize) -> Scene {
    let mut scene = Scene::new(dim, dim);
    scene.push_rect(0.0, 0.0, dim as f32, dim as f32, [1.0, 1.0, 1.0, 1.0]);

    // Line boxes every 24px across the whole page. This is the content
    // that must be re-appended into the master scene every frame even
    // though it never changes.
    let mut y = 16.0;
    while y < dim as f32 - 16.0 {
        let mut x = 16.0;
        while x < dim as f32 - 16.0 {
            let w = if ((x + y) as u32 / 24) % 3 == 2 {
                90.0
            } else {
                140.0
            };
            scene.push_rect(x, y, x + w, y + 11.0, [0.32, 0.32, 0.36, 1.0]);
            x += 160.0;
        }
        y += 24.0;
    }

    // The only thing that changes: a caret in the top-left tile,
    // blinking via color so its bounds stay inside one tile.
    let on = frame % 2 == 0;
    let color = if on {
        [0.1, 0.3, 0.9, 1.0]
    } else {
        [1.0, 1.0, 1.0, 1.0]
    };
    scene.push_rect(40.0, 40.0, 42.0, 56.0, color);

    scene
}

fn median(mut values: Vec<Duration>) -> Duration {
    if values.is_empty() {
        return Duration::ZERO;
    }
    values.sort();
    values[values.len() / 2]
}

fn micros(d: Duration) -> f64 {
    d.as_secs_f64() * 1e6
}

fn main() {
    let handles = boot().expect("wgpu boot");
    let adapter_info = handles.adapter.get_info();
    println!(
        "adapter: {} ({:?}, {:?})",
        adapter_info.name, adapter_info.device_type, adapter_info.backend
    );
    println!(
        "tile size: {TILE}px   measured frames per config: {FRAMES} (median)   \
         profile: {}",
        if cfg!(debug_assertions) {
            "DEBUG — numbers are not meaningful, re-run with --release"
        } else {
            "release"
        }
    );
    println!();
    println!(
        "{:>8}  {:>6}  {:>7}  {:>5}  {:>10}  {:>8}  {:>8}  {:>8}  {:>8}",
        "viewport", "tiles", "ops", "dirty", "invalidate", "rebuild", "compose", "vello", "total"
    );
    println!("{}", "-".repeat(88));

    for dim in [512_u32, 1024, 2048, 4096] {
        let renderer = create_netrender_instance(
            handles.clone(),
            NetrenderOptions {
                tile_cache_size: Some(TILE),
                enable_vello: true,
                ..Default::default()
            },
        )
        .expect("create_netrender_instance");

        let (_target, view) = make_target(&handles.device, dim);

        // Warm-up frame: everything dirty, caches cold. Excluded.
        renderer.render_vello(&page_scene(dim, 0), &view, ColorLoad::Load);

        let mut invalidate = Vec::new();
        let mut rebuild = Vec::new();
        let mut compose = Vec::new();
        let mut vello = Vec::new();
        let mut totals = Vec::new();
        let mut dirty_seen = Vec::new();

        for frame in 1..=FRAMES {
            renderer.render_vello(&page_scene(dim, frame), &view, ColorLoad::Load);

            let t = renderer
                .last_frame_timings()
                .expect("vello path records timings");
            if let Some(d) = t.span("tile_invalidate") {
                invalidate.push(d);
            }
            if let Some(d) = t.span("dirty_tile_rebuild") {
                rebuild.push(d);
            }
            if let Some(d) = t.span("master_compose") {
                compose.push(d);
            }
            if let Some(d) = t.span("vello_render") {
                vello.push(d);
            }
            totals.push(t.total);
            if let Some(d) = renderer.vello_last_dirty_count() {
                dirty_seen.push(d);
            }
        }

        let tiles = (dim / TILE) * (dim / TILE);
        let dirty_typical = {
            let mut d = dirty_seen.clone();
            d.sort_unstable();
            d.get(d.len() / 2).copied().unwrap_or(0)
        };

        println!(
            "{:>8}  {:>6}  {:>7}  {:>5}  {:>10.1}  {:>8.1}  {:>8.1}  {:>8.1}  {:>8.1}",
            format!("{dim}²"),
            tiles,
            page_scene(dim, 0).ops.len(),
            dirty_typical,
            micros(median(invalidate)),
            micros(median(rebuild)),
            micros(median(compose)),
            micros(median(vello)),
            micros(median(totals)),
        );
    }

    println!();
    println!("All figures are microseconds, median of {FRAMES} frames.");
    println!();
    println!("What this measured (2026-08-10, RTX 4060 / Vulkan):");
    println!(
        "  - `rebuild` stays cheap. The tile cache works: one dirty tile is one \
         tile re-lowered,"
    );
    println!("    regardless of page size.");
    println!(
        "  - `vello` barely moves (2x across a 64x tile increase). It is not the \
         bottleneck here."
    );
    println!(
        "  - `invalidate` dominates, and grows with tiles x ops. At 4096 it is \
         ~87% of the frame"
    );
    println!(
        "    for detecting that a single tile changed. That is netrender's own \
         code, not vello's:"
    );
    println!(
        "    `hash_tile_deps` walks the whole op list once per tile, so the cost \
         is O(tiles x ops)"
    );
    println!("    and both grow with page area.");
    println!();
    println!(
        "E1 as written blames vello's whole-scene re-encode. On this evidence that \
         is not the"
    );
    println!(
        "first thing to fix. Bin ops into tiles in one O(ops) pass and hash each \
         tile from its"
    );
    println!("own bin; that is O(ops + tiles). See roadmap E3.");
}
