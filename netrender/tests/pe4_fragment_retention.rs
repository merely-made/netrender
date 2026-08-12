/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! Roadmap E4 (spike) — retained-fragment receipts.
//!
//! The load-bearing assertion is pixel identity: a scene that places a
//! retained fragment must render byte-identically to the flat scene
//! with the same ops pushed directly, at every placement. Everything
//! else (retention counters, master reuse) is only meaningful once
//! that holds, because a cache that changes pixels is not a cache.
//!
//! Content is rects and strokes only — deterministic under vello's
//! area AA on a single device, which is what makes byte-equality a
//! usable oracle (the same choice the pa/p2 golden suites made).

use netrender::scene::{SceneFragment, Transform};
use netrender::{boot, create_netrender_instance, ColorLoad, NetrenderOptions, Renderer, Scene};

const DIM: u32 = 256;
const TILE: u32 = 64;

fn make_target(device: &wgpu::Device) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("pe4 target"),
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

/// The shared content: a card with a border and two inner rects, in
/// fragment-local coordinates (origin at 0,0).
fn card_fragment() -> SceneFragment {
    let mut f = SceneFragment::new();
    f.push_rect(0.0, 0.0, 96.0, 72.0, [0.9, 0.9, 0.95, 1.0]);
    f.push_rect(8.0, 8.0, 88.0, 28.0, [0.2, 0.4, 0.8, 1.0]);
    f.push_rect(8.0, 36.0, 64.0, 64.0, [0.85, 0.3, 0.3, 1.0]);
    f
}

/// The same content pushed flat into a scene at `(dx, dy)`.
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

#[test]
fn fragment_scene_matches_flat_scene_pixel_for_pixel() {
    let handles = boot().expect("wgpu boot");

    // Flat reference.
    let flat_r = renderer(&handles);
    let mut flat = base_scene();
    card_flat(&mut flat, 40.0, 32.0);
    let flat_bytes = render_bytes(&flat_r, &handles, &flat);

    // Fragment version: same content, placed via translate.
    let frag_r = renderer(&handles);
    let id = frag_r.register_fragment(card_fragment()).expect("register");
    let mut placed = base_scene();
    placed.place_fragment(id, Transform::translate_2d(40.0, 32.0));
    let frag_bytes = render_bytes(&frag_r, &handles, &placed);

    assert_eq!(
        flat_bytes, frag_bytes,
        "a placed fragment must render byte-identically to the flat equivalent"
    );
}

#[test]
fn placement_change_moves_pixels_without_relowering() {
    let handles = boot().expect("wgpu boot");
    let r = renderer(&handles);
    let id = r.register_fragment(card_fragment()).expect("register");

    // Frame 1 at (16, 16); frame 2 at (96, 120). Each must match its
    // own flat reference, and the fragment must lower exactly once.
    for (i, (dx, dy)) in [(16.0f32, 16.0f32), (96.0, 120.0)].iter().enumerate() {
        let mut placed = base_scene();
        placed.place_fragment(id, Transform::translate_2d(*dx, *dy));
        let got = render_bytes(&r, &handles, &placed);

        let flat_r = renderer(&handles);
        let mut flat = base_scene();
        card_flat(&mut flat, *dx, *dy);
        let expected = render_bytes(&flat_r, &handles, &flat);

        assert_eq!(got, expected, "placement {i} must match its flat reference");
    }

    assert_eq!(
        r.fragment_lower_count(),
        Some(1),
        "two placements of unchanged content must lower once — retention is the point"
    );
}

#[test]
fn unchanged_frame_reuses_the_cached_master() {
    let handles = boot().expect("wgpu boot");
    let r = renderer(&handles);
    let id = r.register_fragment(card_fragment()).expect("register");

    let build = || {
        let mut s = base_scene();
        s.place_fragment(id, Transform::translate_2d(50.0, 50.0));
        s
    };

    let first = render_bytes(&r, &handles, &build());
    assert_eq!(r.fragment_master_hits(), Some(0), "first frame builds");
    let second = render_bytes(&r, &handles, &build());
    assert_eq!(
        r.fragment_master_hits(),
        Some(1),
        "an identical frame must reuse the cached master"
    );
    assert_eq!(first, second, "master reuse must not change pixels");
}

#[test]
fn content_update_relowers_and_changes_pixels() {
    let handles = boot().expect("wgpu boot");
    let r = renderer(&handles);
    let id = r.register_fragment(card_fragment()).expect("register");

    let place = |scene: &mut Scene| scene.place_fragment(id, Transform::translate_2d(60.0, 60.0));

    let mut s1 = base_scene();
    place(&mut s1);
    let before = render_bytes(&r, &handles, &s1);

    // New content under the same id: recolor the header bar.
    let mut updated = SceneFragment::new();
    updated.push_rect(0.0, 0.0, 96.0, 72.0, [0.9, 0.9, 0.95, 1.0]);
    updated.push_rect(8.0, 8.0, 88.0, 28.0, [0.1, 0.7, 0.3, 1.0]);
    updated.push_rect(8.0, 36.0, 64.0, 64.0, [0.85, 0.3, 0.3, 1.0]);
    assert_eq!(r.update_fragment(id, updated), Some(true));

    let mut s2 = base_scene();
    place(&mut s2);
    let after = render_bytes(&r, &handles, &s2);

    assert_ne!(before, after, "a content update must change the output");
    assert_eq!(
        r.fragment_lower_count(),
        Some(2),
        "the update must re-lower exactly once"
    );

    // And the updated output still matches its flat reference.
    let flat_r = renderer(&handles);
    let mut flat = base_scene();
    flat.push_rect(60.0, 60.0, 156.0, 132.0, [0.9, 0.9, 0.95, 1.0]);
    flat.push_rect(68.0, 68.0, 148.0, 88.0, [0.1, 0.7, 0.3, 1.0]);
    flat.push_rect(68.0, 96.0, 124.0, 124.0, [0.85, 0.3, 0.3, 1.0]);
    let expected = render_bytes(&flat_r, &handles, &flat);
    assert_eq!(
        after, expected,
        "updated content must match its flat reference"
    );
}

