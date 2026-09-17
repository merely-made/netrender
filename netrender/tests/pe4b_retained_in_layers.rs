// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Roadmap E4, genet T4 — retained placements inside layer scopes.
//!
//! `pe4_fragment_retention.rs` pins the depth-0 path. This file pins the
//! increment: `PushLayer` / `PopLayer` are hoisted onto the master
//! `vello::Scene`, so a placement inside an open layer appends its cached
//! lowering into that layer instead of inlining the fragment's ops.
//!
//! Every case is an *independently expanded* reference: a second scene, in a
//! second `Renderer`, that pushes the same content directly under the same
//! layer and never touches the retained path. Byte equality against that is
//! the load-bearing assertion; the counters only mean something once it holds.
//!
//! Content is opaque rects, deterministic under vello's area AA on one
//! device — the same oracle choice pe4 and the p2 goldens made.

use std::sync::{Mutex, OnceLock};

use netrender::scene::{SceneClip, SceneFilter, SceneFragment, SceneLayer, Transform};
use netrender::{boot, create_netrender_instance, ColorLoad, NetrenderOptions, Renderer, Scene};

const DIM: u32 = 256;
const TILE: u32 = 64;

// ── harness ───────────────────────────────────────────────────────────

fn make_target(device: &wgpu::Device) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("pe4b target"),
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

// ── the shared content ────────────────────────────────────────────────

fn card_fragment() -> SceneFragment {
    let mut f = SceneFragment::new();
    f.push_rect(0.0, 0.0, 96.0, 72.0, [0.9, 0.9, 0.95, 1.0]);
    f.push_rect(8.0, 8.0, 88.0, 28.0, [0.2, 0.4, 0.8, 1.0]);
    f.push_rect(8.0, 36.0, 64.0, 64.0, [0.85, 0.3, 0.3, 1.0]);
    f
}

