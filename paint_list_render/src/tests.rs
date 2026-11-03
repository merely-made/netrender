// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Translator unit tests.

use crate::composite::merge_layers;

use paint_list_api::{
    BorderDetails, BorderItem, BorderSide, BorderStyle, ColorF, CommonPlacement, DeviceIntSize,
    EngineId, LayoutPoint, LayoutRect, LayoutSideOffsets, NormalBorder, PaintCmd, PaintList,
    PrimitiveFlags, RectItem,
};
use serde::{Deserialize, Serialize};

use super::*;

/// Minimal `PaintList` impl for driving `translate_paint_list`
/// from tests without pulling in a producer crate.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
struct StubPaintList {
    viewport: DeviceIntSize,
    commands: Vec<PaintCmd>,
}

impl PaintList for StubPaintList {
    fn engine_id(&self) -> EngineId {
        EngineId::GENET
    }
    fn viewport(&self) -> DeviceIntSize {
        self.viewport
    }
    fn generation_id(&self) -> u64 {
        0
    }
    fn commands(&self) -> &[PaintCmd] {
        &self.commands
    }
}

fn box2d(x: f32, y: f32, w: f32, h: f32) -> LayoutRect {
    LayoutRect::new(LayoutPoint::new(x, y), LayoutPoint::new(x + w, y + h))
}

fn placement_at(bounds: LayoutRect) -> CommonPlacement {
    CommonPlacement {
        bounds,
        flags: PrimitiveFlags::empty(),
    }
}

fn list_with(viewport: DeviceIntSize, cmds: Vec<PaintCmd>) -> StubPaintList {
    StubPaintList {
        viewport,
        commands: cmds,
    }
}

fn test_border(style: BorderStyle) -> BorderItem {
    BorderItem {
        placement: placement_at(box2d(2.0, 2.0, 24.0, 18.0)),
        widths: LayoutSideOffsets::new(2.0, 2.0, 2.0, 2.0),
        details: BorderDetails::Normal(NormalBorder {
            top: BorderSide {
                color: ColorF::BLACK,
                style,
            },
            right: BorderSide {
                color: ColorF::BLACK,
                style,
            },
            bottom: BorderSide {
                color: ColorF::BLACK,
                style,
            },
            left: BorderSide {
                color: ColorF::BLACK,
                style,
            },
            ..NormalBorder::default()
        }),
    }
}

#[test]
fn unsupported_border_refuses_entire_stream_with_command_index() {
    let list = list_with(
        DeviceIntSize::new(32, 24),
        vec![
            PaintCmd::DrawRect(RectItem {
                placement: placement_at(box2d(0.0, 0.0, 1.0, 1.0)),
                color: ColorF::WHITE,
            }),
            PaintCmd::DrawBorder(test_border(BorderStyle::Double)),
        ],
    );
    let error = translate_paint_list(&list).expect_err("double border must refuse");
    assert_eq!(error.command, 1);
    assert_eq!(error.reason, "border style or geometry is not lowered");

    let fragment_error = translate_paint_cmds_to_fragment(&list.commands, &[], &[])
        .expect_err("retained translation must refuse the same border");
    assert_eq!(fragment_error, error);
}

#[test]
fn mixed_patterned_border_refuses_instead_of_painting_solid_rects() {
    let mut border = test_border(BorderStyle::Dashed);
    let BorderDetails::Normal(sides) = &mut border.details else {
        unreachable!()
    };
    sides.right.style = BorderStyle::Solid;
    let error = translate_paint_cmd_stream(
        DeviceIntSize::new(32, 24),
        &[PaintCmd::DrawBorder(border)],
        &[],
        &[],
    )
    .expect_err("mixed dashed border must refuse");
    assert_eq!(error.command, 0);
    assert_eq!(error.reason, "border style or geometry is not lowered");
}

