use super::*;

mod content;

/// Trivial PaintList impl for trait-bound and serialization tests.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
struct StubPaintList {
    viewport: DeviceIntSize,
    commands: Vec<PaintCmd>,
    generation: u64,
}

impl PaintList for StubPaintList {
    fn engine_id(&self) -> EngineId {
        EngineId::UNASSIGNED
    }
    fn viewport(&self) -> DeviceIntSize {
        self.viewport
    }
    fn generation_id(&self) -> u64 {
        self.generation
    }
    fn commands(&self) -> &[PaintCmd] {
        &self.commands
    }
}

fn box2d(x: f32, y: f32, w: f32, h: f32) -> LayoutRect {
    LayoutRect::new(LayoutPoint::new(x, y), LayoutPoint::new(x + w, y + h))
}

#[test]
fn primitive_flags_or_combines() {
    let f = PrimitiveFlags::HIT_TESTABLE | PrimitiveFlags::ANTIALIASED;
    assert!(f.contains(PrimitiveFlags::HIT_TESTABLE));
    assert!(f.contains(PrimitiveFlags::ANTIALIASED));
    assert!(!f.contains(PrimitiveFlags::IS_BACKFACE));
}

#[test]
fn stub_paint_list_satisfies_trait_bounds() {
    // Sized usage: this is the canonical dispatch shape. The trait
    // isn't `dyn`-compatible (Clone + Serialize bounds aren't
    // object-safe); engine-agnostic dispatch goes through the
    // closed-set envelope downstream.
    fn assert_paint_list<L: PaintList>(_: &L) {}
    let list = StubPaintList::default();
    assert_paint_list(&list);
}

#[test]
fn paint_cmd_round_trips_through_json() {
    // serde+derive being wired correctly is enough to validate the
    // command surface. We round-trip through serde_json which only
    // needs Serialize + Deserialize impls; if any item or spec is
    // missing a derive, this fails to compile or to deserialize.
    let cmd = PaintCmd::DrawRect(RectItem {
        placement: CommonPlacement::new(box2d(0.0, 0.0, 100.0, 50.0)),
        color: ColorF::default(),
    });
    let serialized = serde_json::to_string(&cmd).expect("serialize");
    let parsed: PaintCmd = serde_json::from_str(&serialized).expect("deserialize");
    match parsed {
        PaintCmd::DrawRect(_) => {}
        other => panic!("round-trip lost variant: {other:?}"),
    }
}

#[test]
fn external_texture_content_generation_defaults_none() {
    // The PM-3 forward-looking field defaults to None; producers
    // set it only when texture-as-source rather than compositor-
    // pass. Pin the default so downstream lowering tests can rely
    // on it.
    let item = ExternalTextureItem {
        placement: CommonPlacement::new(box2d(0.0, 0.0, 200.0, 200.0)),
        texture_key: 0xDEADBEEF,
        opacity: 1.0,
        content_generation: None,
    };
    assert_eq!(item.content_generation, None);
}

#[test]
fn paint_envelope_preserves_list_fields() {
    let viewport = DeviceIntSize::new(800, 600);
    let stub = StubPaintList {
        viewport,
        commands: vec![
            PaintCmd::DrawRect(RectItem {
                placement: CommonPlacement::new(box2d(0.0, 0.0, 100.0, 50.0)),
                color: ColorF::default(),
            }),
            PaintCmd::PopLayer,
        ],
        generation: 42,
    };
    let envelope = PaintEnvelope::from_list(&stub);
    assert_eq!(envelope.engine_id(), EngineId::UNASSIGNED);
    assert_eq!(envelope.viewport(), viewport);
    assert_eq!(envelope.generation_id(), 42);
    assert_eq!(envelope.commands().len(), 2);
}

#[test]
fn paint_envelope_round_trips_through_serde() {
    let envelope = PaintEnvelope {
        engine: EngineId::GENET,
        viewport: DeviceIntSize::new(1024, 768),
        generation: 7,
        commands: vec![PaintCmd::PopLayer],
        fonts: Vec::new(),
        images: Vec::new(),
    };
    let json = serde_json::to_string(&envelope).expect("serialize");
    let parsed: PaintEnvelope = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(parsed.engine, EngineId::GENET);
    assert_eq!(parsed.viewport, envelope.viewport);
    assert_eq!(parsed.generation, 7);
    assert_eq!(parsed.commands.len(), 1);
}

#[test]
fn image_resource_clone_shares_pixels_and_keeps_wire_shape() {
    use std::sync::Arc;

    let pixels = Arc::new(vec![255, 0, 0, 255, 0, 255, 0, 255]);
    let image = ImageResource {
        key: ImageKey::new(IdNamespace(3), 7),
        width: 2,
        height: 1,
        data: pixels.clone(),
    };
    assert_eq!(
        image.data.len(),
        image.width as usize * image.height as usize * 4
    );
    assert!(Arc::ptr_eq(&image.data, &image.clone().data));

    let json = serde_json::to_value(&image).expect("serialize image");
    assert_eq!(
        json["data"],
        serde_json::json!([255, 0, 0, 255, 0, 255, 0, 255])
    );
    let parsed: ImageResource = serde_json::from_value(json).expect("deserialize image");
    assert_eq!(parsed.key, image.key);
    assert_eq!((parsed.width, parsed.height), (2, 1));
    assert_eq!(&*parsed.data, &*pixels);
}

#[test]
fn engine_id_sentinels_are_stable() {
    // These values cross IPC; renumbering them is a wire-break.
    assert_eq!(EngineId::GENET.0, 0);
    assert_eq!(EngineId::NEMATIC.0, 1);
    assert_eq!(EngineId::SCRYING.0, 2);
    assert_eq!(EngineId::INKER.0, 3);
}
