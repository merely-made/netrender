// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Phase 9C receipt — typed-plan rectangular clip fast path.

use super::{make_renderer, pixel, render_clip_mask};
use crate::Renderer;

const W: u32 = 64;

fn render_mask(renderer: &Renderer, radius: f32, has_rounded_corners: bool) -> Vec<u8> {
    let mask = render_clip_mask(
        renderer,
        W,
        [16.0, 16.0, 48.0, 48.0],
        radius,
        has_rounded_corners,
    );
    renderer.wgpu_device.read_rgba8_texture(&mask, W, W)
}

#[test]
fn p9c_01_fast_path_is_axis_aligned_step() {
    let _gpu_guard = super::gpu_test_guard();
    let renderer = make_renderer();
    let bytes = render_mask(&renderer, 0.0, false);
    assert_eq!(pixel(&bytes, W, 32, 32)[0], 255, "interior pixel");
    assert_eq!(pixel(&bytes, W, 0, 0)[0], 0, "far-outside pixel");
    assert!(
        pixel(&bytes, W, 16, 16)[0] >= 200,
        "inner corner should be covered"
    );
    assert_eq!(
        pixel(&bytes, W, 14, 14)[0],
        0,
        "outer corner should be clear"
    );
}

#[test]
fn p9c_02_fast_path_pixel_match_rounded_at_zero_radius() {
    let _gpu_guard = super::gpu_test_guard();
    let renderer = make_renderer();
    let fast = render_mask(&renderer, 0.0, false);
    let rounded_zero = render_mask(&renderer, 0.0, true);
    assert_eq!(fast.len(), rounded_zero.len(), "readback length");
    let mut max_diff: u8 = 0;
    let mut diff_count = 0usize;
    for (a, b) in fast.iter().zip(rounded_zero.iter()) {
        let d = (*a as i16 - *b as i16).unsigned_abs() as u8;
        if d > 2 {
            diff_count += 1;
        }
        max_diff = max_diff.max(d);
    }
    assert_eq!(
        diff_count, 0,
        "fast path vs rounded zero-radius mismatch: {diff_count} channels, max {max_diff}"
    );
}