/// Painter order across the fragment boundary: direct op below,
/// fragment, direct op above — all overlapping.
#[test]
fn fragment_interleaves_with_direct_ops_in_painter_order() {
    let handles = boot().expect("wgpu boot");

    let frag_r = renderer(&handles);
    let id = frag_r.register_fragment(card_fragment()).expect("register");
    let mut placed = base_scene();
    placed.push_rect(30.0, 30.0, 150.0, 150.0, [1.0, 0.85, 0.1, 1.0]); // under
    placed.place_fragment(id, Transform::translate_2d(40.0, 40.0));
    placed.push_rect(90.0, 90.0, 130.0, 130.0, [0.1, 0.1, 0.1, 0.8]); // over
    let got = render_bytes(&frag_r, &handles, &placed);

    let flat_r = renderer(&handles);
    let mut flat = base_scene();
    flat.push_rect(30.0, 30.0, 150.0, 150.0, [1.0, 0.85, 0.1, 1.0]);
    card_flat(&mut flat, 40.0, 40.0);
    flat.push_rect(90.0, 90.0, 130.0, 130.0, [0.1, 0.1, 0.1, 0.8]);
    let expected = render_bytes(&flat_r, &handles, &flat);

    assert_eq!(
        got, expected,
        "a fragment between two direct ops must keep painter order"
    );
}

/// A placement under a non-identity fragment-local transform: content
/// that itself uses `push_rect_transformed` inside the fragment, then
/// gets placed. Placement must compose on top of the local transform.
#[test]
fn placement_composes_with_fragment_local_transforms() {
    let handles = boot().expect("wgpu boot");

    use netrender::scene::{SceneOp, SceneRect, NO_CLIP, SHARP_CLIP};

    let frag_r = renderer(&handles);
    let mut f = SceneFragment::new();
    let local = f.push_transform(Transform::translate_2d(10.0, 6.0));
    f.push_rect(0.0, 0.0, 40.0, 40.0, [0.3, 0.8, 0.5, 1.0]);
    f.push_op(SceneOp::Rect(SceneRect {
        x0: 0.0,
        y0: 0.0,
        x1: 40.0,
        y1: 40.0,
        color: [0.9, 0.4, 0.1, 0.9],
        transform_id: local,
        clip_rect: NO_CLIP,
        clip_corner_radii: SHARP_CLIP,
    }));
    let id = frag_r.register_fragment(f).expect("register");
    let mut placed = base_scene();
    placed.place_fragment(id, Transform::translate_2d(100.0, 80.0));
    let got = render_bytes(&frag_r, &handles, &placed);

    let flat_r = renderer(&handles);
    let mut flat = base_scene();
    flat.push_rect(100.0, 80.0, 140.0, 120.0, [0.3, 0.8, 0.5, 1.0]);
    flat.push_rect(110.0, 86.0, 150.0, 126.0, [0.9, 0.4, 0.1, 0.9]);
    let expected = render_bytes(&flat_r, &handles, &flat);

    assert_eq!(
        got, expected,
        "placement must compose with the fragment's own local transforms"
    );
}

/// The layer fallback: a fragment placed inside a PushLayer scope
/// inlines un-retained but must still be pixel-correct, including the
/// layer's alpha.
#[test]
fn layer_scoped_placement_falls_back_correctly() {
    use netrender::scene::{SceneClip, SceneLayer, SceneOp};

    let handles = boot().expect("wgpu boot");

    let frag_r = renderer(&handles);
    let id = frag_r.register_fragment(card_fragment()).expect("register");
    let mut placed = base_scene();
    let mut layer = SceneLayer::clip(SceneClip::Rect {
        rect: [0.0, 0.0, DIM as f32, DIM as f32],
        radii: [0.0; 4],
    });
    layer.alpha = 0.5;
    placed.push_layer(layer.clone());
    placed.place_fragment(id, Transform::translate_2d(70.0, 70.0));
    placed.ops.push(SceneOp::PopLayer);
    let got = render_bytes(&frag_r, &handles, &placed);

    let flat_r = renderer(&handles);
    let mut flat = base_scene();
    flat.push_layer(layer);
    card_flat(&mut flat, 70.0, 70.0);
    flat.ops.push(SceneOp::PopLayer);
    let expected = render_bytes(&flat_r, &handles, &flat);

    assert_eq!(
        got, expected,
        "a layer-scoped placement must fall back to a pixel-correct inline"
    );
    assert_eq!(
        frag_r.fragment_lower_count(),
        Some(0),
        "the layer fallback inlines; it must not populate the retained cache"
    );
}
