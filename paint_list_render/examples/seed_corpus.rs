// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Writes the seed fixtures in `tests/corpus/`.
//!
//! Run with `cargo run -p paint_list_render --example seed_corpus`.
//!
//! **This is not how you contribute a fixture.** These seeds are
//! hand-built because no consumer capture existed when the corpus was
//! created. A real capture is two lines wherever your app already has a
//! `PaintList`:
//!
//! ```ignore
//! let envelope = paint_list_api::PaintEnvelope::from_list(&list);
//! std::fs::write("my_scene.paintlist", postcard::to_allocvec(&envelope)?)?;
//! ```
//!
//! Drop the result in `tests/corpus/`, add a `.provenance` sidecar, and
//! run the harness once with `NETRENDER_CORPUS_BLESS=1` to record its
//! `.ops`. See `tests/corpus/README.md`.

use std::fs;
use std::path::Path;

use paint_list_api::items::{PathCommand, PathData, PathItem, RectItem};
use paint_list_api::specs::{ClipKind, ClipSpec, FilterOp, LayerSpec, TransformKind, TransformSpec};
use paint_list_api::{
    ColorF, CommonPlacement, DeviceIntSize, EngineId, LayoutPoint, LayoutRect, LayoutTransform,
    PaintCmd, PaintEnvelope,
};

fn rect(x0: f32, y0: f32, x1: f32, y1: f32) -> LayoutRect {
    LayoutRect::new(LayoutPoint::new(x0, y0), LayoutPoint::new(x1, y1))
}

fn fill(x0: f32, y0: f32, x1: f32, y1: f32, color: ColorF) -> PaintCmd {
    PaintCmd::DrawRect(RectItem {
        placement: CommonPlacement::new(rect(x0, y0, x1, y1)),
        color,
    })
}

fn rgba(r: f32, g: f32, b: f32, a: f32) -> ColorF {
    ColorF::new(r, g, b, a)
}

/// Uniform 2D scale, in the same literal form `genet-wpt` builds its
/// device-scale root transform.
fn scale_transform(scale: f32) -> PaintCmd {
    PaintCmd::PushTransform(TransformSpec {
        origin: LayoutPoint::new(0.0, 0.0),
        transform: LayoutTransform::new(
            scale, 0.0, 0.0, 0.0, 0.0, scale, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
        ),
        kind: TransformKind::Standard,
    })
}

/// Mirrors what `genet-wpt` actually emits per reftest: an opaque white
/// backdrop inserted at index 0, a device-scale root transform, page
/// content, then the matching pop.
fn wpt_reftest_page() -> PaintEnvelope {
    let mut commands = vec![
        fill(0.0, 0.0, 800.0, 600.0, ColorF::WHITE),
        scale_transform(2.0),
    ];

    // A heading band and two body columns: the shape a simple reftest
    // page lowers to once layout has run.
    commands.push(fill(16.0, 16.0, 384.0, 48.0, rgba(0.13, 0.13, 0.15, 1.0)));
    for col in 0..2 {
        let x0 = 16.0 + col as f32 * 192.0;
        for line in 0..8 {
            let y0 = 64.0 + line as f32 * 18.0;
            let width = if line % 3 == 2 { 120.0 } else { 176.0 };
            commands.push(fill(
                x0,
                y0,
                x0 + width,
                y0 + 10.0,
                rgba(0.35, 0.35, 0.4, 1.0),
            ));
        }
    }

    // A translucent callout box over the text, the common
    // stacking-context case.
    commands.push(PaintCmd::PushLayer(LayerSpec {
        opacity: 0.8,
        ..Default::default()
    }));
    commands.push(fill(48.0, 96.0, 336.0, 168.0, rgba(0.9, 0.95, 1.0, 1.0)));
    commands.push(fill(48.0, 96.0, 336.0, 100.0, rgba(0.2, 0.45, 0.85, 1.0)));
    commands.push(PaintCmd::PopLayer);

    commands.push(PaintCmd::PopTransform);

    PaintEnvelope {
        engine: EngineId::GENET,
        viewport: DeviceIntSize::new(800, 600),
        generation: 1,
        commands,
        fonts: Vec::new(),
        images: Vec::new(),
    }
}

