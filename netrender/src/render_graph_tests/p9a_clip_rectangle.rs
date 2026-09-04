// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Phase 9A receipt — typed-plan `cs_clip_rectangle` rounded mask.

use std::sync::Arc;

use super::{make_renderer, pixel, render_clip_mask, render_to_bytes};
use crate::{ImageKey, NO_CLIP, Scene};

const W: u32 = 64;

fn channel_diff(a: u8, b: u8) -> u8 {
    (a as i16 - b as i16).unsigned_abs() as u8
}

fn assert_within_tol(actual: [u8; 4], expected: [u8; 4], tol: u8, where_: &str) {
    let diffs = [
        channel_diff(actual[0], expected[0]),
        channel_diff(actual[1], expected[1]),
        channel_diff(actual[2], expected[2]),
        channel_diff(actual[3], expected[3]),
    ];
    let max = *diffs.iter().max().unwrap();
    assert!(max <= tol, "{where_}: actual {actual:?}, expected {expected:?} (max channel diff = {max}, tol = {tol})");
}

#[test]
fn p9a_01_mask_pixels_match_sdf() {
    let _gpu_guard = super::gpu_test_guard();
    let renderer = make_renderer();
    let bytes = renderer.wgpu_device.read_rgba8_texture(
        &render_clip_mask(&renderer, W, [4.0, 4.0, 60.0, 60.0], 16.0, true),
        W,
        W,
    );
    let center = pixel(&bytes, W, 32, 32);
    assert_eq!(center[3], 255, "center should have full alpha coverage");
    for &c in &center[..4] {
        assert!(c >= 250, "center channel should be near-1.0, got {c}");
    }
    let outside = pixel(&bytes, W, 1, 1);
    for &c in &outside[..4] {
        assert!(c <= 5, "far-corner channel should be ~0, got {c}");
    }
    let arc_outside = pixel(&bytes, W, 5, 5);
    for &c in &arc_outside[..4] {
        assert!(c <= 5, "(5,5) outside rounded corner should be ~0, got {c}");
    }
    let edge_inside = pixel(&bytes, W, 32, 5);
    for &c in &edge_inside[..4] {
        assert!(c >= 250, "top edge should be near-1.0, got {c}");
    }
}

#[test]
fn p9a_02_mask_composes_as_red_rect() {
    let _gpu_guard = super::gpu_test_guard();
    let renderer = make_renderer();
    let mask = Arc::new(render_clip_mask(
        &renderer,
        W,
        [4.0, 4.0, 60.0, 60.0],
        16.0,
        true,
    ));
    const MASK_KEY: ImageKey = 0xCAFE_9A1F;
    renderer.insert_image_vello(MASK_KEY, mask);
    let mut scene = Scene::new(W, W);
    scene.push_image_full(
        0.0,
        0.0,
        W as f32,
        W as f32,
        [0.0, 0.0, 1.0, 1.0],
        [1.0, 0.0, 0.0, 0.999],
        MASK_KEY,
        0,
        NO_CLIP,
    );
    let bytes = render_to_bytes(&renderer, &scene);
    assert_within_tol(pixel(&bytes, W, 32, 32), [255, 0, 0, 255], 2, "center red");
    for &(x, y) in &[(0, 0), (63, 0), (0, 63), (63, 63)] {
        assert_within_tol(pixel(&bytes, W, x, y), [0, 0, 0, 255], 2, "corner black");
    }
    assert_within_tol(pixel(&bytes, W, 32, 5), [255, 0, 0, 255], 2, "edge red");
}
