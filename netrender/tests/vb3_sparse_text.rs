// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! VB3 synthetic caller-shaped text acceptance, not a real Isocosm panel capture.
//! Portable licensed fixtures; CPU tests never request a GPU device.
#![cfg(any(feature = "vello-cpu", feature = "vello-hybrid"))]

use std::sync::Arc;
use netrender::{FontBlob, Glyph, Scene, SceneGlyphRun, SceneLayer, SceneOp, Transform, peniko::Blob};
use netrender::vello_backends::{
    SparseResourceLimits, SparseResourceStats, SparseSessionError, SparseTextLimits,
    SparseTextStats,
};
use skrifa::MetadataProvider;

const DIM: u32 = 128;
const BOX_FONT: &[u8] = include_bytes!("fixtures/vb3_fonts/variabletest_box.ttf");
const INTER: &[u8] = include_bytes!("fixtures/vb3_fonts/Inter.var.subset.ttf");

trait Session {
    fn render(&mut self, scene: &Scene) -> Result<Vec<u8>, SparseSessionError>;
    fn text_stats(&self) -> SparseTextStats;
    fn resource_stats(&self) -> SparseResourceStats;
    fn clear_text_cache(&mut self);
}

fn font(bytes: &[u8]) -> FontBlob {
    FontBlob {
        data: Blob::new(Arc::new(bytes.to_vec())),
        index: 0,
    }
}

/// Wrap unchanged outline tables in a deterministic TTC v1 container. SFNT table
/// offsets are absolute from the collection start, not from each face directory.
fn collection() -> FontBlob {
    let mut bytes = Vec::from(&b"ttcf\0\x01\0\0\0\0\0\x02\0\0\0\0\0\0\0\0"[..]);
    for (face, data) in [INTER, BOX_FONT].into_iter().enumerate() {
        let start = bytes.len().next_multiple_of(4);
        bytes.resize(start, 0);
        bytes[12 + face * 4..16 + face * 4].copy_from_slice(&(start as u32).to_be_bytes());
        bytes.extend_from_slice(data);
        let count = u16::from_be_bytes(data[4..6].try_into().unwrap()) as usize;
        for table in 0..count {
            let record = 12 + table * 16;
            let offset =
                u32::from_be_bytes(data[record + 8..record + 12].try_into().unwrap()) as usize;
            bytes[start + record + 8..start + record + 12]
                .copy_from_slice(&((start + offset) as u32).to_be_bytes());
            if &data[record..record + 4] == b"head" {
                // In a collection the per-font checkSumAdjustment is not used.
                bytes[start + offset + 8..start + offset + 12].fill(0);
            }
        }
    }
    FontBlob {
        data: Blob::new(Arc::new(bytes)),
        index: 1,
    }
}

fn without_head() -> FontBlob {
    let mut bytes = BOX_FONT.to_vec();
    let count = u16::from_be_bytes(bytes[4..6].try_into().unwrap()) as usize;
    let record = (0..count)
        .map(|i| 12 + i * 16)
        .find(|offset| &bytes[*offset..*offset + 4] == b"head")
        .unwrap();
    bytes[record..record + 4].copy_from_slice(b"xxxx");
    font(&bytes)
}

fn glyph(bytes: &[u8], ch: char, x: f32, y: f32) -> Glyph {
    let id = skrifa::FontRef::new(bytes)
        .unwrap()
        .charmap()
        .map(ch as u32)
        .expect("fixture contains character")
        .to_u32();
    Glyph { id, x, y }
}

fn scene(bytes: &[u8], ch: char) -> Scene {
    let mut scene = Scene::new(DIM, DIM);
    let id = scene.push_font(font(bytes));
    scene.push_glyph_run(id, 64.0, vec![glyph(bytes, ch, 16.0, 96.0)], [1.0; 4]);
    scene
}

fn run(scene: &mut Scene) -> &mut SceneGlyphRun {
    scene
        .ops
        .iter_mut()
        .find_map(|op| match op {
            SceneOp::GlyphRun(run) => Some(run),
            _ => None,
        })
        .expect("glyph fixture")
}

fn pixel(bytes: &[u8], x: u32, y: u32) -> [u8; 4] {
    bytes[((y * DIM + x) * 4) as usize..][..4]
        .try_into()
        .unwrap()
}

fn alpha_mass(bytes: &[u8]) -> u64 {
    bytes.chunks_exact(4).map(|p| p[3] as u64).sum()
}

