// Copyright 2026 Mark Alan Boykin
// SPDX-License-Identifier: MPL-2.0
#![cfg(feature = "resource-bundle")]

use super::*;

fn joined_border_capture(radius: LayoutSize) -> PortablePaintCapture {
    let bounds = LayoutRect::new(LayoutPoint::new(0.0, 0.0), LayoutPoint::new(24.0, 24.0));
    let side = BorderSide {
        style: BorderStyle::Solid,
        color: ColorF::new(0.25, 0.5, 0.75, 1.0),
    };
    let border = NormalBorder {
        top: side,
        right: side,
        bottom: side,
        left: BorderSide {
            style: BorderStyle::Solid,
            color: ColorF::new(1.0, 0.0, 0.0, 0.5),
        },
        radius: BorderRadius {
            top_left: radius,
            top_right: radius,
            bottom_left: radius,
            bottom_right: radius,
        },
        do_aa: true,
    };
    PortablePaintCapture {
        envelope: PaintEnvelope {
            engine: EngineId::GENET,
            viewport: DeviceIntSize::new(24, 24),
            generation: 1,
            commands: vec![PaintCmd::DrawBorder(BorderItem {
                placement: CommonPlacement::new(bounds),
                widths: LayoutSideOffsets::new(2.0, 2.0, 2.0, 6.0),
                details: BorderDetails::Normal(border),
            })],
            fonts: vec![],
            images: vec![],
        },
        producers: vec![],
    }
}

#[test]
fn border_antialiasing_request_cannot_be_silently_changed() {
    let mut c = joined_border_capture(LayoutSize::new(5.0, 3.0));
    validate_capture(&c).unwrap();
    let PaintCmd::DrawBorder(item) = &mut c.envelope.commands[0] else {
        panic!("border missing")
    };
    let BorderDetails::Normal(border) = &mut item.details else {
        panic!("normal border missing")
    };
    border.do_aa = false;
    assert!(matches!(
        validate_capture(&c),
        Err(BundleError::InvalidEnvelope {
            command: Some(0),
            ..
        })
    ));
}

#[test]
fn portable_capture_uses_the_normal_border_style_admission() {
    let mut c = joined_border_capture(LayoutSize::zero());
    let PaintCmd::DrawBorder(item) = &mut c.envelope.commands[0] else {
        unreachable!()
    };
    let BorderDetails::Normal(border) = &mut item.details else {
        unreachable!()
    };
    border.left.style = BorderStyle::Double;
    assert!(matches!(
        validate_capture(&c),
        Err(BundleError::InvalidEnvelope {
            command: Some(0),
            reason: "border style or geometry is not lowered"
        })
    ));
}