#[test]
fn nonfinite_normal_border_inputs_refuse_before_emission() {
    let cases: [(&str, fn(&mut BorderItem)); 4] = [
        ("non-finite border bounds", |border| {
            border.placement.bounds.min.x = f32::INFINITY;
        }),
        ("non-finite border width", |border| {
            border.widths.left = f32::NAN;
        }),
        ("non-finite border radius", |border| {
            let BorderDetails::Normal(sides) = &mut border.details else {
                unreachable!()
            };
            sides.radius.top_left.width = f32::NAN;
        }),
        ("non-finite border color", |border| {
            let BorderDetails::Normal(sides) = &mut border.details else {
                unreachable!()
            };
            sides.top.color.a = f32::NAN;
        }),
    ];
    for (reason, mutate) in cases {
        let mut border = test_border(BorderStyle::Solid);
        mutate(&mut border);
        let error = translate_paint_cmd_stream(
            DeviceIntSize::new(32, 24),
            &[PaintCmd::DrawBorder(border)],
            &[],
            &[],
        )
        .expect_err("non-finite border must refuse");
        assert_eq!(error, TranslationError { command: 0, reason });
    }
}

#[test]
fn nine_patch_slice_offsets_stay_within_each_source_axis() {
    use std::sync::Arc;
    use paint_list_api::{
        DeviceIntSideOffsets, IdNamespace, ImageKey, ImageRendering, ImageResource,
        NinePatchBorder, NinePatchSource, RepeatMode,
    };

    let key = ImageKey::new(IdNamespace(1), 20);
    let image = ImageResource {
        key,
        width: 4,
        height: 4,
        data: Arc::new(vec![255; 4 * 4 * 4]),
    };
    let make_border = |slice| {
        PaintCmd::DrawBorder(BorderItem {
            placement: placement_at(box2d(2.0, 2.0, 24.0, 18.0)),
            widths: LayoutSideOffsets::new(2.0, 2.0, 2.0, 2.0),
            details: BorderDetails::NinePatch(NinePatchBorder {
                source: NinePatchSource::Image(key, ImageRendering::Auto),
                width: 4,
                height: 4,
                slice,
                fill: false,
                repeat_horizontal: RepeatMode::Stretch,
                repeat_vertical: RepeatMode::Stretch,
            }),
        })
    };
    let translated = translate_paint_cmd_stream(
        DeviceIntSize::new(32, 24),
        &[make_border(DeviceIntSideOffsets::new(1, 1, 1, 1))],
        &[],
        &[image.clone()],
    )
    .expect("in-source slices admitted");
    assert!(
        translated
            .scene
            .ops
            .iter()
            .any(|op| matches!(op, netrender::SceneOp::Image(_)))
    );

    // The renderer permits overlapping source corners; only individual
    // offsets outside their own axis are rejected.
    translate_paint_cmd_stream(
        DeviceIntSize::new(32, 24),
        &[make_border(DeviceIntSideOffsets::new(3, 3, 3, 3))],
        &[],
        &[image.clone()],
    )
    .expect("overlapping but in-source slices admitted");

    for (slice, reason) in [
        (
            DeviceIntSideOffsets::new(-1, 1, 1, 1),
            "negative nine-patch source slice",
        ),
        (
            DeviceIntSideOffsets::new(1, 1, 1, -1),
            "negative nine-patch source slice",
        ),
        (
            DeviceIntSideOffsets::new(5, 1, 1, 1),
            "nine-patch source slice exceeds its axis extent",
        ),
        (
            DeviceIntSideOffsets::new(1, 5, 1, 1),
            "nine-patch source slice exceeds its axis extent",
        ),
        (
            DeviceIntSideOffsets::new(1, 1, 5, 1),
            "nine-patch source slice exceeds its axis extent",
        ),
        (
            DeviceIntSideOffsets::new(1, 1, 1, 5),
            "nine-patch source slice exceeds its axis extent",
        ),
    ] {
        let error = translate_paint_cmd_stream(
            DeviceIntSize::new(32, 24),
            &[make_border(slice)],
            &[],
            &[image.clone()],
        )
        .expect_err("out-of-source slice must refuse");
        assert_eq!(error, TranslationError { command: 0, reason });
    }
}

