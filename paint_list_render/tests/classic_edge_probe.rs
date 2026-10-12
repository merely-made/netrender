// Copyright 2026 Mark Alan Boykin
// SPDX-License-Identifier: MPL-2.0
//! CPU/source controls for the diagnostic fixture protocol. Never boots GPU.
#![cfg(feature = "resource-bundle")]
#[path = "../examples/classic_edge_probe/fixtures.rs"]
mod fixtures;
use netrender::{PathOp, SceneClip, SceneOp};
use paint_list_render::bundle::{scene_digest, validate_source_scene, BundleLimits};

#[test]
fn diagnostic_polyline_shares_exact_backend_input_and_bounded_geometry() {
    use netrender::vello_backends::{validate_scene_for_backend, VelloBackend};
    let cases = fixtures::geometry_cases();
    let fill = cases.iter().find(|c| c.name == "polyline-fill").unwrap();
    let clip = cases
        .iter()
        .find(|c| c.name == "polyline-clip-only")
        .unwrap();
    let SceneOp::Shape(shape) = &fill.scene.ops[0] else {
        panic!("polyline fill missing")
    };
    let SceneOp::PushLayer(layer) = &clip.scene.ops[0] else {
        panic!("polyline clip missing")
    };
    let SceneClip::Path(path) = &layer.clip else {
        panic!("shared polyline clip missing")
    };
    assert_eq!(
        shape.path.ops, path.ops,
        "fill and clip must share exact f32 endpoints"
    );
    assert_eq!(
        path.ops
            .iter()
            .filter(|op| matches!(op, PathOp::LineTo(..)))
            .count(),
        fixtures::POLYLINE_TOTAL_SEGMENTS
    );
    assert!(!path
        .ops
        .iter()
        .any(|op| matches!(op, PathOp::QuadTo(..) | PathOp::CubicTo(..))));
    assert_eq!(shape.transform_id, 0);
    assert_eq!(layer.transform_id, 0);
    for case in [fill, clip] {
        let expected = scene_digest(&case.scene).unwrap();
        for backend in [
            VelloBackend::Classic,
            VelloBackend::Cpu,
            VelloBackend::Hybrid,
        ] {
            let backend_input = case.scene.clone();
            validate_scene_for_backend(backend, &backend_input).unwrap();
            assert_eq!(
                scene_digest(&backend_input).unwrap(),
                expected,
                "backend-specific preparation must not change the shared Scene"
            );
        }
    }
    let mut current = [0.0f64; 2];
    for op in fixtures::outer_curve().ops {
        match op {
            PathOp::MoveTo(x, y) | PathOp::LineTo(x, y) => current = [f64::from(x), f64::from(y)],
            PathOp::CubicTo(a, b, c, d, x, y) => {
                let points = [
                    current,
                    [f64::from(a), f64::from(b)],
                    [f64::from(c), f64::from(d)],
                    [f64::from(x), f64::from(y)],
                ];
                for i in 0..2 {
                    let dx = 6.0 * (points[i][0] - 2.0 * points[i + 1][0] + points[i + 2][0]);
                    let dy = 6.0 * (points[i][1] - 2.0 * points[i + 1][1] + points[i + 2][1]);
                    assert!(dx.hypot(dy) <= fixtures::POLYLINE_SECOND_DERIVATIVE_BOUND);
                }
                current = points[3];
            }
            PathOp::Close => {}
            PathOp::QuadTo(..) => panic!("unexpected canonical quadratic"),
        }
    }
    let n = fixtures::POLYLINE_SEGMENTS_PER_QUARTER as f64;
    let curve_error = fixtures::POLYLINE_SECOND_DERIVATIVE_BOUND / (8.0 * n * n);
    let rounding_error = 2.0f64.sqrt() / 1048576.0;
    assert!(curve_error < 1.0 / 512.0);
    assert!(curve_error + rounding_error < 1.0 / 256.0);
}

#[test]
fn isolated_curve_is_the_exact_outer_contour_of_the_actual_border() {
    let cases = fixtures::geometry_cases();
    let border = cases
        .iter()
        .find(|c| c.name == "border-ellipse-accent-plus")
        .unwrap();
    let SceneOp::PushLayer(layer) = &border.scene.ops[0] else {
        panic!("missing border ring")
    };
    let SceneClip::Path(ring) = &layer.clip else {
        panic!("ring clip lost")
    };
    let outer = fixtures::outer_curve();
    assert_eq!(&ring.ops[..outer.ops.len()], outer.ops.as_slice());
    assert!(
        ring.ops.len() > outer.ops.len(),
        "border must still retain its inner hole"
    );
    for case in cases {
        validate_source_scene(&case.scene, BundleLimits::default()).unwrap();
    }
}

#[test]
fn controls_separate_curve_fill_single_clip_and_coincident_coverage() {
    let cases = fixtures::geometry_cases();
    for (name, clip_count, same_fill) in [
        ("curve-fill", 0, true),
        ("curve-clip-only", 1, false),
        ("curve-fill-and-coincident-clip", 1, true),
        ("curve-double-coincident-clip", 2, false),
    ] {
        let case = cases.iter().find(|c| c.name == name).unwrap();
        assert_eq!(
            case.scene
                .ops
                .iter()
                .filter(|op| matches!(op, SceneOp::PushLayer(_)))
                .count(),
            clip_count
        );
        assert_eq!(
            case.scene
                .ops
                .iter()
                .filter(|op| matches!(op, SceneOp::PopLayer))
                .count(),
            clip_count
        );
        let shape = case
            .scene
            .ops
            .iter()
            .find_map(|op| {
                if let SceneOp::Shape(s) = op {
                    Some(s)
                } else {
                    None
                }
            })
            .unwrap();
        assert_eq!(shape.path.ops == fixtures::outer_curve().ops, same_fill);
        assert!(
            shape.clip_rect.iter().all(|v| v.is_infinite()),
            "primitive clip would contaminate the layer control"
        );
    }
}

#[cfg(feature = "replay-cpu")]
#[test]
fn cpu_controls_preserve_transparent_exterior_hole_and_accent_alpha() {
    use netrender::vello_backends::{CpuSession, SparseResourceLimits};
    for case in fixtures::geometry_cases() {
        if ![
            "curve-fill",
            "curve-clip-only",
            "polyline-fill",
            "polyline-clip-only",
            "border-ellipse-accent-plus",
            "border-ellipse-uniform",
        ]
        .contains(&case.name.as_str())
        {
            continue;
        }
        let mut cpu = CpuSession::new(SparseResourceLimits::default());
        let pixels = cpu.render(&case.scene).unwrap();
        let rgba = pixels.data_as_u8_slice();
        let alpha = |x, y| rgba[((y * 40 + x) * 4 + 3) as usize];
        assert_eq!(alpha(35, 35), 0, "{} transparent exterior", case.name);
        if case.name.starts_with("border") {
            assert_eq!(alpha(12, 12), 0, "{} transparent border hole", case.name);
            if case.name.contains("accent") {
                assert!(
                    (1..255).contains(&alpha(2, 12)),
                    "accent alpha must stay translucent"
                );
            }
        } else {
            assert_eq!(alpha(12, 12), 255, "{} opaque interior", case.name);
        }
    }
}
