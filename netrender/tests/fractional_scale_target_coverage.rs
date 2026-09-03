// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Auto-DPI D2 — the scaled render covers the WHOLE target texture.
//!
//! A host lays out at `physical / scale` and carries that viewport as an
//! integer, so the division truncates before the scene ever reaches the
//! rasterizer. Re-deriving the render size as `round(viewport * scale)` then
//! lands *short* of the texture at every fractional scale — 800 physical over
//! a layout scale of 1.8 lays out at 444, and `round(444 * 1.8)` is 799, so
//! the 800th row is never written and stays at the texture's zero fill.
//!
//! Measured in the wild as 1120 fully transparent pixels in a genet host
//! capture on a 200% display at 0.9 content zoom (layout scale 1.8), against
//! zero at zoom 1.0. It is not a zoom defect: a 150% display reproduces it
//! with no zoom at all (1000 physical over 1.5 lays out at 666, and
//! `round(666 * 1.5)` is 999).
//!
//! Two halves to the receipt:
//!
//! - `legacy_*` — CPU only, no GPU needed. Pins the arithmetic both ways: the
//!   old derivation already landed on the target at scale 1.0 and at any
//!   integer scale that DIVIDES the physical size (so reading the target
//!   instead is a no-op on every surface a HiDPI display at zoom 1.0
//!   produces), and lost exactly one row everywhere it does not — which is
//!   every fractional scale in practice, and an odd surface at device scale
//!   2.0 as well.
//! - `scaled_render_*` — GPU. Renders through the public
//!   `Renderer::render_vello_scaled` entry and counts untouched pixels in the
//!   readback, the same measurement the host capture makes.

use netrender::{ColorLoad, NetrenderOptions, Scene, boot, create_netrender_instance};

const TILE_SIZE: u32 = 64;

/// The render size netrender derived before this receipt existed, given the
/// physical target and the layout scale — including the host's own truncating
/// division, which is where the precision actually goes.
fn legacy_render_size(physical: u32, scale: f32) -> u32 {
    let viewport = (physical as f32 / scale) as u32;
    ((viewport as f32) * scale).round().max(1.0) as u32
}

/// The viewport a host lays out for a physical target at `scale`, the
/// truncation included.
fn host_viewport(physical: u32, scale: f32) -> u32 {
    (physical as f32 / scale) as u32
}

/// The no-op proof: wherever the old derivation already landed on the target,
/// reading the target hands vello the same number. That is scale 1.0 for every
/// size, and an integer scale for every size it divides — which is what a
/// HiDPI surface at zoom 1.0 gives you, the genet host captures included
/// (1800x1280 and 1120x800 at device scale 2.0).
#[test]
fn legacy_size_already_matched_the_target_where_the_scale_divides_it() {
    for physical in [63u32, 64, 256, 511, 800, 801, 900, 1120, 1281, 1800] {
        assert_eq!(
            legacy_render_size(physical, 1.0),
            physical,
            "scale 1.0 into a {physical}px target must be the identity"
        );
    }
    for (physical, scale) in [
        (1800u32, 2.0f32),
        (1280, 2.0),
        (1120, 2.0),
        (800, 2.0),
        (512, 2.0),
        (1024, 2.0),
        (768, 3.0),
        (900, 3.0),
    ] {
        assert_eq!(
            legacy_render_size(physical, scale),
            physical,
            "scale {scale} divides {physical}, so it was already exact"
        );
    }
}

/// The defect. It is the truncating division that loses the row, so it fires
/// wherever the scale does NOT divide the physical target — every fractional
/// scale in practice, but an odd surface at an integer scale too.
#[test]
fn legacy_size_fell_one_short_where_the_scale_does_not_divide_the_target() {
    // The measured case: an 1120x800 genet frame at layout scale 1.8. 1.8
    // divides 1120 to within a rounding step but not 800, which is why the
    // capture lost exactly one 1120-wide ROW — 1120 transparent pixels — and
    // not a column with it.
    assert_eq!(host_viewport(800, 1.8), 444);
    assert_eq!(legacy_render_size(800, 1.8), 799);
    assert_eq!(legacy_render_size(1120, 1.8), 1120);
    // A 150% display with no content zoom at all.
    assert_eq!(legacy_render_size(1000, 1.5), 999);
    // 125% zoom on a 200% display.
    assert_eq!(legacy_render_size(1281, 2.5), 1280);
    // Not only fractional scales: an odd surface at device scale 2.0 loses a
    // row today, and a 64px target at 3.0 loses one too.
    assert_eq!(legacy_render_size(801, 2.0), 800);
    assert_eq!(legacy_render_size(64, 3.0), 63);
}

fn make_target(device: &wgpu::Device, dim: u32) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("scaled target coverage"),
        size: wgpu::Extent3d {
            width: dim,
            height: dim,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::STORAGE_BINDING
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[wgpu::TextureFormat::Rgba8UnormSrgb],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor {
        label: Some("scaled target coverage storage view"),
        format: Some(wgpu::TextureFormat::Rgba8Unorm),
        ..Default::default()
    });
    (texture, view)
}

