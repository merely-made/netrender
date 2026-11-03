// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

use super::*;

fn unsupported_scene(kind: &str) -> Scene {
    let mut scene = Scene::new(8, 8);
    match kind {
        "Image" => scene.push_image(
            0.0,
            0.0,
            8.0,
            8.0,
            1,
            netrender::ImageData::from_bytes(1, 1, vec![255; 4]),
        ),
        "Pattern" => scene.push_pattern(1, [0.0, 0.0, 8.0, 8.0], [1.0, 1.0]),
        "GlyphRun" => scene.push_glyph_run(0, 12.0, Vec::new(), [1.0, 1.0, 1.0, 1.0]),
        "Fragment" => scene.place_fragment(1, Transform::IDENTITY),
        "ElementFilter" => {
            let mut layer = SceneLayer::alpha(1.0);
            layer.filters = vec![SceneFilter::Blur(2.0)];
            scene.push_layer(layer);
            scene.pop_layer();
        }
        "BackdropFilter" => {
            let mut layer = SceneLayer::alpha(1.0);
            layer.backdrop_filter = Some(SceneFilter::Blur(2.0));
            scene.push_layer(layer);
            scene.pop_layer();
        }
        "BackdropColorFilter" => {
            let mut layer = SceneLayer::alpha(1.0);
            layer.backdrop_filter = Some(SceneFilter::Invert(1.0));
            scene.push_layer(layer);
            scene.pop_layer();
        }
        _ => unreachable!("unsupported fixture kind"),
    }
    scene
}

fn assert_unsupported(
    scene_kind: &str,
    expected_operation: &'static str,
    expected_reason: &'static str,
) {
    let scene = unsupported_scene(scene_kind);
    for (backend, result) in [
        (VelloBackend::Cpu, scene_to_vello_cpu(&scene).map(|_| ())),
        (
            VelloBackend::Hybrid,
            scene_to_vello_hybrid(&scene).map(|_| ()),
        ),
    ] {
        match result.expect_err("sparse adapter should refuse unsupported operation") {
            BackendAdmissionError::UnsupportedOperation {
                backend: actual_backend,
                op_index,
                operation,
                reason,
            } => {
                assert_eq!(actual_backend, backend);
                assert_eq!(op_index, 0);
                assert_eq!(operation, expected_operation);
                assert_eq!(reason, expected_reason);
            }
            other => panic!("{scene_kind}: unexpected refusal {other:?}"),
        }
    }
}

#[test]
fn rg2a_sparse_refusal_table_is_typed_and_attributed() {
    let classic = VelloBackend::Classic.capabilities();
    assert!(classic.element_filters);
    assert!(classic.backdrop_blur);
    assert!(!classic.backdrop_color_filters);
    validate_scene_for_backend(VelloBackend::Classic, &unsupported_scene("ElementFilter"))
        .expect("Classic element-filter admission");
    validate_scene_for_backend(VelloBackend::Classic, &unsupported_scene("BackdropFilter"))
        .expect("Classic backdrop-blur admission");
    assert!(matches!(
        validate_scene_for_backend(
            VelloBackend::Classic,
            &unsupported_scene("BackdropColorFilter")
        ),
        Err(BackendAdmissionError::UnsupportedOperation {
            backend: VelloBackend::Classic,
            op_index: 0,
            operation: "PushLayer",
            reason: "Classic backdrop preprocessing currently supports Blur only",
        })
    ));

    for backend in [VelloBackend::Cpu, VelloBackend::Hybrid] {
        let capabilities = backend.capabilities();
        assert!(!capabilities.images);
        assert!(!capabilities.patterns);
        assert!(!capabilities.text);
        assert!(!capabilities.element_filters);
        assert!(!capabilities.backdrop_blur);
        assert!(!capabilities.backdrop_color_filters);
        assert!(!capabilities.retained_fragments);
    }
    assert_unsupported(
        "Image",
        "Image",
        "image resources require an owned CpuSession or HybridSession",
    );
    assert_unsupported(
        "Pattern",
        "Pattern",
        "pattern resources require an owned CpuSession or HybridSession",
    );
    assert_unsupported(
        "GlyphRun",
        "GlyphRun",
        "text resources require an owned CpuSession or HybridSession",
    );
    assert_unsupported(
        "Fragment",
        "Fragment",
        "fragment registries are backend-owned; only Hybrid append has been proven upstream",
    );
    assert_unsupported(
        "ElementFilter",
        "PushLayer",
        "Netrender filter graphs have not been mapped to sparse filters",
    );
    assert_unsupported(
        "BackdropFilter",
        "PushLayer",
        "Netrender filter graphs have not been mapped to sparse filters",
    );

    let mut invalid_transform = Scene::new(8, 8);
    invalid_transform.push_rect_transformed(0.0, 0.0, 4.0, 4.0, [1.0; 4], 7);
    for (backend, result) in [
        (
            VelloBackend::Classic,
            validate_scene_for_backend(VelloBackend::Classic, &invalid_transform),
        ),
        (
            VelloBackend::Cpu,
            scene_to_vello_cpu(&invalid_transform).map(|_| ()),
        ),
        (
            VelloBackend::Hybrid,
            scene_to_vello_hybrid(&invalid_transform).map(|_| ()),
        ),
    ] {
        assert!(matches!(
            result,
            Err(BackendAdmissionError::InvalidTransform {
                backend: actual,
                op_index: 0,
                transform_id: 7,
            }) if actual == backend
        ));
    }

    let mut unbalanced = Scene::new(8, 8);
    unbalanced.pop_layer();
    for (backend, result) in [
        (
            VelloBackend::Classic,
            validate_scene_for_backend(VelloBackend::Classic, &unbalanced),
        ),
        (
            VelloBackend::Cpu,
            scene_to_vello_cpu(&unbalanced).map(|_| ()),
        ),
        (
            VelloBackend::Hybrid,
            scene_to_vello_hybrid(&unbalanced).map(|_| ()),
        ),
    ] {
        assert!(matches!(
            result,
            Err(BackendAdmissionError::UnbalancedLayers {
                backend: actual,
                op_index: Some(0),
            }) if actual == backend
        ));
    }

    let mut unterminated = Scene::new(8, 8);
    unterminated.push_layer_alpha(1.0);
    for (backend, result) in [
        (
            VelloBackend::Classic,
            validate_scene_for_backend(VelloBackend::Classic, &unterminated),
        ),
        (
            VelloBackend::Cpu,
            scene_to_vello_cpu(&unterminated).map(|_| ()),
        ),
        (
            VelloBackend::Hybrid,
            scene_to_vello_hybrid(&unterminated).map(|_| ()),
        ),
    ] {
        assert!(matches!(
            result,
            Err(BackendAdmissionError::UnbalancedLayers {
                backend: actual,
                op_index: None,
            }) if actual == backend
        ));
    }

    let oversized = Scene::new(65_536, 1);
    for (backend, result) in [
        (
            VelloBackend::Cpu,
            scene_to_vello_cpu(&oversized).map(|_| ()),
        ),
        (
            VelloBackend::Hybrid,
            scene_to_vello_hybrid(&oversized).map(|_| ()),
        ),
    ] {
        assert!(matches!(
            result,
            Err(BackendAdmissionError::ViewportTooLarge {
                backend: actual,
                width: 65_536,
                height: 1,
            }) if actual == backend
        ));
    }
}
