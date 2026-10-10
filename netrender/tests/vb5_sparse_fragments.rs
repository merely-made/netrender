// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Source-retention correctness. All tests are CPU-only, even in combined builds.
#![cfg(any(feature = "vello-cpu", feature = "vello-hybrid"))]
use netrender::{Scene, SceneFragment, SceneOp, SceneLayer, SceneFilter, Transform};
use netrender::vello_backends::{
    SparseFragmentRegistry, SparseFragmentLimits, SparseResourceLimits, SparseSessionError,
    VelloBackend,
};

fn registry() -> SparseFragmentRegistry {
    SparseFragmentRegistry::new(SparseFragmentLimits::default())
}
fn resolve(registry: &SparseFragmentRegistry, scene: &Scene) -> Result<Scene, SparseSessionError> {
    registry.resolve(scene, VelloBackend::Cpu, SparseResourceLimits::default())
}
fn red() -> SceneFragment {
    let mut fragment = SceneFragment::new();
    fragment.push_rect(0., 0., 16., 16., [1., 0., 0., 1.]);
    fragment
}

#[test]
fn keys_generations_removal_and_budgets() {
    let mut registry = registry();
    let id = registry.register_fragment(red()).unwrap();
    assert_eq!(registry.generation(id), Some(0));
    let stats = registry.stats();
    let mut invalid = red();
    invalid.ops.push(SceneOp::PopLayer);
    assert!(registry.update_fragment(id, invalid).is_err());
    assert_eq!(registry.stats(), stats);
    assert_eq!(registry.generation(id), Some(0));
    registry.update_fragment(id, red()).unwrap();
    assert_eq!(registry.generation(id), Some(1));
    let mut frame = Scene::new(64, 64);
    frame.place_fragment(id, Transform::IDENTITY);
    // Another registry cannot alias the ID, even when it has its own first entry.
    let mut foreign = self::registry();
    let foreign_id = foreign.register_fragment(red()).unwrap();
    assert_ne!(id, foreign_id);
    assert!(
        matches!(resolve(&foreign, &frame), Err(SparseSessionError::MissingFragment { id: missing }) if missing == id)
    );
    assert!(registry.remove_fragment(id));
    assert!(matches!(
        resolve(&registry, &frame),
        Err(SparseSessionError::MissingFragment { .. })
    ));
    assert_ne!(registry.register_fragment(red()).unwrap(), id);
    registry.clear();
    assert_eq!(registry.stats().fragments, 0);
    let mut bounded = SparseFragmentRegistry::new(SparseFragmentLimits {
        max_fragments: 1,
        ..Default::default()
    });
    bounded.register_fragment(red()).unwrap();
    assert!(matches!(
        bounded.register_fragment(red()),
        Err(SparseSessionError::ResourceBudgetExceeded { .. })
    ));
    assert_eq!(bounded.stats().fragments, 1);
    let mut frame = Scene::new(64, 64);
    let id = foreign_id;
    for _ in 0..4 {
        frame.place_fragment(id, Transform::IDENTITY);
    }
    assert!(matches!(
        foreign.resolve(
            &frame,
            VelloBackend::Cpu,
            SparseResourceLimits {
                max_ops: 12,
                ..Default::default()
            }
        ),
        Err(SparseSessionError::ResourceBudgetExceeded { .. })
    ));
}

#[test]
fn malformed_and_unsupported_content_is_refused() {
    let mut registry = registry();
    let mut nested = Scene::new(64, 64);
    nested.place_fragment(7, Transform::IDENTITY);
    assert!(registry
        .register_fragment(SceneFragment::from_scene(nested))
        .is_err());
    let mut malformed = red();
    if let SceneOp::Rect(rect) = &mut malformed.ops[0] {
        rect.transform_id = 999;
    }
    assert!(registry.register_fragment(malformed).is_err());
    let mut malformed = red();
    malformed.transforms.clear();
    assert!(registry.register_fragment(malformed).is_err());
    let mut filtered = Scene::new(64, 64);
    let mut layer = SceneLayer::alpha(1.);
    layer.filters.push(SceneFilter::Blur(2.));
    filtered.ops.push(SceneOp::PushLayer(layer));
    filtered.push_rect(0., 0., 10., 10., [1.; 4]);
    filtered.ops.push(SceneOp::PopLayer);
    let id = registry
        .register_fragment(SceneFragment::from_scene(filtered))
        .unwrap();
    let mut frame = Scene::new(64, 64);
    frame.place_fragment(id, Transform::IDENTITY);
    for backend in [VelloBackend::Cpu, VelloBackend::Hybrid] {
        assert!(matches!(
            registry.resolve(&frame, backend, Default::default()),
            Err(SparseSessionError::Admission(_))
        ));
    }
    let mut malformed = red();
    malformed.transforms.push(Transform::scale_2d(0., 1.));
    let id = registry.register_fragment(malformed).unwrap();
    let mut frame = Scene::new(64, 64);
    frame.place_fragment(id, Transform::IDENTITY);
    assert!(resolve(&registry, &frame).is_err());
}

