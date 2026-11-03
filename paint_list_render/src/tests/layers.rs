// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

use super::*;

/// `DrawStroke` (the orrery's edge primitive — a straight or routed polyline)
/// lowers to one stroked `SceneShape`, so edges render. (Was warn-skipped.)
#[test]
fn draw_stroke_emits_stroked_scene_shape() {
    use paint_list_api::{LayoutPoint, PathCommand, PathData, StrokeCap, StrokeItem, StrokeJoin};

    let list = list_with(
        DeviceIntSize::new(800, 600),
        vec![PaintCmd::DrawStroke(StrokeItem {
            placement: placement_at(box2d(0.0, 0.0, 100.0, 100.0)),
            path: PathData {
                commands: vec![
                    PathCommand::MoveTo(LayoutPoint::new(0.0, 0.0)),
                    PathCommand::LineTo(LayoutPoint::new(100.0, 80.0)),
                ],
            },
            color: ColorF {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 1.0,
            },
            width: 2.0,
            cap: StrokeCap::Butt,
            join: StrokeJoin::Miter,
            dash: None,
        })],
    );
    let scene = translate_paint_list(&list).expect("translate paint list");
    let stroked = scene
        .ops
        .iter()
        .filter(|o| matches!(o, netrender::SceneOp::Shape(s) if s.stroke.is_some()))
        .count();
    assert_eq!(stroked, 1, "DrawStroke lowers to one stroked SceneShape");
}

/// A `DrawStroke` with round cap / join and a dash pattern carries those
/// decorations onto the lowered `ScenePathStroke` (not just color + width), so a
/// dashed, round-capped widget stroke renders instead of a solid butt one.
#[test]
fn draw_stroke_carries_cap_join_dash() {
    use paint_list_api::{
        DashPattern, LayoutPoint, PathCommand, PathData, StrokeCap, StrokeItem, StrokeJoin,
    };

    let list = list_with(
        DeviceIntSize::new(800, 600),
        vec![PaintCmd::DrawStroke(StrokeItem {
            placement: placement_at(box2d(0.0, 0.0, 100.0, 100.0)),
            path: PathData {
                commands: vec![
                    PathCommand::MoveTo(LayoutPoint::new(0.0, 0.0)),
                    PathCommand::LineTo(LayoutPoint::new(100.0, 80.0)),
                ],
            },
            color: ColorF {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 1.0,
            },
            width: 2.0,
            cap: StrokeCap::Round,
            join: StrokeJoin::Round,
            dash: Some(DashPattern {
                intervals: vec![5.0, 3.0],
                offset: 1.0,
            }),
        })],
    );
    let scene = translate_paint_list(&list).expect("translate paint list");
    let shape = scene
        .ops
        .iter()
        .find_map(|o| match o {
            netrender::SceneOp::Shape(s) if s.stroke.is_some() => Some(s),
            _ => None,
        })
        .expect("stroked shape");
    let stroke = shape.stroke.as_ref().unwrap();
    assert_eq!(stroke.cap, netrender::SceneStrokeCap::Round);
    assert_eq!(stroke.join, netrender::SceneStrokeJoin::Round);
    assert_eq!(stroke.dash_pattern, vec![5.0, 3.0]);
    assert_eq!(stroke.dash_offset, 1.0);
}

#[test]
fn push_pop_layer_emits_layer_pair() {
    let list = list_with(
        DeviceIntSize::new(800, 600),
        vec![
            PaintCmd::PushLayer(ple::LayerSpec {
                opacity: 0.5,
                ..ple::LayerSpec::default()
            }),
            PaintCmd::PopLayer,
        ],
    );
    let scene = translate_paint_list(&list).expect("translate paint list");
    assert_eq!(scene.ops.len(), 2);
    assert!(matches!(scene.ops[0], netrender::SceneOp::PushLayer(_)));
    assert!(matches!(scene.ops[1], netrender::SceneOp::PopLayer));
}

