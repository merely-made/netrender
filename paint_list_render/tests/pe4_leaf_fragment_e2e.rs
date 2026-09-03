// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Roadmap E4 — the sprigging-shaped consumer flow, end to end.
//!
//! Sprigging's retention model: a `Leaf` repaints only when
//! `paint_dirty`, `RenderedLeaves` caches each leaf's `PaintCmd` splice
//! with an epoch, and the host splices cached commands into each
//! frame's envelope. This test drives the fragment version of that
//! flow against the envelope version and asserts they produce the same
//! pixels:
//!
//!   envelope path: full command list → `translate_paint_list` → render
//!   fragment path: leaf splice → `translate_paint_cmds_to_fragment`
//!                  → `register_fragment` → `place_fragment` at the
//!                  leaf's box → render
//!
//! Plus the retention receipt: moving the leaf's box (layout change,
//! placement-only) must not re-translate or re-lower the leaf.
//!
//! Content is rects, strokes, and a filled path — the sprigging Path-A
//! vocabulary minus text (fonts would make byte-equality depend on a
//! system font; the glyph path has its own receipts in netrender).

use paint_list_api::items::{PathCommand, PathData, PathItem, RectItem, StrokeItem};
use paint_list_api::specs::{ClipKind, ClipSpec};
use paint_list_api::{
    ColorF, CommonPlacement, DeviceIntSize, EngineId, LayoutPoint, LayoutRect, PaintCmd,
    PaintEnvelope,
};
use paint_list_render::{translate_paint_cmds_to_fragment, translate_paint_list};

use netrender::scene::Transform;
use netrender::{boot, create_netrender_instance, ColorLoad, NetrenderOptions, Renderer, Scene};

const DIM: u32 = 256;

fn rect(x0: f32, y0: f32, x1: f32, y1: f32) -> LayoutRect {
    LayoutRect::new(LayoutPoint::new(x0, y0), LayoutPoint::new(x1, y1))
}

fn fill(x0: f32, y0: f32, x1: f32, y1: f32, color: ColorF) -> PaintCmd {
    PaintCmd::DrawRect(RectItem {
        placement: CommonPlacement::new(rect(x0, y0, x1, y1)),
        color,
    })
}

/// A meter-ish leaf in LEAF-LOCAL coordinates: a track, a level fill, a
/// tick path, and a clipped highlight. What a sprigging `paint()` would
/// push through its `PaintCx`.
fn leaf_splice() -> Vec<PaintCmd> {
    vec![
        fill(0.0, 0.0, 96.0, 24.0, ColorF::new(0.15, 0.15, 0.18, 1.0)),
        fill(2.0, 2.0, 70.0, 22.0, ColorF::new(0.2, 0.75, 0.4, 1.0)),
        PaintCmd::PushClip(ClipSpec {
            kind: ClipKind::Rect(rect(2.0, 2.0, 70.0, 12.0)),
        }),
        fill(2.0, 2.0, 70.0, 22.0, ColorF::new(1.0, 1.0, 1.0, 0.25)),
        PaintCmd::PopClip,
        PaintCmd::DrawPath(PathItem {
            placement: CommonPlacement::new(rect(76.0, 4.0, 92.0, 20.0)),
            path: PathData {
                commands: vec![
                    PathCommand::MoveTo(LayoutPoint::new(76.0, 20.0)),
                    PathCommand::LineTo(LayoutPoint::new(84.0, 4.0)),
                    PathCommand::LineTo(LayoutPoint::new(92.0, 20.0)),
                    PathCommand::Close,
                ],
            },
            fill: Some(ColorF::new(0.9, 0.8, 0.2, 1.0)),
            stroke: None,
        }),
        PaintCmd::DrawStroke(StrokeItem {
            placement: CommonPlacement::new(rect(0.0, 0.0, 96.0, 24.0)),
            path: PathData {
                commands: vec![
                    PathCommand::MoveTo(LayoutPoint::new(0.5, 0.5)),
                    PathCommand::LineTo(LayoutPoint::new(95.5, 0.5)),
                    PathCommand::LineTo(LayoutPoint::new(95.5, 23.5)),
                    PathCommand::LineTo(LayoutPoint::new(0.5, 23.5)),
                    PathCommand::Close,
                ],
            },
            color: ColorF::new(0.6, 0.6, 0.7, 1.0),
            width: 1.0,
            cap: Default::default(),
            join: Default::default(),
            dash: None,
        }),
    ]
}

