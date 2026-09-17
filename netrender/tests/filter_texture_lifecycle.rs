// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Filter texture lifecycle across frames on a *reused* `Renderer`.
//!
//! The filter passes render each filtered layer's result into a fresh GPU
//! texture every frame and hand it to the rasterizer under a deterministic
//! sentinel `ImageKey`. Cached per-tile `vello::Scene`s bake the vello
//! `ImageData` *identity*, not the key, so the key alone does not tie a reused
//! tile to this frame's texture. Minting a new identity per frame therefore
//! leaves every clean tile pointing at last frame's handle (the animated
//! filtered layer stops updating / goes blank), and never retiring the old
//! identities grows vello's paint-texture table by one per frame.
//!
//! Oracle: a fresh `Renderer` per frame, which has no cross-frame cache state
//! and so cannot exhibit either failure. Byte equality against that is the
//! load-bearing assertion; the override count is the bounded-growth receipt.

use netrender::scene::{SceneClip, SceneFilter, SceneLayer};
use netrender::{boot, create_netrender_instance, ColorLoad, NetrenderOptions, Renderer, Scene};

const DIM: u32 = 256;
const TILE: u32 = 64;

// ── harness ───────────────────────────────────────────────────────────

fn make_target(device: &wgpu::Device) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("filter lifecycle target"),
        size: wgpu::Extent3d {
            width: DIM,
            height: DIM,
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

fn renderer(handles: &netrender::WgpuHandles) -> Renderer {
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

fn render_bytes(r: &Renderer, handles: &netrender::WgpuHandles, scene: &Scene) -> Vec<u8> {
    let (target, view) = make_target(&handles.device);
    r.render_vello(scene, &view, ColorLoad::Clear(wgpu::Color::BLACK));
    r.wgpu_device.read_rgba8_texture(&target, DIM, DIM)
}

/// Pixels that differ, and where the first one is. Byte vectors this size are
/// unreadable in a panic message; this is the comparison the assertions carry.
fn diff_summary(a: &[u8], b: &[u8]) -> Option<String> {
    let differing: Vec<usize> = (0..a.len() / 4)
        .filter(|i| a[i * 4..i * 4 + 4] != b[i * 4..i * 4 + 4])
        .collect();
    let first = *differing.first()?;
    Some(format!(
        "{} of {} pixels differ; first at ({}, {}): got {:?} want {:?}",
        differing.len(),
        a.len() / 4,
        first % DIM as usize,
        first / DIM as usize,
        &a[first * 4..first * 4 + 4],
        &b[first * 4..first * 4 + 4],
    ))
}

/// Receipt capture: set `NETRENDER_FILTER_CAPTURE_DIR` and every comparison
/// this file makes writes `<name>.png` there. Off by default — the test suite
/// asserts, it does not litter.
fn capture(name: &str, bytes: &[u8]) {
    let Ok(dir) = std::env::var("NETRENDER_FILTER_CAPTURE_DIR") else {
        return;
    };
    let dir = std::path::PathBuf::from(dir);
    std::fs::create_dir_all(&dir).expect("capture dir");
    let file = std::fs::File::create(dir.join(format!("{name}.png"))).expect("capture file");
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), DIM, DIM);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header()
        .expect("png header")
        .write_image_data(bytes)
        .expect("png data");
}

// ── content ───────────────────────────────────────────────────────────

fn base_scene() -> Scene {
    let mut s = Scene::new(DIM, DIM);
    s.push_rect(0.0, 0.0, DIM as f32, DIM as f32, [0.06, 0.06, 0.08, 1.0]);
    s
}

