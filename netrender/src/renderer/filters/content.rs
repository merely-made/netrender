// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

use crate::scene::{ImageKey, Scene};

/// The layer's *own content* — the ops strictly between `push_idx` and
/// `pop_idx` — as a flat sub-scene (the same palette clones as
/// [`super::build_prefix_scene`], so inner `transform_id`/`font_id`/`key` indices stay
/// valid). The outer `PushLayer`/`PopLayer` are excluded: the layer's
/// alpha/blend/clip get re-applied when the filtered image composites back.
/// Nested filters/backdrops are stripped so this first cut does not recurse.
pub(in crate::renderer) fn build_layer_content_scene(
    scene: &Scene,
    push_idx: usize,
    pop_idx: usize,
    backdrop_keys: &[ImageKey],
) -> Scene {
    use crate::scene::SceneOp;
    let mut content = Scene::new(scene.viewport_width, scene.viewport_height);
    content.transforms = scene.transforms.clone();
    content.fonts = scene.fonts.clone();
    content.image_sources = scene.image_sources.clone();
    content.root_alpha = scene.root_alpha;
    content.root_blend_mode = scene.root_blend_mode;
    // Only images injected by this frame's backdrop pass are excluded.
    content.ops = scene.ops[push_idx + 1..pop_idx]
        .iter()
        .filter(|op| !matches!(op, SceneOp::Image(im) if backdrop_keys.contains(&im.key)))
        .cloned()
        .collect();
    for op in &mut content.ops {
        if let SceneOp::PushLayer(l) = op {
            l.backdrop_filter = None;
            l.filters.clear();
        }
    }
    // The interior is already balanced; this is a defensive no-op.
    let mut depth: i32 = 0;
    for op in &content.ops {
        match op {
            SceneOp::PushLayer(_) => depth += 1,
            SceneOp::PopLayer => depth -= 1,
            _ => {}
        }
    }
    for _ in 0..depth.max(0) {
        content.ops.push(SceneOp::PopLayer);
    }
    content
}
