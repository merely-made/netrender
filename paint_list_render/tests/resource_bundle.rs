// Copyright 2026 Mark Alan Boykin
// SPDX-License-Identifier: MPL-2.0
#![cfg(feature = "resource-bundle")]

use netrender::Scene;
use paint_list_api::*;
use paint_list_render::bundle::*;

fn capture() -> PortablePaintCapture {
    let bounds = LayoutRect::new(LayoutPoint::new(0.0, 0.0), LayoutPoint::new(2.0, 2.0));
    PortablePaintCapture {
        envelope: PaintEnvelope {
            engine: EngineId::GENET,
            viewport: DeviceIntSize::new(2, 2),
            generation: 1,
            commands: vec![PaintCmd::DrawExternalTexture(ExternalTextureItem {
                placement: CommonPlacement::new(bounds),
                texture_key: 7,
                opacity: 0.5,
                content_generation: Some(3),
            })],
            fonts: vec![],
            images: vec![],
        },
        producers: vec![ProducerSource {
            texture_key: 7,
            content_generation: 3,
            source_identity: 1,
            source_revision: 0,
            logical_size: [2.0, 2.0],
            physical_size: [2, 2],
            layout_scale: 1.0,
            alpha: ResourceAlpha::Straight,
            encoding: ResourceEncoding::Srgb,
            scene: Scene::new(2, 2),
        }],
    }
}
fn bundle() -> CompleteResourceBundle {
    let capture = capture();
    let resources = vec![cpu_payload(
        ResourceRef::ExternalTexture(7),
        capture.producers[0].binding().unwrap(),
        [2, 2],
        ResourceAlpha::Straight,
        [64, 32, 0, 128].repeat(4),
    )
    .unwrap()];
    CompleteResourceBundle {
        version: BUNDLE_VERSION,
        capture_digest: capture_digest(&capture).unwrap(),
        translated: translate_capture(&capture).unwrap(),
        capture,
        resources,
    }
}
fn admit(b: &CompleteResourceBundle) -> Result<Scene, BundleError> {
    admit_bundle(b, b.capture_digest, BundleLimits::default())
}
#[test]
fn explicit_generation_and_zero_revision_survive_translation() {
    let c = capture();
    validate_capture(&c).unwrap();
    let t = translate_capture(&c).unwrap();
    assert_eq!(t.external_textures[0].content_generation, Some(3));
    assert_eq!(t.external_textures[0].opacity, 0.5);
    let b = bundle();
    let before = scene_digest(&b.translated.scene).unwrap();
    let result = admit(&b).unwrap();
    assert_eq!(result.image_sources.len(), 1);
    assert_eq!(scene_digest(&b.translated.scene).unwrap(), before);
    assert_eq!(
        result.image_sources[&netrender::external_image_key(7)]
            .data
            .as_ref(),
        [128, 64, 0, 128].repeat(4)
    );
}
#[test]
fn capture_refuses_missing_stale_duplicate_foreign_and_wrong_geometry() {
    let mut c = capture();
    c.producers.clear();
    assert_eq!(validate_capture(&c), Err(BundleError::MissingProducer(7)));
    for stamp in [None, Some(2)] {
        let mut c = capture();
        let PaintCmd::DrawExternalTexture(e) = &mut c.envelope.commands[0] else {
            unreachable!()
        };
        e.content_generation = stamp;
        assert_eq!(validate_capture(&c), Err(BundleError::StaleProducer(7)));
    }
    let mut c = capture();
    c.producers.push(c.producers[0].clone());
    assert_eq!(validate_capture(&c), Err(BundleError::DuplicateProducer(7)));
    let mut c = capture();
    let mut p = c.producers[0].clone();
    p.texture_key = 8;
    c.producers.push(p);
    assert_eq!(validate_capture(&c), Err(BundleError::ForeignProducer(8)));
    let mut c = capture();
    c.producers[0].physical_size = [3, 2];
    assert!(matches!(
        validate_capture(&c),
        Err(BundleError::InvalidProducer { .. })
    ));
}
#[test]
fn ledger_refuses_every_incomplete_or_mismatched_payload_before_materialization() {
    let mut b = bundle();
    b.resources.clear();
    assert!(matches!(admit(&b), Err(BundleError::MissingResource(_))));
    let mut b = bundle();
    b.resources.push(b.resources[0].clone());
    assert!(matches!(admit(&b), Err(BundleError::DuplicateResource(_))));
    let mut b = bundle();
    b.resources[0].resource = ResourceRef::ExternalTexture(8);
    assert!(matches!(admit(&b), Err(BundleError::ForeignResource(_))));
    let mut b = bundle();
    let ResourceBinding::Producer {
        source_revision, ..
    } = &mut b.resources[0].binding
    else {
        unreachable!()
    };
    *source_revision = 1;
    assert!(matches!(admit(&b), Err(BundleError::StaleResource(_))));
    for kind in 0..4 {
        let mut b = bundle();
        match kind {
            0 => b.resources[0].width = 1,
            1 => b.resources[0].rgba.pop().map(|_| ()).unwrap(),
            2 => b.resources[0].alpha = ResourceAlpha::Premultiplied,
            _ => b.resources[0].encoding = ResourceEncoding::LinearSrgb,
        }
        let before = scene_digest(&b.translated.scene).unwrap();
        assert!(matches!(admit(&b), Err(BundleError::InvalidPayload { .. })));
        assert_eq!(scene_digest(&b.translated.scene).unwrap(), before);
        assert!(b.translated.scene.image_sources.is_empty());
    }
}
#[test]
fn identity_and_side_channels_are_separate_from_pixel_acceptance() {
    let mut b = bundle();
    b.translated.scene.push_rect(0.0, 0.0, 1.0, 1.0, [1.0; 4]);
    assert!(matches!(admit(&b), Err(BundleError::TranslationIdentity)));
    let mut b = bundle();
    b.translated.external_textures[0].opacity = 1.0;
    assert!(matches!(admit(&b), Err(BundleError::SideChannelIdentity)));
    let b = bundle();
    assert!(matches!(
        admit_bundle(&b, [0; 32], BundleLimits::default()),
        Err(BundleError::CaptureIdentity)
    ));
}
#[test]
fn alpha_conversion_preserves_transparency_without_matting() {
    let b = bundle();
    let mut p = b.resources[0].clone();
    p.width = 1;
    p.height = 1;
    p.alpha = ResourceAlpha::Premultiplied;
    p.rgba = vec![64, 32, 0, 128];
    assert_eq!(payload_to_straight(&p).unwrap(), [128, 64, 0, 128]);
    p.rgba = vec![0, 0, 0, 0];
    assert_eq!(payload_to_straight(&p).unwrap(), [0, 0, 0, 0]);
    p.rgba = vec![1, 0, 0, 0];
    assert!(matches!(
        payload_to_straight(&p),
        Err(BundleError::InvalidPayload { .. })
    ));
    p.alpha = ResourceAlpha::Straight;
    p.rgba = vec![200, 50, 7, 0];
    assert_eq!(payload_to_straight(&p).unwrap(), [200, 50, 7, 0]);
}
#[test]
fn finite_budget_and_missing_shadow_are_refusals() {
    let b = bundle();
    let mut limits = BundleLimits::default();
    limits.max_bytes = 15;
    assert!(matches!(
        admit_bundle(&b, b.capture_digest, limits),
        Err(BundleError::Budget { .. })
    ));
    let mut c = capture();
    c.producers[0].layout_scale = f32::NAN;
    assert!(matches!(
        validate_capture(&c),
        Err(BundleError::InvalidProducer { .. })
    ));
    let mut c = capture();
    let bounds = LayoutRect::new(LayoutPoint::new(0.0, 0.0), LayoutPoint::new(2.0, 2.0));
    c.envelope.commands.push(PaintCmd::DrawShadow(ShadowItem {
        placement: CommonPlacement::new(bounds),
        box_bounds: bounds,
        offset: LayoutVector2D::new(0.0, 0.0),
        color: ColorF::new(0.0, 0.0, 0.0, 0.5),
        blur_radius: 2.0,
        spread_radius: 0.0,
        border_radius: BorderRadius::default(),
        clip_mode: BoxShadowClipMode::Outset,
    }));
    let translated = translate_capture(&c).unwrap();
    assert!(!translated.box_shadow_masks.is_empty());
    let mut b = bundle();
    b.capture_digest = capture_digest(&c).unwrap();
    b.capture = c;
    b.translated = translated;
    assert!(matches!(
        admit(&b),
        Err(BundleError::MissingResource(ResourceRef::ShadowMask(_)))
    ));
}

