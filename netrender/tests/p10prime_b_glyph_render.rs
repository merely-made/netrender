// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Phase 10b' — real-font GPU smoke test.
//!
//! Loads the repository's bundled Inter subset, constructs a glyph
//! run, runs it through render_vello, and verifies pixels are painted.

use std::sync::Arc;

use netrender::{
    ColorLoad, FontBlob, Glyph, NetrenderOptions, Scene, boot, create_netrender_instance,
    peniko::Blob,
};
use skrifa::MetadataProvider;

const DIM: u32 = 128;
const TILE_SIZE: u32 = 64;
const INTER: &[u8] = include_bytes!("fixtures/vb3_fonts/Inter.var.subset.ttf");

fn glyph_id(ch: char) -> u32 {
    skrifa::FontRef::new(INTER)
        .expect("bundled Inter fixture")
        .charmap()
        .map(ch as u32)
        .expect("bundled Inter fixture contains glyph")
        .to_u32()
}

fn make_target(device: &wgpu::Device) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("p10b target"),
        size: wgpu::Extent3d {
            width: DIM,
            height: DIM,
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
        label: Some("p10b view"),
        format: Some(wgpu::TextureFormat::Rgba8Unorm),
        ..Default::default()
    });
    (texture, view)
}

/// End-to-end smoke: load a font, push a glyph run, render, verify
/// at least *some* pixels were painted. We don't check specific
/// shapes, only that the
/// rasterization pipeline produced visible output.
#[test]
fn p10b_01_render_real_font_glyph() {
    let handles = boot().expect("wgpu boot");
    let renderer = create_netrender_instance(
        handles.clone(),
        NetrenderOptions {
            tile_cache_size: Some(TILE_SIZE),
            enable_vello: true,
            ..Default::default()
        },
    )
    .expect("create_netrender_instance");

    let mut scene = Scene::new(DIM, DIM);
    let font_id = scene.push_font(FontBlob {
        data: Blob::new(Arc::new(INTER.to_vec())),
        index: 0,
    });

    // Resolve glyph IDs from the fixture's cmap so the test stays valid
    // if the bundled subset is regenerated.
    let glyphs = vec![
        Glyph {
            id: glyph_id('a'),
            x: 16.0,
            y: 64.0,
        },
        Glyph {
            id: glyph_id('l'),
            x: 32.0,
            y: 64.0,
        },
        Glyph {
            id: glyph_id('n'),
            x: 48.0,
            y: 64.0,
        },
        Glyph {
            id: glyph_id('s'),
            x: 64.0,
            y: 64.0,
        },
        Glyph {
            id: glyph_id('t'),
            x: 80.0,
            y: 64.0,
        },
    ];

    scene.push_glyph_run(font_id, 32.0, glyphs, [1.0, 1.0, 1.0, 1.0]);

    let (target, view) = make_target(&handles.device);
    renderer.render_vello(&scene, &view, ColorLoad::Clear(wgpu::Color::BLACK));
    let bytes = renderer.wgpu_device.read_rgba8_texture(&target, DIM, DIM);

    // Count non-black pixels (background was cleared to black).
    // White text-on-black should produce thousands of paint pixels.
    let mut painted = 0usize;
    for chunk in bytes.chunks_exact(4) {
        // "Painted" = at least one of RGB above background black.
        if chunk[0] > 16 || chunk[1] > 16 || chunk[2] > 16 {
            painted += 1;
        }
    }

    eprintln!(
        "p10b_01: painted {} non-background pixels (of {})",
        painted,
        DIM * DIM
    );
    assert!(
        painted > 100,
        "render_vello produced only {} painted pixels — expected hundreds for 5 glyphs at size 32",
        painted
    );
}