/// A three-rect card at `(dx, dy)` — enough internal structure that a stale
/// frame differs from a fresh one in many pixels, not a handful of edges.
fn card(scene: &mut Scene, dx: f32, dy: f32) {
    scene.push_rect(dx, dy, dx + 96.0, dy + 72.0, [0.9, 0.9, 0.95, 1.0]);
    scene.push_rect(
        dx + 8.0,
        dy + 8.0,
        dx + 88.0,
        dy + 28.0,
        [0.2, 0.4, 0.8, 1.0],
    );
    scene.push_rect(
        dx + 8.0,
        dy + 36.0,
        dx + 64.0,
        dy + 64.0,
        [0.85, 0.3, 0.3, 1.0],
    );
}

/// Element CSS `filter` on an unclipped layer: the injected filtered-result
/// image op covers the whole viewport and is byte-identical every frame, so
/// the tile cache sees no scene change at all and reuses every tile. That is
/// the sharpest form of the defect — nothing but the image *identity* can
/// carry the moved content forward.
fn element_filter_scene(dx: f32, dy: f32) -> Scene {
    let mut s = base_scene();
    let mut l = SceneLayer::alpha(1.0);
    l.filters.push(SceneFilter::Invert(1.0));
    s.push_layer(l);
    card(&mut s, dx, dy);
    s.pop_layer();
    s
}

/// `backdrop-filter`: a fixed pane blurs whatever moves behind it. The pane
/// and its injected backdrop image are identical every frame; only the
/// content *behind* moves, so again every tile of the pane is reused.
fn backdrop_filter_scene(dx: f32, dy: f32) -> Scene {
    let mut s = base_scene();
    card(&mut s, dx, dy);
    let mut l = SceneLayer::clip(SceneClip::Rect {
        rect: [48.0, 48.0, 208.0, 208.0],
        radii: [0.0; 4],
    });
    l.backdrop_filter = Some(SceneFilter::Blur(6.0));
    s.push_layer(l);
    s.push_rect(48.0, 48.0, 208.0, 208.0, [1.0, 1.0, 1.0, 0.15]);
    s.pop_layer();
    s
}

const WALK: [(f32, f32); 4] = [(24.0, 20.0), (52.0, 44.0), (88.0, 66.0), (110.0, 90.0)];

/// One reused renderer must match the fresh-renderer oracle on every frame of
/// a walk. Frame 0 passes today; the regression is frame 1 onward.
fn assert_reused_matches_fresh(what: &str, build: fn(f32, f32) -> Scene) {
    let handles = boot().expect("wgpu boot");
    let blank = render_bytes(&renderer(&handles), &handles, &base_scene());

    let r = renderer(&handles);
    let mut previous: Option<Vec<u8>> = None;
    for (i, (dx, dy)) in WALK.iter().enumerate() {
        let scene = build(*dx, *dy);
        let got = render_bytes(&r, &handles, &scene);

        let expected = render_bytes(&renderer(&handles), &handles, &scene);

        // Positive controls: the frame must paint something, and it must
        // differ from the previous frame — otherwise equality is vacuous and
        // a renderer that froze on frame 0 would "pass".
        let slug = what.replace(' ', "_");
        capture(&format!("{slug}_frame{i}_reused"), &got);
        capture(&format!("{slug}_frame{i}_fresh"), &expected);

        assert!(
            diff_summary(&expected, &blank).is_some(),
            "{what}: frame {i} reference must paint the card"
        );
        if let Some(prev) = &previous {
            assert!(
                diff_summary(&expected, prev).is_some(),
                "{what}: frame {i} reference must differ from frame {} — the walk must move pixels",
                i - 1
            );
        }
        if let Some(diff) = diff_summary(&got, &expected) {
            panic!(
                "{what}: frame {i} on a reused renderer must match the fresh-renderer \
                 reference — {diff}"
            );
        }
        previous = Some(expected);
    }
}

#[test]
fn element_filter_reused_renderer_tracks_moving_content() {
    assert_reused_matches_fresh("element filter", element_filter_scene);
}

#[test]
fn backdrop_filter_reused_renderer_tracks_moving_content() {
    assert_reused_matches_fresh("backdrop filter", backdrop_filter_scene);
}