#[test]
fn physical_extent_matches_production_f64_multiplication() {
    let mut c = capture();
    c.producers[0].logical_size = [0.1, 0.1];
    c.producers[0].layout_scale = 10.0;
    c.producers[0].scene = Scene::new(1, 1);
    // f32 multiplication rounds this to 1; production promotes the stored
    // f32 inputs to f64 before multiplication and ceil, yielding 2.
    assert_eq!((0.1f32 * 10.0).ceil() as u32, 1);
    assert_eq!((f64::from(0.1f32) * f64::from(10.0f32)).ceil() as u32, 2);
    c.producers[0].physical_size = [2, 2];
    validate_capture(&c).unwrap();
    c.producers[0].physical_size = [1, 1];
    assert!(matches!(
        validate_capture(&c),
        Err(BundleError::InvalidProducer { .. })
    ));
}

#[test]
fn supported_none_values_do_not_mask_nonfinite_floats() {
    let mut c = capture();
    c.envelope
        .commands
        .insert(0, PaintCmd::PushLayer(LayerSpec::default()));
    c.envelope.commands.push(PaintCmd::PopLayer);
    validate_capture(&c).unwrap();
    let PaintCmd::PushLayer(layer) = &mut c.envelope.commands[0] else {
        unreachable!()
    };
    layer.opacity = f32::NAN;
    assert!(matches!(
        validate_capture(&c),
        Err(BundleError::InvalidEnvelope { .. })
    ));
    let mut c = capture();
    let PaintCmd::DrawExternalTexture(item) = &mut c.envelope.commands[0] else {
        unreachable!()
    };
    item.placement.bounds.min.x = f32::NAN;
    assert!(matches!(
        validate_capture(&c),
        Err(BundleError::InvalidEnvelope { .. })
    ));
}
#[test]
fn untranslated_masks_flags_blends_clip_out_and_key_range_refuse() {
    let bounds = LayoutRect::new(LayoutPoint::new(0.0, 0.0), LayoutPoint::new(2.0, 2.0));
    for kind in 0..4 {
        let mut c = capture();
        let mut layer = LayerSpec::default();
        match kind {
            0 => {
                layer.mask = Some(LayerMask {
                    mask_rect: bounds,
                    image_mask: None,
                })
            }
            1 => layer.flags = LayerFlags::BLEND_CONTAINER,
            2 => layer.mix_blend_mode = MixBlendMode::ColorDodge,
            _ => layer.origin.x = 1.0,
        }
        c.envelope.commands.insert(0, PaintCmd::PushLayer(layer));
        c.envelope.commands.push(PaintCmd::PopLayer);
        assert!(matches!(
            validate_capture(&c),
            Err(BundleError::InvalidEnvelope { .. })
        ));
    }
    let mut c = capture();
    c.envelope.commands.insert(
        0,
        PaintCmd::PushClip(ClipSpec {
            kind: ClipKind::RoundedRect {
                rect: bounds,
                radius: BorderRadius::default(),
                clip_out: true,
            },
        }),
    );
    c.envelope.commands.push(PaintCmd::PopClip);
    assert!(matches!(
        validate_capture(&c),
        Err(BundleError::InvalidEnvelope { .. })
    ));
    let mut c = capture();
    c.producers[0].texture_key = 1 << 62;
    assert!(matches!(
        validate_capture(&c),
        Err(BundleError::InvalidProducer { .. })
    ));
}
#[test]
fn aggregate_source_structure_and_nonempty_sentinel_refuse_before_hashing() {
    let c = capture();
    let mut limits = BundleLimits::default();
    limits.max_ops = 1;
    assert!(matches!(
        bounded_source_scene_digest(&c.producers[0].scene, limits),
        Err(BundleError::Budget { .. })
    ));
    let mut scene = Scene::new(2, 2);
    scene.fonts[0].data = netrender::peniko::Blob::from(vec![1, 2, 3]);
    assert!(matches!(
        bounded_source_scene_digest(&scene, BundleLimits::default()),
        Err(BundleError::InvalidScene { .. })
    ));
    let mut scene = Scene::new(2, 2);
    let mut path = netrender::ScenePath::new();
    for _ in 0..4 {
        path.line_to(1.0, 1.0);
    }
    scene.push_shape_filled(path, [1.0; 4]);
    limits.max_ops = 5;
    assert!(matches!(
        bounded_source_scene_digest(&scene, limits),
        Err(BundleError::Budget { .. })
    ));
}
#[test]
fn invisible_borders_are_noops_and_gradient_tile_fallback_refuses() {
    let bounds = LayoutRect::new(LayoutPoint::new(0.0, 0.0), LayoutPoint::new(2.0, 2.0));
    let mut c = capture();
    c.envelope.commands.push(PaintCmd::DrawBorder(BorderItem {
        placement: CommonPlacement::new(bounds),
        widths: LayoutSideOffsets::new(1.0, 0.0, 0.0, 0.0),
        details: BorderDetails::Normal(NormalBorder::default()),
    }));
    validate_capture(&c).expect("invisible border is admitted as a no-op");
    assert_eq!(translate_capture(&c).unwrap().scene.ops.len(), 1);
    let gradient = LinearGradientItem {
        placement: CommonPlacement::new(bounds),
        gradient: LinearGradientPayload {
            start_point: LayoutPoint::new(0.0, 0.0),
            end_point: LayoutPoint::new(2.0, 2.0),
            extend_mode: ExtendMode::Clamp,
            stops: vec![],
        },
        tile_size: LayoutSize::new(2.0, 2.0),
        tile_spacing: LayoutSize::zero(),
    };
    let mut c = capture();
    c.envelope
        .commands
        .push(PaintCmd::DrawLinearGradient(gradient.clone()));
    validate_capture(&c).unwrap();
    let mut gradient = gradient;
    gradient.tile_size.width = 1.0;
    let mut c = capture();
    c.envelope
        .commands
        .push(PaintCmd::DrawLinearGradient(gradient));
    assert!(matches!(
        validate_capture(&c),
        Err(BundleError::InvalidEnvelope { .. })
    ));
}