#[test]
fn nonuniform_rounded_solid_border_keeps_joined_shapes_and_ellipse_axes() {
    let c = joined_border_capture(LayoutSize::new(5.0, 3.0));
    validate_capture(&c).unwrap();
    let translated = translate_capture(&c).unwrap();
    assert_eq!(translated.scene.ops.len(), 14);
    let netrender::SceneOp::PushLayer(layer) = &translated.scene.ops[0] else {
        panic!("ring clip missing")
    };
    let netrender::SceneClip::Path(ring) = &layer.clip else {
        panic!("rounded ring must be a path")
    };
    assert_eq!(
        ring.ops
            .iter()
            .filter(|op| matches!(op, netrender::PathOp::CubicTo(..)))
            .count(),
        8
    );
    assert!(matches!(
        ring.ops[2],
        netrender::PathOp::CubicTo(_, _, _, _, 24.0, 3.0)
    ));
    let shapes: Vec<_> = translated
        .scene
        .ops
        .iter()
        .filter_map(|op| {
            if let netrender::SceneOp::Shape(s) = op {
                Some(s)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(shapes.len(), 4);
    assert_eq!(shapes[3].fill_color, Some([0.5, 0.0, 0.0, 0.5]));
    assert!(matches!(
        shapes[3].path.ops[2],
        netrender::PathOp::LineTo(6.0, 2.0)
    ));
    let normalized =
        translate_capture(&joined_border_capture(LayoutSize::new(50.0, 50.0))).unwrap();
    let netrender::SceneOp::PushLayer(layer) = &normalized.scene.ops[0] else {
        unreachable!()
    };
    let netrender::SceneClip::Path(ring) = &layer.clip else {
        unreachable!()
    };
    assert!(matches!(ring.ops[0], netrender::PathOp::MoveTo(12.0, 0.0)));
}

#[test]
#[cfg(feature = "replay-cpu")]
fn joined_rounded_border_preserves_transparent_hole_corners_and_side_alpha() {
    let c = joined_border_capture(LayoutSize::new(5.0, 5.0));
    let bundle = CompleteResourceBundle {
        version: BUNDLE_VERSION,
        capture_digest: capture_digest(&c).unwrap(),
        translated: translate_capture(&c).unwrap(),
        capture: c,
        resources: vec![],
    };
    let scene = admit(&bundle).unwrap();
    let mut cpu = netrender::vello_backends::CpuSession::new(
        netrender::vello_backends::SparseResourceLimits::default(),
    );
    let image = cpu.render(&scene).unwrap();
    let pixels = image.data_as_u8_slice();
    let at = |x: usize, y: usize| &pixels[(y * 24 + x) * 4..(y * 24 + x + 1) * 4];
    assert_eq!(at(0, 0), [0, 0, 0, 0]);
    assert_eq!(at(12, 12), [0, 0, 0, 0]);
    // Inside the rounded ring but inside the rectangular inner bounding box:
    // this corner crescent was absent when side polygons stopped at that box.
    assert!(
        at(22, 1)[3] > 20,
        "rounded inner-corner crescent was left unpainted"
    );
    let left = at(2, 12);
    assert!(left[0].abs_diff(128) <= 1 && left[3].abs_diff(128) <= 1);
    assert_eq!(&left[1..3], [0, 0]);
    let top = at(12, 0);
    assert!(top[0].abs_diff(64) <= 1 && top[1].abs_diff(128) <= 1 && top[2].abs_diff(191) <= 1);
    assert_eq!(top[3], 255);
}

#[test]
fn shadow_geometry_cannot_silently_drop_radius_or_transformed_blur() {
    let bounds = LayoutRect::new(LayoutPoint::new(0.0, 0.0), LayoutPoint::new(2.0, 2.0));
    let base = ShadowItem {
        placement: CommonPlacement::new(bounds),
        box_bounds: bounds,
        offset: LayoutVector2D::new(0.0, 0.0),
        color: ColorF::new(0.0, 0.0, 0.0, 0.5),
        blur_radius: 1.0,
        spread_radius: 0.0,
        border_radius: BorderRadius::default(),
        clip_mode: BoxShadowClipMode::Outset,
    };
    let mut c = capture();
    let mut shadow = base.clone();
    shadow.border_radius.top_left = LayoutSize::new(1.0, 1.0);
    c.envelope.commands.push(PaintCmd::DrawShadow(shadow));
    assert!(matches!(
        validate_capture(&c),
        Err(BundleError::InvalidEnvelope { .. })
    ));
    for (xy, shear) in [(2.0, 0.0), (1.0, 0.2)] {
        let mut c = capture();
        let mut transform = LayoutTransform::identity();
        transform.m11 = xy;
        transform.m12 = shear;
        c.envelope
            .commands
            .push(PaintCmd::PushTransform(TransformSpec {
                origin: LayoutPoint::new(0.0, 0.0),
                transform,
                kind: TransformKind::Standard,
            }));
        c.envelope.commands.push(PaintCmd::DrawShadow(base.clone()));
        c.envelope.commands.push(PaintCmd::PopTransform);
        assert!(matches!(
            validate_capture(&c),
            Err(BundleError::InvalidEnvelope { .. })
        ));
    }
}

#[test]
fn a_borrowed_source_plan_checks_aggregate_budget_before_digests() {
    let a = Scene::new(2, 2);
    let b = Scene::new(2, 2);
    let mut limits = BundleLimits::default();
    limits.max_ops = 3;
    validate_source_scene(&a, limits).unwrap();
    validate_source_scene(&b, limits).unwrap();
    assert!(matches!(
        validate_source_scenes([&a, &b], limits),
        Err(BundleError::Budget { .. })
    ));
    let mut limits = BundleLimits::default();
    limits.max_sources = 1;
    assert!(matches!(
        validate_source_scenes([&a, &b], limits),
        Err(BundleError::Budget { .. })
    ));
}

#[test]
fn large_finite_radius_normalization_does_not_overflow_to_square_corners() {
    let c = joined_border_capture(LayoutSize::new(f32::MAX, f32::MAX));
    validate_capture(&c).unwrap();
    let t = translate_capture(&c).unwrap();
    let netrender::SceneOp::PushLayer(layer) = &t.scene.ops[0] else {
        unreachable!()
    };
    let netrender::SceneClip::Path(path) = &layer.clip else {
        unreachable!()
    };
    assert!(matches!(path.ops[0], netrender::PathOp::MoveTo(12.0, 0.0)));
}

#[test]
#[cfg(feature = "replay-cpu")]
fn additive_side_coverage_keeps_uniform_ring_and_mixed_alpha_seams() {
    let mut c = joined_border_capture(LayoutSize::zero());
    let PaintCmd::DrawBorder(border) = &mut c.envelope.commands[0] else {
        unreachable!()
    };
    border.widths = LayoutSideOffsets::new(8.0, 8.0, 8.0, 8.0);
    let mixed = translate_capture(&c).unwrap().scene;
    let mut cpu = netrender::vello_backends::CpuSession::new(
        netrender::vello_backends::SparseResourceLimits::default(),
    );
    let pixels = cpu.render(&mixed).unwrap().data_as_u8_slice().to_vec();
    let seam = &pixels[(4 * 24 + 4) * 4..(4 * 24 + 5) * 4];
    for (actual, expected) in seam.iter().zip([96u8, 64, 96, 192]) {
        assert!(
            actual.abs_diff(expected) <= 2,
            "additive translucent/opaque seam {seam:?}"
        );
    }
    let PaintCmd::DrawBorder(border) = &mut c.envelope.commands[0] else {
        unreachable!()
    };
    let BorderDetails::Normal(sides) = &mut border.details else {
        unreachable!()
    };
    sides.left = sides.top;
    let uniform = translate_capture(&c).unwrap().scene;
    assert_eq!(
        uniform.ops.len(),
        1,
        "identical active side colors fill one ring"
    );
    let netrender::SceneOp::PushLayer(layer) = &mixed.ops[0] else {
        unreachable!()
    };
    let netrender::SceneClip::Path(ring) = &layer.clip else {
        unreachable!()
    };
    let mut reference = Scene::new(24, 24);
    reference.push_shape_filled(ring.clone(), [0.25, 0.5, 0.75, 1.0]);
    let actual = cpu.render(&uniform).unwrap().data_as_u8_slice().to_vec();
    let expected = cpu.render(&reference).unwrap().data_as_u8_slice().to_vec();
    assert_eq!(actual, expected);
    assert_eq!(
        actual[(4 * 24 + 4) * 4 + 3],
        255,
        "opaque diagonal seam lost alpha"
    );
}