#[test]
fn push_layer_carries_filter_chain() {
    // CSS `filter` lands on `SceneLayer::filters` (the layer's own output
    // chain); `opacity()` in the chain folds into the layer alpha instead.
    let list = list_with(
        DeviceIntSize::new(800, 600),
        vec![
            PaintCmd::PushLayer(ple::LayerSpec {
                filters: vec![
                    ple::FilterOp::Grayscale(1.0),
                    ple::FilterOp::Opacity(0.5),
                    ple::FilterOp::Blur(2.0),
                ],
                ..ple::LayerSpec::default()
            }),
            PaintCmd::PopLayer,
        ],
    );
    let scene = translate_paint_list(&list).expect("translate paint list");
    let layer = match &scene.ops[0] {
        netrender::SceneOp::PushLayer(l) => l,
        other => panic!("expected PushLayer, got {other:?}"),
    };
    assert_eq!(
        layer.filters,
        vec![
            netrender::SceneFilter::Grayscale(1.0),
            netrender::SceneFilter::Blur(2.0)
        ],
        "color/blur ops map to SceneLayer.filters in order"
    );
    assert_eq!(layer.alpha, 0.5, "opacity() folds into the layer alpha");
}

#[test]
fn push_transform_adds_palette_entry_and_positions_child_ops() {
    // PushTransform is a coordinate-space change, NOT a compositing
    // layer: it adds a transform palette entry and threads the id
    // onto child ops, but emits no PushLayer/PopLayer.
    let list = list_with(
        DeviceIntSize::new(800, 600),
        vec![
            PaintCmd::PushTransform(ple::TransformSpec {
                origin: LayoutPoint::new(10.0, 20.0),
                transform: paint_list_api::LayoutTransform::identity(),
                kind: ple::TransformKind::Standard,
            }),
            PaintCmd::DrawRect(RectItem {
                placement: placement_at(box2d(0.0, 0.0, 100.0, 50.0)),
                color: ColorF {
                    r: 1.0,
                    g: 0.0,
                    b: 0.0,
                    a: 1.0,
                },
            }),
            PaintCmd::PopTransform,
        ],
    );
    let scene = translate_paint_list(&list).expect("translate paint list");
    // One transform palette entry beyond identity (index 1).
    assert!(
        scene.transforms.len() >= 2,
        "transforms: {:?}",
        scene.transforms
    );
    // No layers — just the rect.
    assert_eq!(scene.ops.len(), 1);
    let rect = match &scene.ops[0] {
        netrender::SceneOp::Rect(r) => r,
        other => panic!("expected Rect, got {other:?}"),
    };
    // The rect picks up the pushed transform (non-zero id).
    assert!(
        rect.transform_id > 0,
        "rect should carry the pushed transform id"
    );
    // That transform translates by the origin (10, 20).
    let t = &scene.transforms[rect.transform_id as usize];
    assert_eq!(t.m[12], 10.0, "tx");
    assert_eq!(t.m[13], 20.0, "ty");
}

#[test]
fn nested_transforms_compose() {
    // Outer translate(10, 20), inner translate(5, 5) → a rect
    // inside both should resolve to translate(15, 25).
    let list = list_with(
        DeviceIntSize::new(800, 600),
        vec![
            PaintCmd::PushTransform(ple::TransformSpec {
                origin: LayoutPoint::new(10.0, 20.0),
                transform: paint_list_api::LayoutTransform::identity(),
                kind: ple::TransformKind::Standard,
            }),
            PaintCmd::PushTransform(ple::TransformSpec {
                origin: LayoutPoint::new(5.0, 5.0),
                transform: paint_list_api::LayoutTransform::identity(),
                kind: ple::TransformKind::Standard,
            }),
            PaintCmd::DrawRect(RectItem {
                placement: placement_at(box2d(0.0, 0.0, 10.0, 10.0)),
                color: ColorF {
                    r: 0.0,
                    g: 1.0,
                    b: 0.0,
                    a: 1.0,
                },
            }),
            PaintCmd::PopTransform,
            PaintCmd::PopTransform,
        ],
    );
    let scene = translate_paint_list(&list).expect("translate paint list");
    let rect = match &scene.ops[0] {
        netrender::SceneOp::Rect(r) => r,
        other => panic!("expected Rect, got {other:?}"),
    };
    let t = &scene.transforms[rect.transform_id as usize];
    assert_eq!(t.m[12], 15.0, "composed tx (10 + 5)");
    assert_eq!(t.m[13], 25.0, "composed ty (20 + 5)");
}