// ── bounded growth ────────────────────────────────────────────────────

/// The filter passes register at most one GPU texture per filtered layer in
/// the frame, for the renderer's whole life — not one per frame. These scenes
/// carry exactly one filtered layer each, so the bound is 1. Before the
/// key/handle reuse scheme this counter equalled the frame number.
fn assert_bounded_overrides(what: &str, build: fn(f32, f32) -> Scene, bound: usize) {
    const FRAMES: usize = 60;
    let handles = boot().expect("wgpu boot");
    let r = renderer(&handles);
    for frame in 0..FRAMES {
        let t = frame as f32;
        render_bytes(&r, &handles, &build(16.0 + t * 1.5, 12.0 + t));
        let live = r
            .vello_live_texture_registrations()
            .expect("vello enabled in this fixture");
        assert!(
            live <= bound,
            "{what}: after frame {frame} the renderer holds {live} registered filter \
             textures, over the bound of {bound}"
        );
    }
}

#[test]
fn element_filter_texture_table_stays_bounded() {
    assert_bounded_overrides("element filter", element_filter_scene, 1);
}

#[test]
fn backdrop_filter_texture_table_stays_bounded() {
    assert_bounded_overrides("backdrop filter", backdrop_filter_scene, 1);
}

/// A filtered layer that goes away must not leave its texture registered:
/// the table shrinks back to zero once nothing in the scene is filtered.
#[test]
fn retiring_the_filter_retires_its_texture() {
    let handles = boot().expect("wgpu boot");
    let r = renderer(&handles);
    render_bytes(&r, &handles, &element_filter_scene(24.0, 20.0));
    assert_eq!(
        r.vello_live_texture_registrations(),
        Some(1),
        "a filtered frame registers exactly one filter texture"
    );

    let mut plain = base_scene();
    card(&mut plain, 24.0, 20.0);
    render_bytes(&r, &handles, &plain);
    assert_eq!(
        r.vello_live_texture_registrations(),
        Some(0),
        "an unfiltered frame must retire the filter texture"
    );
}

// ── static content still reuses tiles ─────────────────────────────────

/// The reason the old policy existed: a clean tile must still resolve its
/// filter image. Handle reuse keeps that true without the leak — an
/// unchanged filtered scene must dirty no tile at all on the second frame.
fn assert_static_reuses_tiles(what: &str, build: fn(f32, f32) -> Scene) {
    let handles = boot().expect("wgpu boot");
    let r = renderer(&handles);
    let scene = build(40.0, 32.0);

    let first = render_bytes(&r, &handles, &scene);
    let dirty_first = r.vello_last_dirty_count().expect("vello enabled");
    assert!(
        dirty_first > 0,
        "{what}: the first frame must dirty tiles, else reuse proves nothing"
    );

    let second = render_bytes(&r, &handles, &scene);
    assert_eq!(
        r.vello_last_dirty_count(),
        Some(0),
        "{what}: an unchanged filtered scene must reuse every tile"
    );
    assert!(
        diff_summary(&first, &second).is_none(),
        "{what}: a reused frame must be byte-identical to the frame it reuses"
    );
}

#[test]
fn static_element_filter_reuses_tiles() {
    assert_static_reuses_tiles("element filter", element_filter_scene);
}

#[test]
fn static_backdrop_filter_reuses_tiles() {
    assert_static_reuses_tiles("backdrop filter", backdrop_filter_scene);
}

/// Names the adapter the byte-equality receipts were taken on. Not an
/// assertion about the host — a line in the log so a receipt can be read
/// back later and attributed.
#[test]
fn report_adapter() {
    let handles = boot().expect("wgpu boot");
    let info = handles.adapter.get_info();
    println!(
        "adapter: {} / {:?} / {} / {}",
        info.name, info.backend, info.driver, info.driver_info
    );
}
