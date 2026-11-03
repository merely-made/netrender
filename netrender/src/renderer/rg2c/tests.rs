// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

use super::*;

fn scene() -> Scene {
    let mut scene = Scene::new(64, 64);
    scene.push_rect(0.0, 0.0, 32.0, 64.0, [0.0, 0.0, 1.0, 1.0]);
    scene.push_rect(32.0, 0.0, 64.0, 64.0, [1.0, 1.0, 0.0, 1.0]);
    let mut layer = crate::SceneLayer::clip(SceneClip::Rect {
        rect: [16.0, 8.0, 48.0, 56.0],
        radii: [0.0; 4],
    });
    layer.alpha = 0.6;
    layer.backdrop_filter = Some(SceneFilter::Blur(8.0));
    layer.filters.push(SceneFilter::Invert(1.0));
    scene.push_layer(layer);
    scene.push_rect(16.0, 8.0, 48.0, 56.0, [1.0, 0.0, 0.0, 0.35]);
    scene.pop_layer();
    scene
}

#[test]
fn decomposes_one_bounded_trailing_combined_effect() {
    let effect = decompose_combined_effect(&scene()).unwrap();
    assert_eq!(effect.bounds, [16.0, 8.0, 48.0, 56.0]);
    assert_eq!(effect.alpha, 0.6);
    assert_eq!(effect.backdrop_radius, 8.0);
    assert_eq!(effect.element_filters, [SceneFilter::Invert(1.0)]);
    assert_eq!(effect.prefix.ops.len(), 2);
    assert_eq!(effect.content.ops.len(), 1);
    assert!(
        !effect
            .prefix
            .ops
            .iter()
            .any(|op| matches!(op, SceneOp::PushLayer(layer) if layer.backdrop_filter.is_some()))
    );
    assert!(
        !effect
            .content
            .ops
            .iter()
            .any(|op| matches!(op, SceneOp::PushLayer(layer) if !layer.filters.is_empty()))
    );
}

#[test]
fn refuses_a_tail_instead_of_losing_painter_order() {
    let mut scene = scene();
    scene.push_rect(0.0, 0.0, 4.0, 4.0, [0.0, 1.0, 0.0, 1.0]);
    assert_eq!(
        decompose_combined_effect(&scene).unwrap_err(),
        CombinedEffectDecompositionError::LayerMustBeTrailing
    );
}