#[cfg(feature = "vello-cpu")]
mod cpu {
    use super::*;
    use netrender::{FontBlob, Glyph, ImageData, SceneClip, peniko::Blob};
    use netrender::vello_backends::CpuSession;
    use skrifa::MetadataProvider;
    use std::sync::Arc;
    const FONT: &[u8] = include_bytes!("fixtures/vb3_fonts/variabletest_box.ttf");
    fn session() -> CpuSession {
        CpuSession::new(Default::default())
    }
    fn pixel(bytes: &[u8], x: usize, y: usize) -> [u8; 4] {
        bytes[(y * 64 + x) * 4..][..4].try_into().unwrap()
    }
    fn content() -> Scene {
        let mut content = Scene::new(64, 64);
        content.push_rect(0., 0., 16., 16., [1., 0., 0., 1.]);
        content.push_image(
            16.,
            0.,
            32.,
            16.,
            73,
            ImageData::from_bytes(1, 1, vec![0, 0, 255, 255]),
        );
        let font = content.push_font(FontBlob {
            data: Blob::new(Arc::new(FONT.to_vec())),
            index: 0,
        });
        let id = skrifa::FontRef::new(FONT)
            .unwrap()
            .charmap()
            .map('A' as u32)
            .unwrap()
            .to_u32();
        content.push_glyph_run(font, 16., vec![Glyph { id, x: 2., y: 34. }], [1.; 4]);
        content
    }

    #[test]
    fn local_geometry_and_rounded_device_clip_rotate_under_placement() {
        let mut source = Scene::new(64, 64);
        let local = source.push_transform(Transform::translate_2d(4., 0.));
        source.push_rect_clipped_rounded(
            0.,
            0.,
            16.,
            16.,
            [1., 0., 0., 1.],
            local,
            [0., 0., 8., 16.],
            [2.; 4],
        );
        let mut registry = registry();
        let id = registry
            .register_fragment(SceneFragment::from_scene(source))
            .unwrap();
        let mut frame = Scene::new(64, 64);
        frame.place_fragment(
            id,
            Transform::rotate_2d(std::f32::consts::FRAC_PI_2)
                .then(&Transform::translate_2d(40., 8.)),
        );
        let actual = session()
            .render(&resolve(&registry, &frame).unwrap())
            .unwrap();
        let bytes = actual.data_as_u8_slice();
        assert_eq!(pixel(bytes, 32, 14), [255, 0, 0, 255]);
        assert_eq!(
            pixel(bytes, 32, 10),
            [0; 4],
            "local geometry begins at y=12 after placement"
        );
        assert_eq!(
            pixel(bytes, 32, 20),
            [0; 4],
            "fragment clip ends at y=16 after placement"
        );
        assert!(
            pixel(bytes, 24, 15)[3] < 255,
            "rounded corner follows placement"
        );
    }

    #[test]
    fn retained_text_images_match_direct_scene_and_update_without_stale_pixels() {
        let mut registry = registry();
        let original = content();
        let expected = session().render(&original).unwrap();
        let id = registry
            .register_fragment(SceneFragment::from_scene(original))
            .unwrap();
        let mut frame = Scene::new(64, 64);
        // Exercise palette remapping with an unused independent parent font.
        frame.push_font(FontBlob {
            data: Blob::new(Arc::new(FONT.to_vec())),
            index: 0,
        });
        frame.place_fragment(id, Transform::IDENTITY);
        let mut cpu = session();
        let first = cpu.render(&resolve(&registry, &frame).unwrap()).unwrap();
        assert_eq!(first.data_as_u8_slice(), expected.data_as_u8_slice());
        let images = cpu.stats();
        let text = cpu.text_stats();
        cpu.render(&resolve(&registry, &frame).unwrap()).unwrap();
        assert_eq!(cpu.stats().hydrated_images, images.hydrated_images);
        assert_eq!(cpu.text_stats().cached_glyphs, text.cached_glyphs);
        let mut updated = content();
        updated
            .image_sources
            .insert(73, ImageData::from_bytes(1, 1, vec![0, 255, 0, 255]));
        registry
            .update_fragment(id, SceneFragment::from_scene(updated))
            .unwrap();
        let next = cpu.render(&resolve(&registry, &frame).unwrap()).unwrap();
        assert_eq!(pixel(next.data_as_u8_slice(), 20, 8), [0, 255, 0, 255]);
        assert_eq!(pixel(first.data_as_u8_slice(), 20, 8), [0, 0, 255, 255]);
        assert_eq!(cpu.stats().hydrated_images, images.hydrated_images + 1);
        // A conflicting shared key refuses without mutating admitted resources.
        frame.set_image_source(73, ImageData::from_bytes(1, 1, vec![255; 4]));
        let before = cpu.stats();
        let before_text = cpu.text_stats();
        assert!(matches!(
            resolve(&registry, &frame),
            Err(SparseSessionError::InvalidImage { .. })
        ));
        assert_eq!(cpu.stats(), before);
        assert_eq!(cpu.text_stats(), before_text);
    }