#[test]
fn push_clip_rect_emits_clipped_layer() {
    let list = list_with(
        DeviceIntSize::new(800, 600),
        vec![
            PaintCmd::PushClip(ple::ClipSpec {
                kind: ple::ClipKind::Rect(box2d(0.0, 0.0, 100.0, 100.0)),
            }),
            PaintCmd::PopClip,
        ],
    );
    let scene = translate_paint_list(&list).expect("translate paint list");
    assert_eq!(scene.ops.len(), 2);
    let layer = match &scene.ops[0] {
        netrender::SceneOp::PushLayer(l) => l,
        other => panic!("expected PushLayer, got {other:?}"),
    };
    assert!(matches!(layer.clip, netrender::SceneClip::Rect { .. }));
    assert!(matches!(scene.ops[1], netrender::SceneOp::PopLayer));
}

#[test]
fn external_texture_stages_an_in_order_scene_image() {
    use paint_list_api::ExternalTextureItem;
    let list = list_with(
        DeviceIntSize::new(800, 600),
        vec![PaintCmd::DrawExternalTexture(ExternalTextureItem {
            placement: placement_at(box2d(0.0, 0.0, 200.0, 200.0)),
            texture_key: 0xC0FFEE,
            opacity: 0.75,
            content_generation: None,
        })],
    );
    let out = translate_paint_cmd_stream(list.viewport, &list.commands, &[], &[])
        .expect("translate external texture");
    let image = match &out.scene.ops[..] {
        [netrender::SceneOp::Image(image)] => image,
        other => panic!("expected one in-order external image, got {other:?}"),
    };
    assert_eq!(out.external_textures.len(), 1);
    assert_eq!(out.external_textures[0].texture_key, 0xC0FFEE);
    assert_eq!(out.external_textures[0].scene_op_boundary, 0);
    assert_eq!(image.key, out.external_textures[0].image_key);
    assert_eq!(image.color, [0.75; 4]);
}

#[test]
fn external_texture_keeps_the_active_transform_and_layer_scope() {
    use paint_list_api::{ExternalTextureItem, LayerSpec, TransformKind, TransformSpec};

    let list = list_with(
        DeviceIntSize::new(160, 120),
        vec![
            PaintCmd::PushTransform(TransformSpec {
                origin: LayoutPoint::new(0.0, 0.0),
                transform: paint_list_api::LayoutTransform::translation(12.0, 8.0, 0.0),
                kind: TransformKind::Standard,
            }),
            PaintCmd::PushClip(paint_list_api::ClipSpec {
                kind: paint_list_api::ClipKind::Rect(box2d(0.0, 0.0, 64.0, 48.0)),
            }),
            PaintCmd::PushLayer(LayerSpec {
                opacity: 0.5,
                ..LayerSpec::default()
            }),
            PaintCmd::DrawExternalTexture(ExternalTextureItem {
                placement: placement_at(box2d(4.0, 6.0, 20.0, 10.0)),
                texture_key: 77,
                opacity: 1.0,
                content_generation: None,
            }),
            PaintCmd::PopLayer,
            PaintCmd::PopClip,
            PaintCmd::PopTransform,
        ],
    );

    let out = translate_paint_cmd_stream(list.viewport, &list.commands, &[], &[])
        .expect("translate external texture");
    let image = out
        .scene
        .ops
        .iter()
        .find_map(|op| match op {
            netrender::SceneOp::Image(image) => Some(image),
            _ => None,
        })
        .expect("external image");
    assert_ne!(image.transform_id, 0, "canvas inherits its CTM");
    assert_eq!(image.color, [1.0; 4], "layer owns element opacity");
    assert!(matches!(out.scene.ops[0], netrender::SceneOp::PushLayer(_)));
    assert!(matches!(out.scene.ops[1], netrender::SceneOp::PushLayer(_)));
    assert_eq!(out.external_textures[0].scene_op_boundary, 2);
}
