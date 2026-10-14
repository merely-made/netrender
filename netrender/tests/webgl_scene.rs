// Copyright 2026 Mark Alan Boykin
// SPDX-License-Identifier: MPL-2.0

//! Host-independent admission for the browser sparse realization.
//! Runtime pixel/cache/generation qualification belongs to the browser fixture.
use netrender::{Scene, SceneLayer, SceneFilter, SceneOp};
use netrender::vello_backends::{VelloBackend, validate_scene_for_backend};

#[test]
fn webgl_names_its_own_route_and_admits_ordered_sparse_geometry() {
    let backend = VelloBackend::WebGl2;
    assert!(backend.capabilities().gpu_output);
    assert!(!backend.capabilities().cpu_output);
    assert_eq!(
        backend.is_compiled(),
        cfg!(all(target_arch = "wasm32", feature = "vello-webgl"))
    );
    let mut scene = Scene::new(31, 23);
    scene.push_rect(0., 0., 31., 23., [1., 0., 0., 1.]);
    scene.ops.push(SceneOp::PushLayer(SceneLayer::alpha(0.5)));
    scene.push_rect(0., 0., 10., 10., [0., 0., 1., 1.]);
    scene.ops.push(SceneOp::PopLayer);
    validate_scene_for_backend(backend, &scene).unwrap();
}

#[test]
fn late_filter_is_explicitly_refused_instead_of_drawing_a_prefix() {
    let mut scene = Scene::new(31, 23);
    scene.push_rect(0., 0., 31., 23., [1., 0., 0., 1.]);
    let mut layer = SceneLayer::alpha(1.);
    layer.filters.push(SceneFilter::Blur(2.));
    scene.ops.push(SceneOp::PushLayer(layer));
    scene.push_rect(0., 0., 10., 10., [0., 0., 1., 1.]);
    scene.ops.push(SceneOp::PopLayer);
    let failure = validate_scene_for_backend(VelloBackend::WebGl2, &scene).unwrap_err();
    assert!(format!("{failure:?}").contains("filter"));
    assert_eq!(
        scene.ops.len(),
        4,
        "admission does not mutate caller content"
    );
}

#[cfg(feature = "vello-cpu")]
#[test]
fn cpu_reference_for_browser_image_corner_anchors() {
    use netrender::{ImageData, NO_CLIP};
    use netrender::vello_backends::{CpuSession, SparseResourceLimits};
    let mut scene = Scene::new(128, 128);
    scene.push_rect(0., 0., 128., 128., [0., 0., 1., 1.]);
    scene.image_sources.insert(
        71,
        ImageData::from_bytes(
            2,
            2,
            vec![
                255, 0, 0, 128, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 0,
            ],
        ),
    );
    scene.push_image_full(8., 40., 40., 72., [0., 0., 1., 1.], [1.; 4], 71, 0, NO_CLIP);
    let mut session = CpuSession::new(SparseResourceLimits::default());
    let frame = session.render(&scene).unwrap();
    let pixel = |x: usize, y: usize| &frame.data_as_u8_slice()[(y * 128 + x) * 4..][..4];
    for (x, y, expected) in [
        (8, 40, [128u8, 0, 127, 255]),
        (39, 40, [0, 255, 0, 255]),
        (39, 71, [0, 0, 255, 255]),
    ] {
        let actual = pixel(x, y);
        assert!(
            actual.iter().zip(expected).all(|(a, b)| a.abs_diff(b) <= 4),
            "reference {x},{y}: {actual:?}, expected {expected:?}"
        );
    }
}
