// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

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
//! vello nor `compose` dominated. `TileCache::invalidate` did, at ~87% of
//! a 4096² frame, because dirty detection rescanned the whole op list once
//! per tile. Rasterization was not the cost of a mostly-static frame;
//! deciding what to rasterize was.
//!
//! That became roadmap E3, now cleared: `invalidate` dropped 8.2× at
//! 4096² and its per-op cost is flat, so it tracks op count rather than
//! tiles × ops. See verification record §11.35. E1 itself stays open and
//! upstream-gated, but it was never the interesting number here.
//!
//! This example is kept as the before/after instrument. Re-run it after
//! any change to the tile cache or the master-compose path.
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
//! submit, not GPU execution; there are no timestamp queries here, and
//! it varies by a few hundred microseconds run to run on this machine,
//! so treat small movements in that column as noise. `invalidate`,
//! `rebuild` and `compose` are pure CPU work netrender does and are
//! stable enough to compare across runs.

use std::time::Duration;

use netrender::scene::Transform;
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

/// The same page under a camera pan: identical content every frame, but
/// everything sits beneath a translate whose offset moves 7px per frame
/// (sub-tile, so most tiles still see mostly the same primitives).
///
/// This is the "graph canvas pan" / scroll shape. The per-op digest
/// folds the world AABB in (the anti-ghosting rule), so a placement
/// change is indistinguishable from a content change: every tile's hash
/// moves, every tile goes dirty, and the whole scene is re-lowered even
/// though nothing about it changed but the camera.
fn panned_scene(dim: u32, frame: usize) -> Scene {
    let mut scene = Scene::new(dim, dim);
    let cam = scene.push_transform(Transform::translate_2d(frame as f32 * 7.0, 0.0));

    scene.push_rect_transformed(0.0, 0.0, dim as f32, dim as f32, [1.0, 1.0, 1.0, 1.0], cam);
    let mut y = 16.0;
    while y < dim as f32 - 16.0 {
        let mut x = 16.0;
        while x < dim as f32 - 16.0 {
            let w = if ((x + y) as u32 / 24) % 3 == 2 {
                90.0
            } else {
                140.0
            };
            scene.push_rect_transformed(x, y, x + w, y + 11.0, [0.32, 0.32, 0.36, 1.0], cam);
            x += 160.0;
        }
        y += 24.0;
    }
    scene.push_rect_transformed(40.0, 40.0, 42.0, 56.0, [0.1, 0.3, 0.9, 1.0], cam);

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

fn run_table(handles: &netrender::WgpuHandles, scene_fn: &dyn Fn(u32, usize) -> Scene) {
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
        renderer.render_vello(&scene_fn(dim, 0), &view, ColorLoad::Load);

        let mut invalidate = Vec::new();
        let mut rebuild = Vec::new();
        let mut compose = Vec::new();
        let mut vello = Vec::new();
        let mut totals = Vec::new();
        let mut dirty_seen = Vec::new();

        for frame in 1..=FRAMES {
            renderer.render_vello(&scene_fn(dim, frame), &view, ColorLoad::Load);

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
            scene_fn(dim, 0).ops.len(),
            dirty_typical,
            micros(median(invalidate)),
            micros(median(rebuild)),
            micros(median(compose)),
            micros(median(vello)),
            micros(median(totals)),
        );
    }
}