/// The same content pushed flat at `(dx, dy)` — the independent expansion.
fn card_flat(scene: &mut Scene, dx: f32, dy: f32) {
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

fn base_scene() -> Scene {
    let mut s = Scene::new(DIM, DIM);
    s.push_rect(0.0, 0.0, DIM as f32, DIM as f32, [0.06, 0.06, 0.08, 1.0]);
    s
}

/// The three layer kinds the done-condition names. The clip rect deliberately
/// cuts the card's left and top edges so the clip is load-bearing.
fn rect_clip_layer() -> SceneLayer {
    SceneLayer::clip(SceneClip::Rect {
        rect: [64.0, 48.0, 192.0, 160.0],
        radii: [0.0; 4],
    })
}

fn alpha_layer() -> SceneLayer {
    SceneLayer::alpha(0.45)
}

fn filter_layer() -> SceneLayer {
    let mut l = SceneLayer::alpha(1.0);
    l.filters.push(SceneFilter::Invert(1.0));
    l
}

/// Retained: the card placed inside `layer`.
fn placed_in_layer(id: u64, layer: SceneLayer, dx: f32, dy: f32) -> Scene {
    let mut s = base_scene();
    s.push_layer(layer);
    s.place_fragment(id, Transform::translate_2d(dx, dy));
    s.pop_layer();
    s
}

/// Expanded: the same content inlined inside the same `layer`.
fn expanded_in_layer(layer: SceneLayer, dx: f32, dy: f32) -> Scene {
    let mut s = base_scene();
    s.push_layer(layer);
    card_flat(&mut s, dx, dy);
    s.pop_layer();
    s
}

/// One case: retained-in-layer must equal the independent expansion, and the
/// layer must actually change pixels (otherwise byte equality proves nothing).
fn assert_layer_case(what: &str, layer: SceneLayer, dx: f32, dy: f32) {
    let handles = boot().expect("wgpu boot");

    let frag_r = renderer(&handles);
    let id = frag_r.register_fragment(card_fragment()).expect("register");
    let got = render_bytes(
        &frag_r,
        &handles,
        &placed_in_layer(id, layer.clone(), dx, dy),
    );

    let ref_r = renderer(&handles);
    let expected = render_bytes(&ref_r, &handles, &expanded_in_layer(layer, dx, dy));

    // Positive control: the same content with no layer at all must differ,
    // so an equality that ignored the layer entirely could not pass.
    let bare_r = renderer(&handles);
    let mut bare = base_scene();
    card_flat(&mut bare, dx, dy);
    let bare_bytes = render_bytes(&bare_r, &handles, &bare);
    assert_ne!(
        expected, bare_bytes,
        "{what}: the reference layer must change pixels, else this case is vacuous"
    );

    assert_eq!(
        got, expected,
        "{what}: a placement inside the layer must match the expanded reference"
    );
    assert_eq!(
        frag_r.fragment_lower_count(),
        Some(1),
        "{what}: the placement must retain — exactly one lowering"
    );
}

// ── done-condition 1: three layer kinds, expanded references ──────────

#[test]
fn rect_clip_layer_placement_matches_expanded_reference() {
    assert_layer_case("rect clip", rect_clip_layer(), 40.0, 32.0);
}

#[test]
fn alpha_layer_placement_matches_expanded_reference() {
    assert_layer_case("alpha", alpha_layer(), 40.0, 32.0);
}

#[test]
fn filter_layer_placement_matches_expanded_reference() {
    assert_layer_case("element filter", filter_layer(), 40.0, 32.0);
}

/// Nesting plus painter order across the fragment boundary, both inside the
/// scope: the hoisted push/pop keep a depth counter, and a run must flush
/// before every append or the direct ops would reorder around the placement.
#[test]
fn nested_layers_keep_painter_order_around_a_placement() {
    let handles = boot().expect("wgpu boot");

    let build = |place: &mut dyn FnMut(&mut Scene)| {
        let mut s = base_scene();
        s.push_layer(alpha_layer());
        s.push_layer(rect_clip_layer());
        s.push_rect(30.0, 30.0, 150.0, 150.0, [1.0, 0.85, 0.1, 1.0]); // under
        place(&mut s);
        s.push_rect(90.0, 90.0, 130.0, 130.0, [0.1, 0.1, 0.1, 1.0]); // over
        s.pop_layer();
        s.pop_layer();
        s
    };

    let frag_r = renderer(&handles);
    let id = frag_r.register_fragment(card_fragment()).expect("register");
    let placed =
        build(&mut |s: &mut Scene| s.place_fragment(id, Transform::translate_2d(40.0, 40.0)));
    let got = render_bytes(&frag_r, &handles, &placed);

    let ref_r = renderer(&handles);
    let flat = build(&mut |s: &mut Scene| card_flat(s, 40.0, 40.0));
    let expected = render_bytes(&ref_r, &handles, &flat);

    assert_eq!(
        got, expected,
        "a placement between two direct ops inside nested layers must keep painter order"
    );
    assert_eq!(frag_r.fragment_lower_count(), Some(1));
}

/// Done-condition 1 across a moving placement: every frame of a
/// placement-only walk must match its independent expansion, in all three
/// layer kinds. One long-lived `Renderer` walks the placement, so the whole
/// cross-frame cache lattice is under test; the reference side stays a fresh
/// `Renderer` per frame because it is the independent oracle.
#[test]
fn moving_placements_match_expanded_references() {
    let handles = boot().expect("wgpu boot");
    let blank = render_bytes(&renderer(&handles), &handles, &base_scene());
    let places = [(24.0f32, 20.0f32), (52.0, 44.0), (88.0, 66.0)];

    for (what, layer) in [
        ("rect clip", rect_clip_layer()),
        ("alpha", alpha_layer()),
        ("element filter", filter_layer()),
    ] {
        let r = renderer(&handles);
        let id = r.register_fragment(card_fragment()).expect("register");
        for (i, (dx, dy)) in places.iter().enumerate() {
            let got = render_bytes(&r, &handles, &placed_in_layer(id, layer.clone(), *dx, *dy));

            let ref_r = renderer(&handles);
            let expected = render_bytes(
                &ref_r,
                &handles,
                &expanded_in_layer(layer.clone(), *dx, *dy),
            );

            assert_ne!(
                got, blank,
                "{what}: frame {i} must paint the card, else equality is vacuous"
            );
            assert_eq!(got, expected, "{what}: frame {i} must match its expansion");
            assert_eq!(r.fragment_lower_count(), Some(1), "{what}: frame {i}");
        }
    }
}

// ── done-condition 2: lower count flat across placement-only frames ───

/// The same registered fragment placed at a new transform every frame, on one
/// long-lived `Renderer`, in all three layer kinds. The count must not grow:
/// before the hoist, a layer-scoped placement inlined and the count stayed at
/// zero because nothing was ever retained; now it is one lowering per
/// fragment, for good.
#[test]
fn layer_scoped_placement_lowers_once_across_moving_frames() {
    let handles = boot().expect("wgpu boot");
    let places = [
        (24.0f32, 20.0f32),
        (52.0, 44.0),
        (88.0, 66.0),
        (110.0, 90.0),
    ];

    for (what, layer) in [
        ("rect clip", rect_clip_layer()),
        ("alpha", alpha_layer()),
        ("element filter", filter_layer()),
    ] {
        let r = renderer(&handles);
        let id = r.register_fragment(card_fragment()).expect("register");

        for (i, (dx, dy)) in places.iter().enumerate() {
            render_bytes(&r, &handles, &placed_in_layer(id, layer.clone(), *dx, *dy));
            assert_eq!(
                r.fragment_lower_count(),
                Some(1),
                "{what}: frame {i} — a placement-only change must not re-lower"
            );
        }
    }
}

// ── done-condition 3: the layer-scope warning no longer fires ─────────

struct CaptureLogger;

static WARNINGS: OnceLock<Mutex<Vec<String>>> = OnceLock::new();

fn warnings() -> &'static Mutex<Vec<String>> {
    WARNINGS.get_or_init(|| Mutex::new(Vec::new()))
}