/// The same splice re-based to `(dx, dy)`: what the host's envelope
/// path effectively does when it splices leaf commands at the leaf's
/// layout box. Built by wrapping in a PushTransform the translator
/// composes, exactly as genet's splice path does.
fn envelope_with_leaf_at(dx: f32, dy: f32) -> PaintEnvelope {
    use paint_list_api::specs::{TransformKind, TransformSpec};
    use paint_list_api::LayoutTransform;

    let mut commands = vec![fill(
        0.0,
        0.0,
        DIM as f32,
        DIM as f32,
        ColorF::new(0.05, 0.05, 0.08, 1.0),
    )];
    commands.push(PaintCmd::PushTransform(TransformSpec {
        origin: LayoutPoint::new(0.0, 0.0),
        transform: LayoutTransform::new(
            1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, dx, dy, 0.0, 1.0,
        ),
        kind: TransformKind::Standard,
    }));
    commands.extend(leaf_splice());
    commands.push(PaintCmd::PopTransform);

    PaintEnvelope {
        engine: EngineId::GENET,
        viewport: DeviceIntSize::new(DIM as i32, DIM as i32),
        generation: 1,
        commands,
        fonts: Vec::new(),
        images: Vec::new(),
    }
}

fn make_target(device: &wgpu::Device) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("pe4 e2e target"),
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
            tile_cache_size: Some(64),
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

/// Fragment-path scene: backdrop as a direct op, leaf placed at its box.
fn fragment_scene(id: u64, dx: f32, dy: f32) -> Scene {
    let mut scene = Scene::new(DIM, DIM);
    scene.push_rect(0.0, 0.0, DIM as f32, DIM as f32, [0.05, 0.05, 0.08, 1.0]);
    scene.place_fragment(id, Transform::translate_2d(dx, dy));
    scene
}

#[test]
fn leaf_fragment_flow_matches_envelope_flow_and_retains_across_layout_moves() {
    let handles = boot().expect("wgpu boot");

    // Fragment path renderer: translate the leaf splice ONCE.
    let frag_r = renderer(&handles);
    let fragment = translate_paint_cmds_to_fragment(&leaf_splice(), &[], &[]);
    let id = frag_r.register_fragment(fragment).expect("register");

    // Two layout positions for the leaf's box: initial, then moved
    // (a relayout that only moved the leaf — sprigging's placement-only
    // case).
    for (dx, dy) in [(24.0f32, 40.0f32), (120.0, 168.0)] {
        // Envelope path: full list, fresh renderer (its own tile state),
        // translated flat.
        let env_r = renderer(&handles);
        let flat_scene = translate_paint_list(&envelope_with_leaf_at(dx, dy));
        let expected = render_bytes(&env_r, &handles, &flat_scene);

        let got = render_bytes(&frag_r, &handles, &fragment_scene(id, dx, dy));

        assert_eq!(
            got, expected,
            "leaf at ({dx},{dy}): fragment path must match the envelope path pixel-for-pixel"
        );
    }

    assert_eq!(
        frag_r.fragment_lower_count(),
        Some(1),
        "moving the leaf's layout box must not re-lower the leaf — this is the \
         retention sprigging's epoch model buys through the fragment seam"
    );
}