#[test]
fn uniform_patterned_border_keeps_stroke_path() {
    for style in [BorderStyle::Dashed, BorderStyle::Dotted] {
        let translated = translate_paint_cmd_stream(
            DeviceIntSize::new(32, 24),
            &[PaintCmd::DrawBorder(test_border(style))],
            &[],
            &[],
        )
        .expect("uniform patterned border admitted");
        let [netrender::SceneOp::Stroke(stroke)] = translated.scene.ops.as_slice() else {
            panic!("patterned border must remain one stroke");
        };
        assert!(!stroke.dash_pattern.is_empty());
    }
}

#[test]
#[cfg(feature = "replay-cpu")]
fn uniform_patterned_border_keeps_painted_segments_and_gaps_on_cpu() {
    for style in [BorderStyle::Dashed, BorderStyle::Dotted] {
        let translated = translate_paint_cmd_stream(
            DeviceIntSize::new(32, 24),
            &[PaintCmd::DrawBorder(test_border(style))],
            &[],
            &[],
        )
        .expect("uniform patterned border admitted");
        let mut cpu = netrender::vello_backends::CpuSession::new(
            netrender::vello_backends::SparseResourceLimits::default(),
        );
        let image = cpu
            .render(&translated.scene)
            .expect("rasterize patterned border");
        let rgba = image.data_as_u8_slice();
        let alpha = |x: usize, y: usize| rgba[(y * 32 + x) * 4 + 3];
        let top: Vec<_> = (5..23).map(|x| alpha(x, 3)).collect();
        assert!(
            top.iter().any(|a| *a > 200),
            "painted {style:?} segment: {top:?}"
        );
        assert!(top.contains(&0), "transparent {style:?} gap: {top:?}");
        assert_eq!(alpha(14, 10), 0, "border center stays transparent");
    }
}

#[test]
fn translucent_colors_premultiply_at_lowering() {
    // ColorF is unpremultiplied; scene colors are premultiplied and every
    // consumer divides by alpha on the way back out. An unpremultiplied
    // copy therefore over-brightened every translucent fill toward white:
    // half-alpha pure red became (1/0.5 -> clamped) full white at half
    // cover. Found by mesocosm's minimap, whose 35%-alpha cells rendered
    // as a white sheet.
    let list = list_with(
        DeviceIntSize::new(64, 64),
        vec![PaintCmd::DrawRect(RectItem {
            placement: placement_at(box2d(0.0, 0.0, 32.0, 32.0)),
            color: ColorF::new(1.0, 0.0, 0.0, 0.5),
        })],
    );
    let scene = translate_paint_list(&list).expect("translate paint list");
    let rect_color = scene
        .ops
        .iter()
        .find_map(|op| match op {
            netrender::SceneOp::Rect(r) => Some(r.color),
            _ => None,
        })
        .expect("one rect");

    assert_eq!(
        rect_color,
        [0.5, 0.0, 0.0, 0.5],
        "premultiplied, alpha intact"
    );
}

#[test]
fn empty_list_translates_to_empty_scene() {
    let list = list_with(DeviceIntSize::new(800, 600), Vec::new());
    let scene = translate_paint_list(&list).expect("translate paint list");
    assert_eq!(scene.viewport_width, 800);
    assert_eq!(scene.viewport_height, 600);
    assert_eq!(scene.ops.len(), 0);
}

#[test]
fn draw_rect_emits_scene_rect() {
    let list = list_with(
        DeviceIntSize::new(800, 600),
        vec![PaintCmd::DrawRect(RectItem {
            placement: placement_at(box2d(10.0, 20.0, 100.0, 50.0)),
            color: ColorF {
                r: 1.0,
                g: 0.0,
                b: 0.0,
                a: 1.0,
            },
        })],
    );
    let scene = translate_paint_list(&list).expect("translate paint list");
    assert_eq!(scene.ops.len(), 1);
    assert!(matches!(scene.ops[0], netrender::SceneOp::Rect(_)));
}

mod resources;

mod layers;
