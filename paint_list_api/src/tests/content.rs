use super::*;
use std::sync::Arc;

fn content() -> PaintEnvelope {
    let placement = CommonPlacement::new(box2d(1.0, 2.0, 40.0, 30.0));
    let path = PathData {
        commands: vec![
            PathCommand::MoveTo(LayoutPoint::new(0.0, 0.0)),
            PathCommand::QuadTo {
                control: LayoutPoint::new(5.0, 10.0),
                to: LayoutPoint::new(20.0, 0.0),
            },
            PathCommand::Close,
        ],
    };
    let font_key = FontInstanceKey::new(IdNamespace(1), 2);
    let image_key = ImageKey::new(IdNamespace(3), 4);
    PaintEnvelope {
        engine: EngineId::GENET,
        viewport: DeviceIntSize::new(80, 60),
        generation: 7,
        commands: vec![
            PaintCmd::PushTransform(TransformSpec {
                origin: LayoutPoint::new(1.0, 2.0),
                transform: LayoutTransform::identity(),
                kind: TransformKind::Standard,
            }),
            PaintCmd::PushClip(ClipSpec {
                kind: ClipKind::Path(path.clone()),
            }),
            PaintCmd::PushLayer(LayerSpec {
                opacity: 0.75,
                filters: vec![FilterOp::Blur(1.0)],
                mask: Some(LayerMask {
                    mask_rect: placement.bounds,
                    image_mask: Some(image_key),
                }),
                ..Default::default()
            }),
            PaintCmd::DrawPath(PathItem {
                placement,
                path,
                fill: Some(ColorF::BLACK),
                stroke: None,
            }),
            PaintCmd::DrawText(TextRunItem {
                placement,
                font_instance: font_key,
                font_size: 12.0,
                color: ColorF::WHITE,
                glyphs: vec![GlyphInstance {
                    index: 8,
                    point: LayoutPoint::new(2.0, 12.0),
                }],
                options: TextOptions::default(),
            }),
            PaintCmd::DrawImage(ImageItem {
                placement,
                image_key,
                image_rendering: ImageRendering::Auto,
                alpha_type: AlphaType::Alpha,
                color: ColorF::WHITE,
            }),
            PaintCmd::DrawExternalTexture(ExternalTextureItem {
                placement,
                texture_key: 17,
                opacity: 1.0,
                content_generation: Some(23),
            }),
            PaintCmd::PopLayer,
            PaintCmd::PopClip,
            PaintCmd::PopTransform,
        ],
        fonts: vec![FontResource {
            key: font_key,
            data: Arc::new(vec![1, 2, 3, 4]),
            index: 0,
        }],
        images: vec![ImageResource {
            key: image_key,
            width: 2,
            height: 1,
            data: Arc::new(vec![255, 0, 0, 255, 0, 255, 0, 128]),
        }],
    }
}

fn changed(base: &PaintEnvelope, label: &str, edit: impl FnOnce(&mut PaintEnvelope)) {
    let mut other = base.clone();
    edit(&mut other);
    assert!(!base.same_content(&other), "omitted {label}");
    assert!(!other.same_content(base), "asymmetric {label}");
}

#[test]
fn paint_content_ignores_only_publication_generation() {
    let base = content();
    let mut other = base.clone();
    other.generation = u64::MAX;
    assert_ne!(base, other);
    assert!(base.same_content(&other));
    assert!(other.same_content(&base));
    other.commands.pop();
    assert!(!base.same_content(&other));

    // An unchanged generation does not assert content equality for this API.
    changed(&base, "engine", |v| v.engine = EngineId::INKER);
    changed(&base, "viewport width", |v| v.viewport.width += 1);
    changed(&base, "viewport height", |v| v.viewport.height += 1);
    changed(&base, "command order", |v| v.commands.swap(3, 4));
    changed(&base, "additional command", |v| {
        v.commands.push(PaintCmd::PopClip)
    });
}

