// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::tile_cache::TileCache;
#[test]
fn tiled_long_text_only_encodes_glyphs_near_each_tile() {
    use crate::scene::{FontBlob, Glyph};
    use skrifa::MetadataProvider;
    let glyph_id = skrifa::FontRef::new(include_bytes!(
        "../../tests/fixtures/vb3_fonts/Inter.var.subset.ttf"
    ))
    .unwrap()
    .charmap()
    .map('a' as u32)
    .unwrap()
    .to_u32();
    let mut scene = Scene::new(1024, 64);
    let font = scene.push_font(FontBlob {
        data: vello::peniko::Blob::new(std::sync::Arc::new(
            include_bytes!("../../tests/fixtures/vb3_fonts/Inter.var.subset.ttf").to_vec(),
        )),
        index: 0,
    });
    scene.push_glyph_run(
        font,
        13.0,
        (0..100)
            .map(|i| Glyph {
                id: glyph_id,
                x: i as f32 * 10.0,
                y: 32.0,
            })
            .collect(),
        [1.0; 4],
    );
    let mut cache = TileCache::new(64);
    cache.invalidate(&scene);
    let ops = filter_scene_to_tile_indices(&scene, cache.op_indices((0, 0)));
    let view = crate::vello_rasterizer::SceneView::from_scene(&scene);
    let lowered = crate::vello_rasterizer::scene_to_vello_with_tile(
        &view,
        ops.iter().copied(),
        &Default::default(),
        &Default::default(),
        Some(([0.0, 0.0, 64.0, 64.0], 1.0)),
    );
    let count = lowered.encoding().resources.glyphs.len();
    assert!(
        count < 20,
        "tile encoded {count} glyphs from a 100-glyph run"
    );
}

#[test]
fn tile_keeps_all_glyphs_when_font_bounds_are_unavailable() {
    use crate::scene::{FontBlob, Glyph};
    let mut scene = Scene::new(1024, 64);
    let font = scene.push_font(FontBlob {
        data: vello::peniko::Blob::new(std::sync::Arc::new(
            include_bytes!("../../tests/fixtures/vb3_fonts/Inter.var.subset.ttf").to_vec(),
        )),
        index: 0,
    });
    scene.push_glyph_run(
        font,
        13.0,
        (0..100)
            .map(|i| Glyph {
                id: 65535,
                x: i as f32 * 10.0,
                y: 32.0,
            })
            .collect(),
        [1.0; 4],
    );
    let mut cache = TileCache::new(64);
    cache.invalidate(&scene);
    let ops = filter_scene_to_tile_indices(&scene, cache.op_indices((0, 0)));
    let view = crate::vello_rasterizer::SceneView::from_scene(&scene);
    let lowered = crate::vello_rasterizer::scene_to_vello_with_tile(
        &view,
        ops.iter().copied(),
        &Default::default(),
        &Default::default(),
        Some(([0.0, 0.0, 64.0, 64.0], 1.0)),
    );
    assert_eq!(lowered.encoding().resources.glyphs.len(), 100);
}

