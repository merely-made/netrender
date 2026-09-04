// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Phase 9B receipt — typed-plan box-shadow mask chain.

use super::{make_renderer, pixel, render_box_shadow_mask, render_to_bytes};
use crate::{ImageKey, NO_CLIP, Scene};

const W: u32 = 64;

#[test]
fn p9b_01_blur_softens_mask_edges() {
    let _gpu_guard = super::gpu_test_guard();
    let renderer = make_renderer();
    let mask = render_box_shadow_mask(&renderer, W, [16.0, 16.0, 48.0, 48.0], 8.0);
    let bytes = renderer.wgpu_device.read_rgba8_texture(&mask, W, W);
    assert!(
        pixel(&bytes, W, 32, 32)[3] >= 240,
        "center should remain covered"
    );
    let just_outside = pixel(&bytes, W, 14, 32);
    assert!(
        just_outside[3] > 5 && just_outside[3] < 250,
        "just-outside pixel should be soft, got {}",
        just_outside[3]
    );
    assert!(
        pixel(&bytes, W, 0, 0)[3] <= 5,
        "far outside should remain clear"
    );
}

#[test]
fn p9b_02_drop_shadow_composite() {
    let _gpu_guard = super::gpu_test_guard();
    let renderer = make_renderer();
    const SHADOW_KEY: ImageKey = 0xCAFE_9B0F;
    renderer.insert_image_vello(
        SHADOW_KEY,
        render_box_shadow_mask(&renderer, W, [16.0, 16.0, 48.0, 48.0], 8.0),
    );
    let mut scene = Scene::new(W, W);
    scene.push_image_full(
        0.0,
        0.0,
        W as f32,
        W as f32,
        [0.0, 0.0, 1.0, 1.0],
        [0.3, 0.3, 0.3, 0.999],
        SHADOW_KEY,
        0,
        NO_CLIP,
    );
    let bytes = render_to_bytes(&renderer, &scene);
    let center = pixel(&bytes, W, 32, 32);
    assert!(
        center[0] > 60 && center[0] < 100,
        "shadow interior should be dark gray, got {center:?}"
    );
    let halo = pixel(&bytes, W, 14, 32);
    assert!(
        halo[0] >= 1 && halo[0] < 100,
        "halo should be soft, got {halo:?}"
    );
    let far = pixel(&bytes, W, 0, 0);
    assert!(
        far[0] < 5 && far[1] < 5 && far[2] < 5,
        "far outside should be black, got {far:?}"
    );
}