fn alpha_distance(a: &[u8], b: &[u8]) -> u64 {
    a.chunks_exact(4)
        .zip(b.chunks_exact(4))
        .map(|(a, b)| a[3].abs_diff(b[3]) as u64)
        .sum()
}

fn bounds(bytes: &[u8], left: u32, right: u32) -> [u32; 4] {
    let mut result = [DIM, DIM, 0, 0];
    for y in 0..DIM {
        for x in left..right {
            if pixel(bytes, x, y)[3] > 16 {
                result[0] = result[0].min(x);
                result[1] = result[1].min(y);
                result[2] = result[2].max(x);
                result[3] = result[3].max(y);
            }
        }
    }
    assert!(
        result[0] <= result[2] && result[1] <= result[3],
        "visible glyph required"
    );
    result
}

fn positioned_and_composed(session: &mut impl Session) {
    let mut positioned = scene(BOX_FONT, 'A');
    let r = run(&mut positioned);
    r.font_size = 40.0;
    r.glyphs = vec![
        glyph(BOX_FONT, 'A', 8.0, 64.0),
        glyph(BOX_FONT, 'A', 70.0, 96.0),
    ];
    let pixels = session.render(&positioned).unwrap();
    let first = bounds(&pixels, 0, 64);
    let second = bounds(&pixels, 64, DIM);
    assert_eq!(
        [
            second[0] - first[0],
            second[1] - first[1],
            second[2] - first[2],
            second[3] - first[3]
        ],
        [62, 32, 62, 32],
        "caller-provided glyph origins survive unchanged"
    );
    assert_eq!(
        pixel(&pixels, 50, 70)[3],
        0,
        "caller-selected gap remains empty"
    );

    let mut composed = Scene::new(DIM, DIM);
    let id = composed.push_font(font(BOX_FONT));
    composed.push_rect(0.0, 0.0, DIM as f32, DIM as f32, [0.0, 0.0, 0.0, 1.0]);
    composed.push_layer(SceneLayer::alpha(0.5));
    let transform = composed
        .push_transform(Transform::scale_2d(2.0, 2.0).then(&Transform::translate_2d(8.0, 8.0)));
    composed.push_glyph_run_full(
        id,
        32.0,
        vec![glyph(BOX_FONT, 'A', 8.0, 32.0)],
        [0.25, 0.125, 0.0, 0.5],
        transform,
        [32.0, 40.0, 56.0, 68.0],
        [0.0; 4],
    );
    composed.pop_layer();
    composed.push_rect(40.0, 52.0, 48.0, 60.0, [0.0, 1.0, 0.0, 1.0]);
    let output = session.render(&composed).unwrap();
    let maximum_red = output.chunks_exact(4).map(|p| p[0]).max().unwrap();
    assert!(
        maximum_red.abs_diff(32) <= 3,
        "premultiplied brush and layer alpha: {maximum_red}"
    );
    assert_eq!(
        pixel(&output, 44, 56),
        [0, 255, 0, 255],
        "later solid covers the text"
    );
    for y in 0..DIM {
        for x in 0..DIM {
            if !(32..56).contains(&x) || !(40..68).contains(&y) {
                assert_eq!(
                    pixel(&output, x, y),
                    [0, 0, 0, 255],
                    "device clip at {x},{y}"
                );
            }
        }
    }
}