#[test]
fn a_standard_label_cannot_hide_a_three_dimensional_matrix() {
    let mut c = capture();
    let mut transform = LayoutTransform::identity();
    transform.m43 = 1.0;
    c.envelope.commands.insert(
        0,
        PaintCmd::PushTransform(TransformSpec {
            origin: LayoutPoint::new(0.0, 0.0),
            transform,
            kind: TransformKind::Standard,
        }),
    );
    c.envelope.commands.push(PaintCmd::PopTransform);
    assert!(matches!(
        validate_capture(&c),
        Err(BundleError::InvalidEnvelope { .. })
    ));
    let mut scene = Scene::new(2, 2);
    scene.transforms[0].m[14] = 1.0;
    assert!(matches!(
        validate_source_scene(&scene, BundleLimits::default()),
        Err(BundleError::InvalidScene { .. })
    ));
}

#[test]
fn rounded_element_with_zero_border_widths_is_an_exact_noop() {
    let mut c = capture();
    let bounds = LayoutRect::new(LayoutPoint::new(0.0, 0.0), LayoutPoint::new(2.0, 2.0));
    let mut border = NormalBorder::default();
    border.radius.top_left = LayoutSize::new(1.0, 0.5);
    c.envelope.commands.push(PaintCmd::DrawBorder(BorderItem {
        placement: CommonPlacement::new(bounds),
        widths: LayoutSideOffsets::new(0.0, 0.0, 0.0, 0.0),
        details: BorderDetails::Normal(border),
    }));
    validate_capture(&c).unwrap();
    let scene = translate_capture(&c).unwrap();
    assert_eq!(scene.scene.ops.len(), 1);
}