    #[test]
    fn placement_moves_fragment_clip_but_preserves_parent_clip_opacity_and_order() {
        let mut fragment = Scene::new(64, 64);
        fragment.push_rect_clipped(0., 0., 16., 16., [1., 0., 0., 1.], 0, [0., 0., 8., 16.]);
        let mut registry = registry();
        let id = registry
            .register_fragment(SceneFragment::from_scene(fragment))
            .unwrap();
        let mut frame = Scene::new(64, 64);
        frame.ops.push(SceneOp::PushLayer(SceneLayer::alpha(0.5)));
        frame
            .ops
            .push(SceneOp::PushLayer(SceneLayer::clip(SceneClip::Rect {
                rect: [16., 8., 26., 24.],
                radii: [0.; 4],
            })));
        frame.place_fragment(
            id,
            Transform::scale_2d(2., 1.).then(&Transform::translate_2d(16., 8.)),
        );
        frame.ops.push(SceneOp::PopLayer);
        frame.ops.push(SceneOp::PopLayer);
        frame.push_rect(16., 8., 20., 12., [0., 0., 1., 1.]);
        let actual = session()
            .render(&resolve(&registry, &frame).unwrap())
            .unwrap();
        let bytes = actual.data_as_u8_slice();
        assert_eq!(pixel(bytes, 18, 10), [0, 0, 255, 255]);
        assert_eq!(pixel(bytes, 24, 16), [128, 0, 0, 128]);
        assert_eq!(pixel(bytes, 28, 16), [0; 4]);
        assert_eq!(pixel(bytes, 4, 4), [0; 4]);
        // With no parent clip the moved local clip ends at x=32, not x=8.
        let mut frame = Scene::new(64, 64);
        frame.place_fragment(
            id,
            Transform::scale_2d(2., 1.).then(&Transform::translate_2d(16., 8.)),
        );
        let moved = session()
            .render(&resolve(&registry, &frame).unwrap())
            .unwrap();
        assert_eq!(pixel(moved.data_as_u8_slice(), 30, 16), [255, 0, 0, 255]);
        assert_eq!(pixel(moved.data_as_u8_slice(), 34, 16), [0; 4]);
    }

    #[test]
    fn external_cpu_source_replacement_and_resource_refusal_are_atomic() {
        let mut source = content();
        source.image_sources.clear();
        let mut registry = registry();
        let id = registry
            .register_fragment(SceneFragment::from_scene(source))
            .unwrap();
        let mut frame = Scene::new(64, 64);
        frame.place_fragment(id, Transform::IDENTITY);
        let expanded = resolve(&registry, &frame).unwrap();
        let mut cpu = session();
        assert!(matches!(
            cpu.render(&expanded),
            Err(SparseSessionError::MissingImage { key: 73 })
        ));
        assert_eq!(cpu.stats().hydrated_images, 0);
        assert_eq!(cpu.text_stats().cached_glyphs, 0);
        for color in [[0, 0, 255, 255], [0, 255, 0, 255]] {
            cpu.set_image_source(73, ImageData::from_bytes(1, 1, color.to_vec()))
                .unwrap();
            let image = cpu.render(&expanded).unwrap();
            assert_eq!(pixel(image.data_as_u8_slice(), 20, 8), color);
        }
        cpu.remove_image_source(73);
        let before = cpu.stats();
        let text = cpu.text_stats();
        assert!(cpu.render(&expanded).is_err());
        assert_eq!(cpu.stats(), before);
        assert_eq!(cpu.text_stats(), text);
        // A missing parent font must not become valid by appending fragment fonts.
        let mut malformed = Scene::new(64, 64);
        malformed.push_glyph_run(1, 16., vec![], [1.; 4]);
        malformed.place_fragment(id, Transform::IDENTITY);
        assert!(resolve(&registry, &malformed).is_err());
    }
}