fn variations_and_caller_fallback(session: &mut impl Session) {
    let lower = session.render(&scene(BOX_FONT, 'A')).unwrap();
    let lower_reference = session.render(&scene(BOX_FONT, '\u{2584}')).unwrap();
    let mut raised = scene(BOX_FONT, 'A');
    run(&mut raised).font_axis_values = vec![(*b"UPWD", 350.0)];
    let upper = session.render(&raised).unwrap();
    let upper_reference = session.render(&scene(BOX_FONT, '\u{2580}')).unwrap();
    assert!(alpha_mass(&lower) > 10_000 && alpha_mass(&upper) > 10_000);
    assert!(
        alpha_distance(&lower, &lower_reference) < 1_000,
        "default A matches independent lower-block glyph"
    );
    assert!(
        alpha_distance(&upper, &upper_reference) < 1_000,
        "UPWD350 matches independent upper-block glyph"
    );
    assert!(
        alpha_distance(&lower, &upper) > 10_000,
        "axis change must move actual ink"
    );

    let mut weighted = scene(INTER, 'a');
    run(&mut weighted).font_axis_values = vec![(*b"wght", 100.0)];
    let light = session.render(&weighted).unwrap();
    run(&mut weighted).font_axis_values = vec![(*b"wght", 900.0)];
    let heavy = session.render(&weighted).unwrap();
    assert!(
        alpha_mass(&heavy) > alpha_mass(&light) * 13 / 10,
        "real variable font gains ink at heavy weight"
    );
    run(&mut weighted).font_axis_values.push((*b"ZZZZ", 500.0));
    let unknown_axis = session.render(&weighted).unwrap();
    assert_eq!(
        alpha_distance(&heavy, &unknown_axis),
        0,
        "unknown valid axis tag is ignored"
    );

    assert!(skrifa::FontRef::new(INTER)
        .unwrap()
        .charmap()
        .map('A' as u32)
        .is_none());
    let mut fallback = Scene::new(DIM, DIM);
    let primary = fallback.push_font(font(INTER));
    let secondary = fallback.push_font(font(BOX_FONT));
    fallback.push_glyph_run(
        primary,
        40.0,
        vec![glyph(INTER, 'a', 8.0, 64.0)],
        [1.0, 0.0, 0.0, 1.0],
    );
    fallback.push_glyph_run(
        secondary,
        40.0,
        vec![glyph(BOX_FONT, 'A', 70.0, 64.0)],
        [0.0, 1.0, 0.0, 1.0],
    );
    let pixels = session.render(&fallback).unwrap();
    let mut primary_ink = 0;
    let mut fallback_ink = 0;
    for y in 0..DIM {
        for x in 0..DIM {
            let p = pixel(&pixels, x, y);
            if x < 64 && p[0] > 100 && p[1] == 0 {
                primary_ink += 1;
            }
            if x >= 64 && p[1] > 100 && p[0] == 0 {
                fallback_ink += 1;
            }
        }
    }
    assert!(
        primary_ink > 20 && fallback_ink > 20,
        "both caller-selected fonts must paint: {primary_ink}/{fallback_ink}"
    );

    let collection = collection();
    let first_face = skrifa::FontRef::from_index(collection.data.as_ref(), 0).unwrap();
    let second_face = skrifa::FontRef::from_index(collection.data.as_ref(), 1).unwrap();
    assert!(first_face.charmap().map('A' as u32).is_none());
    assert!(second_face.charmap().map('A' as u32).is_some());
    let mut collection_scene = scene(BOX_FONT, 'A');
    collection_scene.fonts[1] = collection;
    let collection_pixels = session.render(&collection_scene).unwrap();
    assert_eq!(
        alpha_distance(&lower, &collection_pixels),
        0,
        "nonzero collection index selects the same box outlines as the standalone font"
    );
}

fn invalid_text(session: &mut impl Session) {
    let good = scene(BOX_FONT, 'A');
    let reference = session.render(&good).unwrap();
    for case in 0..8 {
        let mut bad = good.clone();
        match case {
            0 => run(&mut bad).font_id = 999,
            1 => bad.fonts[1].data = Blob::new(Arc::new(vec![0, 1, 2, 3])),
            2 => bad.fonts[1].index = 1,
            3 => run(&mut bad).glyphs[0].id = u32::MAX,
            4 => run(&mut bad).glyphs[0].x = f32::NAN,
            5 => run(&mut bad).font_size = f32::NAN,
            6 => run(&mut bad).font_axis_values = vec![(*b"UPWD", f32::INFINITY)],
            7 => bad.fonts[1] = without_head(),
            _ => unreachable!(),
        }
        bad.image_sources
            .insert(177, netrender::ImageData::from_bytes(1, 1, vec![255; 4]));
        let before_text = session.text_stats();
        let before_images = session.resource_stats();
        let error = session.render(&bad).unwrap_err();
        match case {
            0..=2 | 7 => assert!(
                matches!(error, SparseSessionError::InvalidFont { .. }),
                "case {case}: {error:?}"
            ),
            3 => assert!(
                matches!(error, SparseSessionError::InvalidGlyph { .. }),
                "{error:?}"
            ),
            _ => assert!(
                matches!(error, SparseSessionError::InvalidScene { .. }),
                "case {case}: {error:?}"
            ),
        }
        assert_eq!(
            session.text_stats(),
            before_text,
            "failed text preparation is atomic"
        );
        assert_eq!(
            session.resource_stats(),
            before_images,
            "text refusal cannot apply image updates"
        );
        assert_eq!(
            alpha_distance(&session.render(&good).unwrap(), &reference),
            0
        );
    }
}