#[test]
fn tile_keeps_outline_when_glyph_header_understates_ink() {
    use crate::scene::{FontBlob, Glyph};
    use skrifa::MetadataProvider;
    let mut bytes = include_bytes!("../../tests/fixtures/vb3_fonts/Inter.var.subset.ttf").to_vec();
    let glyph = skrifa::FontRef::new(&bytes)
        .unwrap()
        .charmap()
        .map('a' as u32)
        .unwrap()
        .to_u32();
    let u16at = |b: &[u8], n: usize| u16::from_be_bytes([b[n], b[n + 1]]) as usize;
    let u32at = |b: &[u8], n: usize| u32::from_be_bytes(b[n..n + 4].try_into().unwrap()) as usize;
    let tables = u16at(&bytes, 4);
    let mut head = 0;
    let mut loca = 0;
    let mut glyf = 0;
    for i in 0..tables {
        let at = 12 + i * 16;
        match &bytes[at..at + 4] {
            b"head" => head = u32at(&bytes, at + 8),
            b"loca" => loca = u32at(&bytes, at + 8),
            b"glyf" => glyf = u32at(&bytes, at + 8),
            // Hide gvar so GlyphMetrics would take its static-header path.
            b"gvar" => bytes[at + 3] = b's',
            _ => {}
        }
    }
    assert!(head != 0 && loca != 0 && glyf != 0);
    let gid = glyph as usize;
    let offset = if u16at(&bytes, head + 50) == 0 {
        u16at(&bytes, loca + 2 * gid) * 2
    } else {
        u32at(&bytes, loca + 4 * gid)
    };
    let header = glyf + offset;
    bytes[header + 2..header + 10].fill(0);
    bytes[head + 18..head + 20].copy_from_slice(&16u16.to_be_bytes());
    let font_ref = skrifa::FontRef::new(&bytes).unwrap();
    let header_bounds = font_ref
        .glyph_metrics(
            skrifa::instance::Size::new(24.0),
            skrifa::instance::LocationRef::default(),
        )
        .bounds(skrifa::GlyphId::new(glyph))
        .unwrap();
    assert_eq!(header_bounds.x_max, 0.0);
    let glyph_outline = font_ref
        .outline_glyphs()
        .get(skrifa::GlyphId::new(glyph))
        .unwrap();
    let mut pen = skrifa::outline::pen::ControlBoundsPen::default();
    glyph_outline
        .draw(
            skrifa::outline::DrawSettings::unhinted(
                skrifa::instance::Size::new(24.0),
                skrifa::instance::LocationRef::default(),
            ),
            &mut pen,
        )
        .unwrap();
    let actual_bounds = pen.bounding_box().unwrap();
    assert!(actual_bounds.x_min > 24.0);
    let mut scene = Scene::new(512, 704);
    let font = scene.push_font(FontBlob {
        data: vello::peniko::Blob::new(std::sync::Arc::new(bytes)),
        index: 0,
    });
    scene.push_glyph_run(
        font,
        24.0,
        vec![Glyph {
            id: glyph,
            x: 10.0,
            y: 640.0,
        }],
        [1.0; 4],
    );
    let mut cache = TileCache::new(64);
    cache.invalidate(&scene);
    assert!(
        cache.op_indices((7, 9)).any(|index| index == 0),
        "overhanging outline must reach tile membership"
    );
    let ops = filter_scene_to_tile_indices(&scene, cache.op_indices((7, 9)));
    let view = crate::vello_rasterizer::SceneView::from_scene(&scene);
    let lowered = crate::vello_rasterizer::scene_to_vello_with_tile(
        &view,
        ops.iter().copied(),
        &Default::default(),
        &Default::default(),
        Some(([448.0, 576.0, 512.0, 640.0], 1.0)),
    );
    assert_eq!(
        lowered.encoding().resources.glyphs.len(),
        1,
        "ink crossing the tile must be retained"
    );
}

