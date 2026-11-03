// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

use super::*;

/// Composing layers with no side-tables (the orrery underlay + a doc, today)
/// concatenates command streams back-to-front into one scene.
#[test]
fn commands_only_layers_concatenate_into_one_scene() {
    let underlay = vec![PaintCmd::DrawRect(RectItem {
        placement: placement_at(box2d(0.0, 0.0, 10.0, 10.0)),
        color: ColorF {
            r: 1.0,
            g: 0.0,
            b: 0.0,
            a: 1.0,
        },
    })];
    let doc = vec![PaintCmd::DrawRect(RectItem {
        placement: placement_at(box2d(5.0, 5.0, 10.0, 10.0)),
        color: ColorF {
            r: 0.0,
            g: 0.0,
            b: 1.0,
            a: 1.0,
        },
    })];
    let layers = [
        CompositeLayer::commands_only(&underlay),
        CompositeLayer::commands_only(&doc),
    ];
    let out = composite_paint_layers(DeviceIntSize::new(800, 600), &layers)
        .expect("translate composited layers");
    let rects = out
        .scene
        .ops
        .iter()
        .filter(|o| matches!(o, netrender::SceneOp::Rect(_)))
        .count();
    assert_eq!(rects, 2, "both layers' rects composite into one scene");
}

/// Two layers that BOTH mint a font under the default namespace+index (the
/// collision producers fall into) must come out with distinct keys, and each
/// layer's `DrawText` rewritten to its own remapped key — otherwise the
/// translator's `FontInstanceKey -> FontId` map clobbers one font with the
/// other. (Pins the font/image key-namespacing the seam exists for.)
#[test]
fn merge_namespaces_colliding_font_keys() {
    use paint_list_api::{FontInstanceKey, FontResource, IdNamespace, TextOptions, TextRunItem};

    let collide = FontInstanceKey::new(IdNamespace(0), 0);
    let mk = |face: u8| {
        let fonts = vec![FontResource {
            key: collide,
            data: std::sync::Arc::new(vec![face]),
            index: 0,
        }];
        let cmds = vec![PaintCmd::DrawText(TextRunItem {
            placement: placement_at(box2d(0.0, 0.0, 10.0, 10.0)),
            font_instance: collide,
            font_size: 16.0,
            color: ColorF::BLACK,
            glyphs: vec![],
            options: TextOptions::default(),
        })];
        (fonts, cmds)
    };
    let (f0, c0) = mk(0xAA);
    let (f1, c1) = mk(0xBB);
    let layers = [
        CompositeLayer {
            commands: &c0,
            fonts: &f0,
            images: &[],
        },
        CompositeLayer {
            commands: &c1,
            fonts: &f1,
            images: &[],
        },
    ];
    let (commands, fonts, images) = merge_layers(&layers);

    assert!(images.is_empty());
    assert_eq!(fonts.len(), 2, "both fonts survive the merge");
    assert_ne!(fonts[0].key, fonts[1].key, "colliding keys made distinct");
    assert_eq!(
        *fonts[0].data,
        vec![0xAA],
        "font bytes paired to the right new key"
    );
    assert_eq!(*fonts[1].data, vec![0xBB]);
    let text_keys: Vec<_> = commands
        .iter()
        .filter_map(|c| match c {
            PaintCmd::DrawText(t) => Some(t.font_instance),
            _ => None,
        })
        .collect();
    assert_eq!(
        text_keys,
        vec![fonts[0].key, fonts[1].key],
        "each layer's DrawText references its own remapped font",
    );
}