fn bounded_text_limits() -> SparseTextLimits {
    SparseTextLimits {
        max_fonts: 1,
        max_font_bytes: BOX_FONT.len(),
        max_cached_glyphs: 1,
        max_outline_segments: 4096,
        max_cached_outline_segments: 4096,
        max_cache_frames: 3,
        ..Default::default()
    }
}

fn text_lifecycle(session: &mut impl Session) {
    let mut current = scene(BOX_FONT, 'A');
    current
        .image_sources
        .insert(99, netrender::ImageData::from_bytes(1, 1, vec![255; 4]));
    let first_pixels = session.render(&current).unwrap();
    let first = session.text_stats();
    assert_eq!(
        (first.font_count, first.font_bytes, first.cached_glyphs),
        (1, BOX_FONT.len(), 1)
    );
    session.render(&current).unwrap();
    assert_eq!(
        session.text_stats().prepared_glyphs,
        first.prepared_glyphs,
        "same blob/face/glyph reuses preparation"
    );
    for _ in 0..8 {
        current.fonts[1] = font(BOX_FONT);
        assert_eq!(
            alpha_distance(&session.render(&current).unwrap(), &first_pixels),
            0
        );
        let stats = session.text_stats();
        assert_eq!(stats.font_count, 1);
        assert_eq!(stats.font_bytes, BOX_FONT.len());
        assert_eq!(stats.cached_glyphs, 1);
        assert!(stats.cached_outline_segments <= 4096);
        assert!(stats.cache_frames <= 3);
    }
    assert!(
        session.text_stats().cache_resets > first.cache_resets,
        "fresh blobs cannot grow an unbounded preparation epoch"
    );
    let before = session.text_stats();
    let images = session.resource_stats();
    session.clear_text_cache();
    let cleared = session.text_stats();
    assert_eq!(
        (
            cleared.font_count,
            cleared.font_bytes,
            cleared.cached_glyphs,
            cleared.cached_outline_segments
        ),
        (0, 0, 0, 0)
    );
    assert_eq!(cleared.prepared_glyphs, before.prepared_glyphs);
    assert_eq!(
        session.resource_stats(),
        images,
        "clearing text preserves owned image resources"
    );
    assert_eq!(
        alpha_distance(&session.render(&current).unwrap(), &first_pixels),
        0
    );

    let mut too_many = current.clone();
    run(&mut too_many)
        .glyphs
        .push(glyph(BOX_FONT, 'B', 80.0, 96.0));
    let before = session.text_stats();
    assert!(matches!(
        session.render(&too_many),
        Err(SparseSessionError::ResourceBudgetExceeded { .. })
    ));
    assert_eq!(session.text_stats(), before);
    assert!(matches!(
        session.render(&scene(INTER, 'a')),
        Err(SparseSessionError::ResourceBudgetExceeded { .. })
    ));
    assert_eq!(session.text_stats(), before);
    assert_eq!(
        alpha_distance(&session.render(&current).unwrap(), &first_pixels),
        0
    );
}

fn repeated_outline_budget(session: &mut impl Session) {
    let good = scene(BOX_FONT, 'A');
    let reference = session.render(&good).unwrap();
    let mut repeated = good.clone();
    run(&mut repeated).glyphs = vec![glyph(BOX_FONT, 'A', 16.0, 96.0); 128];
    let text_before = session.text_stats();
    let images_before = session.resource_stats();
    // Only one distinct prepared outline, but 128 draws require real frame work.
    // Other limits remain at their defaults and cannot explain this refusal.
    assert!(matches!(
        session.render(&repeated),
        Err(SparseSessionError::ResourceBudgetExceeded { limit: 64, .. })
    ));
    assert_eq!(session.text_stats(), text_before);
    assert_eq!(session.resource_stats(), images_before);
    assert_eq!(
        alpha_distance(&session.render(&good).unwrap(), &reference),
        0
    );
}

fn repeated_outline_limits() -> SparseTextLimits {
    SparseTextLimits {
        max_outline_segments: 64,
        ..Default::default()
    }
}