#[test]
fn paint_content_tracks_shape_transform_clip_and_layer_state() {
    let base = content();
    changed(&base, "transform matrix", |v| {
        let PaintCmd::PushTransform(t) = &mut v.commands[0] else {
            panic!()
        };
        t.transform.m41 = 4.0;
    });
    changed(&base, "clip path", |v| {
        let PaintCmd::PushClip(ClipSpec {
            kind: ClipKind::Path(p),
        }) = &mut v.commands[1]
        else {
            panic!()
        };
        p.commands[1] = PathCommand::LineTo(LayoutPoint::new(20.0, 0.0));
    });
    changed(&base, "layer opacity", |v| {
        let PaintCmd::PushLayer(layer) = &mut v.commands[2] else {
            panic!()
        };
        layer.opacity = 0.5;
    });
    changed(&base, "filter", |v| {
        let PaintCmd::PushLayer(layer) = &mut v.commands[2] else {
            panic!()
        };
        layer.filters[0] = FilterOp::Blur(2.0);
    });
    changed(&base, "mask", |v| {
        let PaintCmd::PushLayer(layer) = &mut v.commands[2] else {
            panic!()
        };
        layer.mask.as_mut().unwrap().mask_rect.max.x += 1.0;
    });
    changed(&base, "shape control point", |v| {
        let PaintCmd::DrawPath(item) = &mut v.commands[3] else {
            panic!()
        };
        let PathCommand::QuadTo { control, .. } = &mut item.path.commands[1] else {
            panic!()
        };
        control.y += 1.0;
    });
    changed(&base, "placement flags", |v| {
        let PaintCmd::DrawPath(item) = &mut v.commands[3] else {
            panic!()
        };
        item.placement.flags = PrimitiveFlags::ANTIALIASED;
    });
}

#[test]
fn paint_content_tracks_text_sampling_and_external_content_epoch() {
    let base = content();
    changed(&base, "glyph position", |v| {
        let PaintCmd::DrawText(item) = &mut v.commands[4] else {
            panic!()
        };
        item.glyphs[0].point.x += 1.0;
    });
    changed(&base, "text render mode", |v| {
        let PaintCmd::DrawText(item) = &mut v.commands[4] else {
            panic!()
        };
        item.options.render_mode = FontRenderMode::Mono;
    });
    changed(&base, "image sampling", |v| {
        let PaintCmd::DrawImage(item) = &mut v.commands[5] else {
            panic!()
        };
        item.image_rendering = ImageRendering::Pixelated;
    });
    changed(&base, "external generation", |v| {
        let PaintCmd::DrawExternalTexture(item) = &mut v.commands[6] else {
            panic!()
        };
        item.content_generation = Some(24);
    });
    changed(&base, "external identity", |v| {
        let PaintCmd::DrawExternalTexture(item) = &mut v.commands[6] else {
            panic!()
        };
        item.texture_key += 1;
    });
}

#[test]
fn paint_content_includes_every_font_and_image_resource_field() {
    let base = content();
    changed(&base, "font key", |v| v.fonts[0].key.1 += 1);
    changed(&base, "font collection index", |v| v.fonts[0].index += 1);
    changed(&base, "font bytes", |v| {
        Arc::make_mut(&mut v.fonts[0].data)[0] ^= 1
    });
    changed(&base, "removed font", |v| v.fonts.clear());
    changed(&base, "image key", |v| v.images[0].key.1 += 1);
    changed(&base, "image width", |v| v.images[0].width += 1);
    changed(&base, "image height", |v| v.images[0].height += 1);
    changed(&base, "image pixels", |v| {
        Arc::make_mut(&mut v.images[0].data)[0] ^= 1
    });
    changed(&base, "removed image", |v| v.images.clear());
    changed(&base, "extra resource", |v| {
        v.images.push(v.images[0].clone())
    });
}

#[test]
fn paint_content_resource_clones_share_storage_without_aliasing_changes() {
    let base = content();
    let mut other = base.clone();
    assert!(Arc::ptr_eq(&base.fonts[0].data, &other.fonts[0].data));
    assert!(Arc::ptr_eq(&base.images[0].data, &other.images[0].data));
    assert!(base.same_content(&other));

    other.fonts[0].data = Arc::new((*base.fonts[0].data).clone());
    other.images[0].data = Arc::new((*base.images[0].data).clone());
    assert!(!Arc::ptr_eq(&base.fonts[0].data, &other.fonts[0].data));
    assert!(!Arc::ptr_eq(&base.images[0].data, &other.images[0].data));
    assert!(base.same_content(&other));
    Arc::make_mut(&mut other.images[0].data)[3] = 0;
    assert_eq!(base.images[0].data[3], 255);
    assert!(!base.same_content(&other));
}

#[test]
fn paint_content_survives_capture_roundtrip_and_refuses_nan_fields() {
    let base = content();
    let wire = serde_json::to_vec(&base).unwrap();
    let captured: PaintEnvelope = serde_json::from_slice(&wire).unwrap();
    assert!(base.same_content(&captured));
    let mut invalid = base;
    let PaintCmd::DrawText(item) = &mut invalid.commands[4] else {
        panic!()
    };
    item.font_size = f32::NAN;
    assert!(!invalid.same_content(&invalid.clone()));
}