#[test]
#[cfg(feature = "replay-cpu")]
fn straight_and_premultiplied_payloads_match_with_transparent_overlap() {
    let mut straight = bundle();
    let bounds = LayoutRect::new(LayoutPoint::new(0.0, 0.0), LayoutPoint::new(2.0, 2.0));
    straight.capture.envelope.commands.insert(
        0,
        PaintCmd::DrawRect(RectItem {
            placement: CommonPlacement::new(bounds),
            color: ColorF::new(0.1, 0.7, 0.2, 0.25),
        }),
    );
    straight.capture_digest = capture_digest(&straight.capture).unwrap();
    straight.translated = translate_capture(&straight.capture).unwrap();
    let mut premult = straight.clone();
    premult.capture.producers[0].alpha = ResourceAlpha::Premultiplied;
    premult.resources[0] = cpu_payload(
        ResourceRef::ExternalTexture(7),
        premult.capture.producers[0].binding().unwrap(),
        [2, 2],
        ResourceAlpha::Premultiplied,
        [64, 32, 0, 128].repeat(4),
    )
    .unwrap();
    premult.capture_digest = capture_digest(&premult.capture).unwrap();
    let a = admit(&straight).unwrap();
    let b = admit(&premult).unwrap();
    let mut cpu = netrender::vello_backends::CpuSession::new(
        netrender::vello_backends::SparseResourceLimits::default(),
    );
    let pixels_a = cpu.render(&a).unwrap().data_as_u8_slice().to_vec();
    let pixels_b = cpu.render(&b).unwrap().data_as_u8_slice().to_vec();
    assert_eq!(pixels_a, pixels_b);
    assert!(pixels_a.chunks_exact(4).all(|p| p[3] > 0 && p[3] < 255));
    assert!(pixels_a.chunks_exact(4).any(|p| p[0] > 0 && p[1] > 0));
}

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