impl log::Log for CaptureLogger {
    fn enabled(&self, m: &log::Metadata) -> bool {
        m.level() <= log::Level::Warn
    }
    fn log(&self, r: &log::Record) {
        if r.level() <= log::Level::Warn {
            warnings()
                .lock()
                .unwrap()
                .push(format!("{}: {}", r.target(), r.args()));
        }
    }
    fn flush(&self) {}
}

/// The planar fixture — a clip layer per body, placements only — must emit no
/// fragment-path warning at all: not the retired layer-scope fallback, not an
/// unregistered id, not `scene_to_vello`'s no-registry arm. Records are shared
/// across this binary's tests, which only widens what the assertion covers.
#[test]
fn planar_clip_fixture_emits_no_fragment_warning() {
    let _ = log::set_logger(&CaptureLogger);
    log::set_max_level(log::LevelFilter::Warn);

    let handles = boot().expect("wgpu boot");
    let r = renderer(&handles);
    let ids: Vec<u64> = (0..6)
        .map(|_| r.register_fragment(card_fragment()).expect("register"))
        .collect();

    for frame in 0..4 {
        let mut s = base_scene();
        for (i, id) in ids.iter().enumerate() {
            let dx = 8.0 + (i % 3) as f32 * 72.0 + frame as f32 * 3.0;
            let dy = 8.0 + (i / 3) as f32 * 108.0 + frame as f32 * 2.0;
            // Each body inside its own content-box clip, the genet shape.
            s.push_layer(SceneLayer::clip(SceneClip::Rect {
                rect: [dx, dy, dx + 96.0, dy + 72.0],
                radii: [0.0; 4],
            }));
            s.place_fragment(*id, Transform::translate_2d(dx, dy));
            s.pop_layer();
        }
        render_bytes(&r, &handles, &s);
    }

    assert_eq!(
        r.fragment_lower_count(),
        Some(ids.len() as u64),
        "one lowering per registered fragment across four placement-only frames"
    );

    let captured = warnings().lock().unwrap().clone();
    let offenders: Vec<&String> = captured
        .iter()
        .filter(|m| m.to_lowercase().contains("fragment"))
        .collect();
    assert!(
        offenders.is_empty(),
        "the fragment path must not warn on the planar clip fixture: {offenders:?}"
    );
}