#[cfg(feature = "vello-cpu")]
mod cpu {
    use super::*;
    use netrender::vello_backends::CpuSession;
    impl Session for CpuSession {
        fn render(&mut self, scene: &Scene) -> Result<Vec<u8>, SparseSessionError> {
            CpuSession::render(self, scene).map(|pixels| pixels.data_as_u8_slice().to_vec())
        }
        fn text_stats(&self) -> SparseTextStats {
            CpuSession::text_stats(self)
        }
        fn resource_stats(&self) -> SparseResourceStats {
            self.stats()
        }
        fn clear_text_cache(&mut self) {
            CpuSession::clear_text_cache(self);
        }
    }
    #[test]
    fn cpu_positions_composition_without_device() {
        positioned_and_composed(&mut CpuSession::new(SparseResourceLimits::default()));
    }
    #[test]
    fn cpu_variations_fallback_without_device() {
        variations_and_caller_fallback(&mut CpuSession::new(SparseResourceLimits::default()));
    }
    #[test]
    fn cpu_invalid_text_without_device() {
        invalid_text(&mut CpuSession::new(SparseResourceLimits::default()));
    }
    #[test]
    fn cpu_bounded_text_lifecycle_without_device() {
        text_lifecycle(&mut CpuSession::new_with_text_limits(
            SparseResourceLimits::default(),
            bounded_text_limits(),
        ));
    }
    #[test]
    fn cpu_repeated_outline_budget_without_device() {
        repeated_outline_budget(&mut CpuSession::new_with_text_limits(
            SparseResourceLimits::default(),
            repeated_outline_limits(),
        ));
    }
}

#[cfg(all(feature = "vello-hybrid", not(target_arch = "wasm32")))]
mod hybrid {
    use super::*;
    use netrender::vello_backends::HybridSession;
    struct Gpu {
        session: HybridSession,
        handles: netrender::WgpuHandles,
        readback: netrender::WgpuDevice,
        texture: wgpu::Texture,
    }
    impl Gpu {
        fn new(limits: SparseTextLimits) -> Self {
            let handles = netrender::boot().expect("VB3 existing device");
            let info = handles.adapter.get_info();
            eprintln!(
                "VB3 text adapter={:?} backend={:?} driver={:?}",
                info.name, info.backend, info.driver
            );
            let session = HybridSession::new_with_text_limits(
                &handles.device,
                &handles.queue,
                SparseResourceLimits::default(),
                limits,
            )
            .unwrap();
            let readback = netrender::WgpuDevice::with_external(handles.clone()).unwrap();
            let texture = handles.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("VB3 text acceptance target"),
                size: wgpu::Extent3d {
                    width: DIM,
                    height: DIM,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::COPY_SRC
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            Self {
                session,
                handles,
                readback,
                texture,
            }
        }
    }
    impl Session for Gpu {
        fn render(&mut self, scene: &Scene) -> Result<Vec<u8>, SparseSessionError> {
            let before = self.readback.read_rgba8_texture(&self.texture, DIM, DIM);
            let mut encoder =
                self.handles
                    .device
                    .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                        label: Some("VB3 text frame"),
                    });
            let result = self.session.render(
                scene,
                &self.handles.device,
                &self.handles.queue,
                &mut encoder,
                &self.texture,
            );
            self.handles.queue.submit([encoder.finish()]);
            let after = self.readback.read_rgba8_texture(&self.texture, DIM, DIM);
            if result.is_err() {
                assert_eq!(after, before, "text refusal must preserve visible target");
            }
            result.map(|()| after)
        }
        fn text_stats(&self) -> SparseTextStats {
            self.session.text_stats()
        }
        fn resource_stats(&self) -> SparseResourceStats {
            self.session.stats()
        }
        fn clear_text_cache(&mut self) {
            self.session.clear_text_cache();
        }
    }
    #[test]
    fn hybrid_positions_composition_on_existing_device() {
        positioned_and_composed(&mut Gpu::new(SparseTextLimits::default()));
    }
    #[test]
    fn hybrid_variations_fallback_on_existing_device() {
        variations_and_caller_fallback(&mut Gpu::new(SparseTextLimits::default()));
    }
    #[test]
    fn hybrid_invalid_text_on_existing_device() {
        invalid_text(&mut Gpu::new(SparseTextLimits::default()));
    }
    #[test]
    fn hybrid_bounded_text_lifecycle_on_existing_device() {
        text_lifecycle(&mut Gpu::new(bounded_text_limits()));
    }
    #[test]
    fn hybrid_repeated_outline_budget_on_existing_device() {
        repeated_outline_budget(&mut Gpu::new(repeated_outline_limits()));
    }
}