#[test]
fn composite_images_remap_keys_without_copying_pixels() {
    use std::sync::Arc;
    use paint_list_api::{AlphaType, IdNamespace, ImageItem, ImageRendering, ImageResource};

    let collide = ImageKey::new(IdNamespace(0), 0);
    let pixels_a = Arc::new(vec![255, 0, 0, 255]);
    let pixels_b = Arc::new(vec![0, 0, 255, 255]);
    let mk = |pixels: Arc<Vec<u8>>| {
        let images = vec![ImageResource {
            key: collide,
            width: 1,
            height: 1,
            data: pixels,
        }];
        let commands = vec![PaintCmd::DrawImage(ImageItem {
            placement: placement_at(box2d(0.0, 0.0, 1.0, 1.0)),
            image_key: collide,
            image_rendering: ImageRendering::Auto,
            alpha_type: AlphaType::Alpha,
            color: ColorF::WHITE,
        })];
        (images, commands)
    };
    let (images_a, commands_a) = mk(pixels_a.clone());
    let (images_b, commands_b) = mk(pixels_b.clone());
    let layers = [
        CompositeLayer {
            commands: &commands_a,
            fonts: &[],
            images: &images_a,
        },
        CompositeLayer {
            commands: &commands_b,
            fonts: &[],
            images: &images_b,
        },
    ];

    let (commands, _, images) = merge_layers(&layers);
    assert_ne!(images[0].key, images[1].key);
    assert!(Arc::ptr_eq(&images[0].data, &pixels_a));
    assert!(Arc::ptr_eq(&images[1].data, &pixels_b));
    let command_keys: Vec<_> = commands
        .iter()
        .filter_map(|cmd| match cmd {
            PaintCmd::DrawImage(item) => Some(item.image_key),
            _ => None,
        })
        .collect();
    assert_eq!(command_keys, vec![images[0].key, images[1].key]);

    let translated = composite_paint_layers(DeviceIntSize::new(2, 1), &layers)
        .expect("translate composited layers");
    let scene_keys: Vec<_> = translated
        .scene
        .ops
        .iter()
        .filter_map(|op| match op {
            netrender::SceneOp::Image(image) => Some(image.key),
            _ => None,
        })
        .collect();
    assert_eq!(scene_keys.len(), 2);
    assert_ne!(scene_keys[0], scene_keys[1]);
    assert!(std::ptr::eq(
        translated.scene.image_sources[&scene_keys[0]]
            .data
            .data()
            .as_ptr(),
        pixels_a.as_ptr(),
    ));
    assert!(std::ptr::eq(
        translated.scene.image_sources[&scene_keys[1]]
            .data
            .data()
            .as_ptr(),
        pixels_b.as_ptr(),
    ));
}

#[test]
fn retained_fragment_keeps_image_pixel_allocation() {
    use std::sync::Arc;
    use paint_list_api::{IdNamespace, ImageResource};

    let pixels = Arc::new(vec![12, 34, 56, 255]);
    let image = ImageResource {
        key: ImageKey::new(IdNamespace(1), 4),
        width: 1,
        height: 1,
        data: pixels.clone(),
    };
    let fragment =
        translate_paint_cmds_to_fragment(&[], &[], &[image]).expect("translate image fragment");
    assert_eq!(fragment.image_sources.len(), 1);
    let source = fragment.image_sources.values().next().unwrap();
    assert_eq!((source.width, source.height), (1, 1));
    assert_eq!(source.data.len(), 4);
    assert!(std::ptr::eq(source.data.data().as_ptr(), pixels.as_ptr()));
}

#[test]
#[cfg(feature = "replay-cpu")]
fn shared_image_payload_rasters_distinct_opaque_and_translucent_pixels() {
    use std::sync::Arc;
    use paint_list_api::{AlphaType, IdNamespace, ImageItem, ImageRendering, ImageResource};

    let key = ImageKey::new(IdNamespace(1), 9);
    let pixels = Arc::new(vec![255, 0, 0, 255, 0, 0, 255, 128]);
    let image = ImageResource {
        key,
        width: 2,
        height: 1,
        data: pixels.clone(),
    };
    let command = PaintCmd::DrawImage(ImageItem {
        placement: placement_at(box2d(0.0, 0.0, 2.0, 1.0)),
        image_key: key,
        image_rendering: ImageRendering::Pixelated,
        alpha_type: AlphaType::Alpha,
        color: ColorF::WHITE,
    });
    let scene = translate_paint_cmd_stream(DeviceIntSize::new(2, 1), &[command], &[], &[image])
        .expect("translate image command")
        .scene;
    let source = scene
        .image_sources
        .values()
        .next()
        .expect("registered source");
    assert!(std::ptr::eq(source.data.data().as_ptr(), pixels.as_ptr()));

    let mut cpu = netrender::vello_backends::CpuSession::new(
        netrender::vello_backends::SparseResourceLimits::default(),
    );
    let rendered = cpu.render(&scene).expect("rasterize translated image");
    assert_eq!(
        rendered.data_as_u8_slice(),
        &[255, 0, 0, 255, 0, 0, 128, 128],
        "CPU output is premultiplied RGBA8"
    );
}