/// Mirrors the shapes `components/cambium/sprigging` emits for host UI:
/// panel rects, a clipped scroll region, filled paths standing in for
/// icon/glyph outlines, and a blurred layer.
fn cambium_panel() -> PaintEnvelope {
    let mut commands = vec![fill(0.0, 0.0, 480.0, 320.0, rgba(0.11, 0.11, 0.13, 1.0))];

    // Title bar.
    commands.push(fill(0.0, 0.0, 480.0, 32.0, rgba(0.16, 0.16, 0.19, 1.0)));

    // A chevron, as a filled path. sprigging lowers glyph and icon
    // outlines through DrawPath.
    commands.push(PaintCmd::DrawPath(PathItem {
        placement: CommonPlacement::new(rect(12.0, 10.0, 24.0, 22.0)),
        path: PathData {
            commands: vec![
                PathCommand::MoveTo(LayoutPoint::new(14.0, 12.0)),
                PathCommand::LineTo(LayoutPoint::new(22.0, 16.0)),
                PathCommand::LineTo(LayoutPoint::new(14.0, 20.0)),
                PathCommand::Close,
            ],
        },
        fill: Some(rgba(0.85, 0.85, 0.9, 1.0)),
        stroke: None,
    }));

    // Clipped scroll region with rows that overflow it.
    commands.push(PaintCmd::PushClip(ClipSpec {
        kind: ClipKind::Rect(rect(0.0, 32.0, 480.0, 280.0)),
    }));
    for row in 0..14 {
        let y0 = 40.0 + row as f32 * 22.0;
        let shade = if row % 2 == 0 { 0.15 } else { 0.13 };
        commands.push(fill(
            8.0,
            y0,
            472.0,
            y0 + 20.0,
            rgba(shade, shade, shade + 0.02, 1.0),
        ));
        commands.push(fill(
            16.0,
            y0 + 6.0,
            240.0,
            y0 + 14.0,
            rgba(0.6, 0.6, 0.66, 1.0),
        ));
    }
    commands.push(PaintCmd::PopClip);

    // Blurred footer overlay: exercises the filter chain on a layer.
    commands.push(PaintCmd::PushLayer(LayerSpec {
        opacity: 0.94,
        filters: vec![FilterOp::Blur(6.0)],
        ..Default::default()
    }));
    commands.push(fill(0.0, 280.0, 480.0, 320.0, rgba(0.2, 0.2, 0.24, 0.85)));
    commands.push(PaintCmd::PopLayer);

    PaintEnvelope {
        engine: EngineId::GENET,
        viewport: DeviceIntSize::new(480, 320),
        generation: 7,
        commands,
        fonts: Vec::new(),
        images: Vec::new(),
    }
}

/// Deep clip / transform / layer nesting. Deliberately awkward: the
/// translator's stack handling is where consumer-visible ordering bugs
/// hide, and no consumer scene reliably exercises the deep case.
fn nested_stacks() -> PaintEnvelope {
    let mut commands = vec![fill(0.0, 0.0, 256.0, 256.0, rgba(0.05, 0.05, 0.08, 1.0))];

    for depth in 0..4 {
        let inset = 8.0 + depth as f32 * 12.0;
        commands.push(PaintCmd::PushClip(ClipSpec {
            kind: ClipKind::Rect(rect(inset, inset, 256.0 - inset, 256.0 - inset)),
        }));
        commands.push(scale_transform(1.0 + depth as f32 * 0.05));
        commands.push(PaintCmd::PushLayer(LayerSpec {
            opacity: 0.9 - depth as f32 * 0.1,
            ..Default::default()
        }));
        let c = 0.2 + depth as f32 * 0.2;
        commands.push(fill(
            inset,
            inset,
            256.0 - inset,
            256.0 - inset,
            rgba(c, 0.4, 1.0 - c, 1.0),
        ));
    }
    // Unwind in the mirror order.
    for _ in 0..4 {
        commands.push(PaintCmd::PopLayer);
        commands.push(PaintCmd::PopTransform);
        commands.push(PaintCmd::PopClip);
    }

    PaintEnvelope {
        engine: EngineId::GENET,
        viewport: DeviceIntSize::new(256, 256),
        generation: 3,
        commands,
        fonts: Vec::new(),
        images: Vec::new(),
    }
}

fn main() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("corpus");
    fs::create_dir_all(&dir).expect("create corpus dir");

    let seeds: Vec<(&str, PaintEnvelope)> = vec![
        ("wpt_reftest_page", wpt_reftest_page()),
        ("cambium_panel", cambium_panel()),
        ("nested_stacks", nested_stacks()),
    ];

    for (name, envelope) in seeds {
        let bytes = postcard::to_allocvec(&envelope).expect("encode envelope");
        let path = dir.join(format!("{name}.paintlist"));
        fs::write(&path, &bytes).expect("write fixture");
        println!(
            "{name}.paintlist  {} commands, {} bytes",
            envelope.commands.len(),
            bytes.len()
        );
    }
}