#[test]
fn glyph_culling_preserves_pixels_with_identical_tile_clips() {
    use crate::scene::{FontBlob, Glyph};
    use skrifa::MetadataProvider;
    let bytes = include_bytes!("../../tests/fixtures/vb3_fonts/Inter.var.subset.ttf");
    let font = skrifa::FontRef::new(bytes).unwrap();
    let glyph = font.charmap().map('a' as u32).unwrap().to_u32();
    let mut scene = Scene::new(256, 256);
    let id = scene.push_font(FontBlob {
        data: vello::peniko::Blob::new(std::sync::Arc::new(bytes.to_vec())),
        index: 0,
    });
    for y in [64.0, 128.0, 192.0] {
        scene.push_glyph_run_variable(
            id,
            24.0,
            (0..10)
                .map(|i| Glyph {
                    id: glyph,
                    x: 61.5 + i as f32 * 18.0,
                    y,
                })
                .collect(),
            [0.5, 0.5, 0.5, 0.5],
            vec![(*b"wght", 700.0)],
        );
    }
    let rotation = scene.push_transform(crate::scene::Transform::rotate_2d(0.03));
    if let Some(SceneOp::GlyphRun(run)) = scene.ops.last_mut() {
        run.transform_id = rotation;
    }
    let mut cache = TileCache::new(64);
    cache.invalidate(&scene);
    let view = crate::vello_rasterizer::SceneView::from_scene(&scene);
    let make_scene = |scale: f32, filter: bool| {
        let mut master = vello::Scene::new();
        for row in 0..4 {
            for col in 0..4 {
                let coord = (col, row);
                let rect = cache.tile_world_rect(coord).unwrap();
                let ops = filter_scene_to_tile_indices(&scene, cache.op_indices(coord));
                let tile = crate::vello_rasterizer::scene_to_vello_with_tile(
                    &view,
                    ops.iter().copied(),
                    &Default::default(),
                    &Default::default(),
                    filter.then_some((rect, scale)),
                );
                let clip = Rect::new(
                    rect[0] as f64,
                    rect[1] as f64,
                    rect[2] as f64,
                    rect[3] as f64,
                );
                master.push_layer(Fill::NonZero, Mix::Normal, 1.0, Affine::IDENTITY, &clip);
                master.append(&tile, None);
                master.pop_layer();
            }
        }
        let mut scaled = vello::Scene::new();
        scaled.append(&master, Some(Affine::scale(scale as f64)));
        scaled
    };
    let handles = crate::boot().expect("wgpu boot");
    let mut renderer = vello::Renderer::new(
        &handles.device,
        vello::RendererOptions {
            use_cpu: false,
            antialiasing_support: vello::AaSupport::area_only(),
            num_init_threads: None,
            pipeline_cache: None,
        },
    )
    .unwrap();
    let readback = netrender_device::WgpuDevice::with_external(handles.clone()).unwrap();
    let render = |renderer: &mut vello::Renderer, scene: &vello::Scene, background: Color| {
        let texture = handles.device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: 256,
                height: 256,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let target = texture.create_view(&wgpu::TextureViewDescriptor::default());
        renderer
            .render_to_texture(
                &handles.device,
                &handles.queue,
                scene,
                &target,
                &vello::RenderParams {
                    base_color: background,
                    width: 256,
                    height: 256,
                    antialiasing_method: vello::AaConfig::Area,
                },
            )
            .unwrap();
        readback.read_rgba8_texture(&texture, 256, 256)
    };
    for (label, background) in [
        ("opaque", Color::BLACK),
        ("transparent", Color::TRANSPARENT),
    ] {
        for scale in [1.0, 0.75, 1.5] {
            let reference = render(&mut renderer, &make_scene(scale, false), background);
            let actual = render(&mut renderer, &make_scene(scale, true), background);
            let differences = reference
                .chunks_exact(4)
                .zip(actual.chunks_exact(4))
                .filter(|(a, b)| a[3] != b[3] || (a[3] != 0 && a[..3] != b[..3]))
                .count();
            let sample: Vec<_> = reference
                .chunks_exact(4)
                .zip(actual.chunks_exact(4))
                .enumerate()
                .filter(|(_, (a, b))| a[3] != b[3] || (a[3] != 0 && a[..3] != b[..3]))
                .take(12)
                .map(|(i, (a, b))| (i % 256, i / 256, a.to_vec(), b.to_vec()))
                .collect();
            assert_eq!(
                differences, 0,
                "pixel differences on {label} at scale {scale}: {sample:?}"
            );
            assert!(
                actual
                    .chunks_exact(4)
                    .any(|p| p[3] != 0 && (p[0] != 0 || p[1] != 0 || p[2] != 0)),
                "glyphs must paint"
            );
        }
    }
}

#[test]
fn unknown_font_bounds_keep_run_in_all_tiles() {
    use crate::scene::{FontBlob, Glyph};
    let mut scene = Scene::new(1024, 64);
    let font = scene.push_font(FontBlob {
        data: vello::peniko::Blob::new(std::sync::Arc::new(vec![1, 2, 3])),
        index: 0,
    });
    scene.push_glyph_run(
        font,
        13.0,
        vec![Glyph {
            id: 1,
            x: 1.0,
            y: 32.0,
        }],
        [1.0; 4],
    );
    let mut cache = TileCache::new(64);
    cache.invalidate(&scene);
    assert!(cache.op_indices((15, 0)).any(|index| index == 0));
}