/// The E4 rematch: identical content and pan to `panned_scene`, but the
/// page lives in ONE retained fragment placed per frame, with the caret
/// as a direct op that keeps blinking (so the master genuinely rebuilds
/// every frame — this is the honest case, not the everything-cached
/// one). The fragment must lower once per config; the pan itself is an
/// append.
fn run_fragment_pan_table(handles: &netrender::WgpuHandles) {
    use netrender::scene::{SceneFragment, Transform};

    println!(
        "{:>8}  {:>6}  {:>7}  {:>5}  {:>10}  {:>8}  {:>8}  {:>8}  {:>8}",
        "viewport", "tiles", "ops", "lowr", "signature", "rebuild", "compose", "vello", "total"
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

        // Register the page content once, in fragment-local space.
        let mut fragment = SceneFragment::new();
        fragment.push_rect(0.0, 0.0, dim as f32, dim as f32, [1.0, 1.0, 1.0, 1.0]);
        let mut y = 16.0;
        while y < dim as f32 - 16.0 {
            let mut x = 16.0;
            while x < dim as f32 - 16.0 {
                let w = if ((x + y) as u32 / 24) % 3 == 2 {
                    90.0
                } else {
                    140.0
                };
                fragment.push_rect(x, y, x + w, y + 11.0, [0.32, 0.32, 0.36, 1.0]);
                x += 160.0;
            }
            y += 24.0;
        }
        let ops = fragment.ops.len() + 2; // + caret + placement
        let id = renderer.register_fragment(fragment).expect("register");

        let scene_for = |frame: usize| {
            let mut scene = Scene::new(dim, dim);
            scene.place_fragment(id, Transform::translate_2d(frame as f32 * 7.0, 0.0));
            let on = frame % 2 == 0;
            let color = if on {
                [0.1, 0.3, 0.9, 1.0]
            } else {
                [1.0, 1.0, 1.0, 1.0]
            };
            scene.push_rect(40.0, 40.0, 42.0, 56.0, color);
            scene
        };

        let (_target, view) = make_target(&handles.device, dim);
        renderer.render_vello(&scene_for(0), &view, ColorLoad::Load);

        let mut signature = Vec::new();
        let mut rebuild = Vec::new();
        let mut compose = Vec::new();
        let mut vello = Vec::new();
        let mut totals = Vec::new();

        for frame in 1..=FRAMES {
            renderer.render_vello(&scene_for(frame), &view, ColorLoad::Load);
            let t = renderer
                .last_frame_timings()
                .expect("vello path records timings");
            if let Some(d) = t.span("fragment_signature") {
                signature.push(d);
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
        }

        let tiles = (dim / TILE) * (dim / TILE);
        println!(
            "{:>8}  {:>6}  {:>7}  {:>5}  {:>10.1}  {:>8.1}  {:>8.1}  {:>8.1}  {:>8.1}",
            format!("{dim}²"),
            tiles,
            ops,
            renderer.fragment_lower_count().unwrap_or(0),
            micros(median(signature)),
            micros(median(rebuild)),
            micros(median(compose)),
            micros(median(vello)),
            micros(median(totals)),
        );
    }
    println!();
    println!(
        "`lowr` is the total fragment lower count after {FRAMES} frames: 1 means the \
         pan never re-lowered."
    );
    println!("`rebuild` has no tile work on this path and should read as 0.");
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
    println!("== static page, one-tile damage (caret blink) ==");
    run_table(&handles, &page_scene);
    println!();
    println!("== same page under a 7px/frame camera pan ==");
    run_table(&handles, &panned_scene);
    println!();
    println!("== same pan, page retained as one E4 fragment (caret stays a direct op) ==");
    run_fragment_pan_table(&handles);

    println!();
    println!("All figures are microseconds, median of {FRAMES} frames.");
    println!();
    println!("Baseline before roadmap E3 (2026-08-10, same machine), `invalidate` column:");
    println!("  512:  6.3    1024: 40.5    2048: 295.2    4096: 3428.1");
    println!("It was ~87% of the 4096 frame, because dirty detection rescanned the whole");
    println!("op list once per tile: O(tiles x ops), both factors growing with page area.");
    println!();
    println!("E3 binned ops to tiles in one pass. Divide the current `invalidate`");
    println!("figure by its op count: the per-op cost should be flat across all four");
    println!("rows, which is what O(ops) looks like. It was ~95 ns/op on the run that");
    println!("cleared E3, against a 55x spread in tiles x ops.");
    println!();
    println!("`vello` varies run to run by a few hundred microseconds on this machine, so");
    println!("read it as noise unless it moves by more than that. `invalidate` is stable.");
}