/// The in-stream marker path: `PaintCmd::PlaceRetainedFragment` inside
/// the envelope's command stream, including under an active transform
/// (the genet-layout shape: the emitter walks in box-local space, so
/// the marker's origin composes with whatever transform is live).
/// This is the wiring genet's `emit_paint_list_with_leaves` drives.
#[test]
fn place_retained_fragment_marker_composes_with_the_active_transform() {
    use paint_list_api::specs::{TransformKind, TransformSpec};
    use paint_list_api::{LayoutTransform, RetainedFragmentRef};

    let handles = boot().expect("wgpu boot");

    let frag_r = renderer(&handles);
    let fragment = translate_paint_cmds_to_fragment(&leaf_splice(), &[], &[]);
    let id = frag_r.register_fragment(fragment).expect("register");

    // Marker envelope: backdrop, then a PushTransform (the enclosing
    // box's space), then the marker at a content offset inside it.
    let mut commands = vec![fill(
        0.0,
        0.0,
        DIM as f32,
        DIM as f32,
        ColorF::new(0.05, 0.05, 0.08, 1.0),
    )];
    commands.push(PaintCmd::PushTransform(TransformSpec {
        origin: LayoutPoint::new(48.0, 64.0),
        transform: LayoutTransform::identity(),
        kind: TransformKind::Standard,
    }));
    commands.push(PaintCmd::PlaceRetainedFragment(RetainedFragmentRef {
        id,
        origin: LayoutPoint::new(6.0, 10.0),
    }));
    commands.push(PaintCmd::PopTransform);
    let envelope = PaintEnvelope {
        engine: EngineId::GENET,
        viewport: DeviceIntSize::new(DIM as i32, DIM as i32),
        generation: 1,
        commands,
        fonts: Vec::new(),
        images: Vec::new(),
    };
    let marker_scene = translate_paint_list(&envelope);
    let got = render_bytes(&frag_r, &handles, &marker_scene);

    // Flat reference: the leaf inlined at the composed position.
    let env_r = renderer(&handles);
    let flat = translate_paint_list(&envelope_with_leaf_at(54.0, 74.0));
    let expected = render_bytes(&env_r, &handles, &flat);

    assert_eq!(
        got, expected,
        "the marker must compose PushTransform(48,64) with origin (6,10)"
    );
    assert_eq!(
        frag_r.fragment_lower_count(),
        Some(1),
        "the marker path must reuse the registered fragment's lowering"
    );
}

/// Epoch bump: the leaf repainted (paint_dirty), the host re-translates
/// its splice and updates the fragment. Pixels must track the new
/// content and exactly one extra lowering must happen.
#[test]
fn leaf_epoch_bump_updates_content() {
    let handles = boot().expect("wgpu boot");
    let frag_r = renderer(&handles);
    let fragment = translate_paint_cmds_to_fragment(&leaf_splice(), &[], &[]);
    let id = frag_r.register_fragment(fragment).expect("register");

    let _ = render_bytes(&frag_r, &handles, &fragment_scene(id, 30.0, 30.0));

    // The "repaint": level fill drops to 40%.
    let mut repainted = leaf_splice();
    repainted[1] = fill(2.0, 2.0, 40.0, 22.0, ColorF::new(0.85, 0.35, 0.2, 1.0));
    let new_fragment = translate_paint_cmds_to_fragment(&repainted, &[], &[]);
    assert_eq!(frag_r.update_fragment(id, new_fragment), Some(true));

    let got = render_bytes(&frag_r, &handles, &fragment_scene(id, 30.0, 30.0));

    // Envelope reference for the repainted state. The leaf splice
    // starts at commands[2] (after the backdrop and the PushTransform),
    // so splice index 1 (the level fill) is commands[3].
    let env_r = renderer(&handles);
    let mut env = envelope_with_leaf_at(30.0, 30.0);
    env.commands[3] = fill(2.0, 2.0, 40.0, 22.0, ColorF::new(0.85, 0.35, 0.2, 1.0));
    let expected = render_bytes(&env_r, &handles, &translate_paint_list(&env));

    assert_eq!(
        got, expected,
        "post-repaint fragment must match the envelope path"
    );
    assert_eq!(
        frag_r.fragment_lower_count(),
        Some(2),
        "one epoch bump, one extra lowering"
    );
}