/// Lay out a red quarter-square on a blue ground at `physical / scale`, render
/// it at `scale` into a `physical`-square texture, and hand back the readback.
///
/// The ground is an opaque CLEAR, so every pixel vello is asked to write comes
/// back with alpha 255 and every pixel it is NOT asked to write keeps the
/// texture's zero fill. Counting alpha-0 pixels is then exactly the host
/// capture's transparent-pixel measurement.
fn render_scaled_square(physical: u32, scale: f32) -> Vec<u8> {
    let viewport = host_viewport(physical, scale);
    let handles = boot().expect("wgpu boot");
    let device = handles.device.clone();
    let renderer = create_netrender_instance(
        handles.clone(),
        NetrenderOptions {
            tile_cache_size: Some(TILE_SIZE),
            enable_vello: true,
            ..Default::default()
        },
    )
    .expect("create_netrender_instance");

    let mut scene = Scene::new(viewport, viewport);
    let quarter = (viewport / 2) as f32;
    scene.push_rect(0.0, 0.0, quarter, quarter, [1.0, 0.0, 0.0, 1.0]);

    let (target, view) = make_target(&device, physical);
    renderer.render_vello_scaled(&scene, &view, ColorLoad::Clear(wgpu::Color::BLUE), scale);

    renderer
        .wgpu_device
        .read_rgba8_texture(&target, physical, physical)
}

/// Pixels vello was never asked to write, as `(count, first (x, y))`.
fn untouched(bytes: &[u8], dim: u32) -> (usize, Option<(u32, u32)>) {
    let mut count = 0usize;
    let mut first = None;
    for y in 0..dim {
        for x in 0..dim {
            if bytes[((y * dim + x) * 4 + 3) as usize] == 0 {
                count += 1;
                first.get_or_insert((x, y));
            }
        }
    }
    (count, first)
}

fn pixel(bytes: &[u8], dim: u32, x: u32, y: u32) -> [u8; 4] {
    let i = ((y * dim + x) * 4) as usize;
    [bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]
}

/// The repair. At layout scale 1.8 the host lays out an 800px target at 444,
/// and the old derivation rendered 799 of its 800 rows.
#[test]
fn scaled_render_fills_a_fractionally_scaled_target() {
    const PHYSICAL: u32 = 800;
    const SCALE: f32 = 1.8;

    // Without the repair this render is 799x799 into an 800x800 texture.
    assert_eq!(legacy_render_size(PHYSICAL, SCALE), PHYSICAL - 1);

    let bytes = render_scaled_square(PHYSICAL, SCALE);
    let (count, first) = untouched(&bytes, PHYSICAL);
    assert_eq!(
        count, 0,
        "{count} pixels of the {PHYSICAL}x{PHYSICAL} target were never written \
         (first at {first:?}); the render stopped short of the texture"
    );

    // Name the two edges the truncation drops, so a regression says which.
    let last = PHYSICAL - 1;
    assert_ne!(
        pixel(&bytes, PHYSICAL, last / 2, last)[3],
        0,
        "the last ROW is unpainted"
    );
    assert_ne!(
        pixel(&bytes, PHYSICAL, last, last / 2)[3],
        0,
        "the last COLUMN is unpainted"
    );
}

/// The other half of the no-op claim, on the GPU: at an integer scale the
/// target is covered as before AND the content edge lands where it always did
/// — a 256-logical square at scale 2.0 ends exactly on physical column 512,
/// with no resampled seam on either side of it.
#[test]
fn scaled_render_is_unchanged_at_an_integer_scale() {
    const PHYSICAL: u32 = 1024;
    const SCALE: f32 = 2.0;

    // The number handed to vello is the same one the old derivation produced.
    assert_eq!(legacy_render_size(PHYSICAL, SCALE), PHYSICAL);

    let bytes = render_scaled_square(PHYSICAL, SCALE);
    let (count, first) = untouched(&bytes, PHYSICAL);
    assert_eq!(
        count, 0,
        "{count} pixels were never written (first at {first:?})"
    );

    // The quarter-square is logical 0..256 at scale 2.0, so physical 0..512.
    let row = 100;
    let inside = pixel(&bytes, PHYSICAL, 0, row);
    let outside = pixel(&bytes, PHYSICAL, PHYSICAL - 1, row);
    assert_eq!(
        pixel(&bytes, PHYSICAL, 511, row),
        inside,
        "column 511 is not fully inside the square — the content moved"
    );
    assert_eq!(
        pixel(&bytes, PHYSICAL, 512, row),
        outside,
        "column 512 is not fully outside the square — the content moved"
    );
    assert!(
        inside[0] > inside[2],
        "the square should be red, got {inside:?}"
    );
    assert!(
        outside[2] > outside[0],
        "the ground should be blue, got {outside:?}"
    );
}
