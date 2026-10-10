use netrender::{Scene, SceneFragment, SceneOp, Transform};
use netrender::vello_backends::{CpuSession, SparseFragmentRegistry, VelloBackend};
use serde_json::json;
use std::{fs::File, path::Path, time::Instant};
use tabard::theme::registry::Mode;
use tabard_workshop::WorkshopState;

fn write_png(path: &Path, scene: &Scene, data: &[u8]) {
    let file = File::options().write(true).create_new(true).open(path).unwrap();
    let mut encoder = png::Encoder::new(file, scene.viewport_width, scene.viewport_height);
    encoder.set_color(png::ColorType::Rgba); encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header().unwrap().write_image_data(data).unwrap();
}

fn main() {
    let out = std::path::PathBuf::from(std::env::args_os().nth(1).expect("output directory"));
    std::fs::create_dir_all(&out).unwrap();
    let mut state = WorkshopState::in_memory();
    let mut cpu = CpuSession::new(Default::default());
    let mut ledger = SparseFragmentRegistry::new(Default::default());
    let mut ids = [None, None, None];
    let mut receipt = vec![];
    for (label, mode, size) in [("light", Mode::Light, (640, 400)), ("dark", Mode::Dark, (640, 400)),
        ("narrow", Mode::Dark, (320, 500)), ("authored", Mode::Light, (640, 400))] {
        state.set_mode(mode);
        if label == "authored" {
            *state.text_field_mut("mode-sheet").unwrap() = cambium::TextInput::new("body { background-color: rgb(17, 43, 79); color: rgb(221, 231, 241); } #preview-heading { color: rgb(243, 211, 117); }");
            state.apply_stylesheet();
        }
        let authority_before = format!("{:?}|{}|{}|{}|{}", state.draft_theme(), state.mode_key(), state.status(), state.is_dirty(), state.has_pending_fields());
        let reader = state.reader_preview(); let stylesheet = state.stylesheet_preview();
        let scenes = [reader.borrow_mut().frame(size.0, size.1), stylesheet.borrow_mut().frame(size.0, size.1)];
        assert!(stylesheet.borrow().diagnostics().is_empty());
        let names = [reader.borrow().accessible_name().to_owned(), stylesheet.borrow().accessible_name().to_owned()];
        let reader_source = scenes[0].clone();
        let mut stylesheet_pixels = None;
        for (index, (kind, scene)) in ["reader", "stylesheet"].into_iter().zip(scenes).enumerate() {
            let title = format!("{kind}-{label}");
            assert!(!scene.ops.is_empty());
            let glyphs = scene.ops.iter().filter_map(|op| if let SceneOp::GlyphRun(run) = op { Some(run.glyphs.len()) } else { None }).sum::<usize>();
            assert!(glyphs > 0, "real shaped text required");
            let start = Instant::now();
            let direct = cpu.render(&scene).unwrap_or_else(|e| panic!("{title}: {e}"));
            let direct_ms = start.elapsed().as_secs_f64() * 1000.;
            if index == 1 { stylesheet_pixels = Some(direct.data_as_u8_slice().to_vec()); }
            let before_images = cpu.stats(); let before_text = cpu.text_stats();
            let mut source = scene.clone(); source.root_alpha = 1.; source.root_blend_mode = netrender::SceneBlendMode::Normal;
            let source = SceneFragment::from_scene(source);
            let id = if let Some(id) = ids[index] { ledger.update_fragment(id, source).unwrap(); id }
                else { let id = ledger.register_fragment(source).unwrap(); ids[index] = Some(id); id };
            let mut frame = Scene::new(size.0, size.1); frame.root_alpha = scene.root_alpha; frame.root_blend_mode = scene.root_blend_mode;
            frame.place_fragment(id, Transform::IDENTITY);
            let expanded = ledger.resolve(&frame, VelloBackend::Cpu, Default::default()).unwrap();
            ledger.resolve(&frame, VelloBackend::Hybrid, Default::default()).unwrap();
            let start = Instant::now();
            let retained = cpu.render(&expanded).unwrap();
            let retained_ms = start.elapsed().as_secs_f64() * 1000.;
            assert_eq!(direct.data_as_u8_slice(), retained.data_as_u8_slice(), "{title}: retained source must preserve every pixel");
            assert_eq!(cpu.stats().hydrated_images, before_images.hydrated_images);
            assert_eq!(cpu.text_stats().cached_glyphs, before_text.cached_glyphs);
            let nonempty = direct.data_as_u8_slice().chunks_exact(4).filter(|p| p[3] > 0).count();
            assert!(nonempty > 100);
            write_png(&out.join(format!("{title}.png")), &scene, direct.data_as_u8_slice());
            // Local only: contains system font payloads; do not add to Git.
            std::fs::write(out.join(format!("{title}.scene")), scene.snapshot_postcard()).unwrap();
            receipt.push(json!({"label": title, "size": [size.0,size.1], "ops":scene.ops.len(),"glyphs":glyphs,"fonts":scene.fonts.len()-1,"images":scene.image_sources.len(),
                "accessible_name":names[index],"retained_pixel_identity":true,"fragment_generation":ledger.generation(id),"hybrid_source_admission":true,
                "gpu_initialized":false,"direct_cpu_ms":direct_ms,"retained_cpu_ms":retained_ms,"nonempty_pixels":nonempty}));
        }
        // Explicit CPU snapshot composition: an actual stylesheet preview's
        // raster becomes a CPU-owned image. This performs no GPU readback and
        // does not substitute pixels for missing whole-workshop capture resources.
        let pixels = stylesheet_pixels.unwrap();
        assert!(pixels.chunks_exact(4).all(|pixel| pixel[3] == 255));
        let image = netrender::ImageData::from_bytes(size.0, size.1, pixels);
        let mut image_source = Scene::new(size.0, size.1);
        image_source.push_image(0., 0., size.0 as f32, size.1 as f32, 0x534e4150, image.clone());
        let source = SceneFragment::from_scene(image_source);
        let image_id = if let Some(id) = ids[2] { ledger.update_fragment(id, source).unwrap(); id }
            else { let id = ledger.register_fragment(source).unwrap(); ids[2] = Some(id); id };
        let mut composed = Scene::new(size.0, size.1 * 2);
        composed.place_fragment(ids[0].unwrap(), Transform::IDENTITY);
        composed.place_fragment(image_id, Transform::translate_2d(0., size.1 as f32));
        let expanded = ledger.resolve(&composed, VelloBackend::Cpu, Default::default()).unwrap();
        ledger.resolve(&composed, VelloBackend::Hybrid, Default::default()).unwrap();
        let mut direct = Scene::new(size.0, size.1 * 2);
        direct.append_fragment(SceneFragment::from_scene(reader_source));
        direct.push_image(0., size.1 as f32, size.0 as f32, (size.1 * 2) as f32, 0x534e4150, image);
        let expected = cpu.render(&direct).unwrap();
        let before_images = cpu.stats();
        let actual = cpu.render(&expanded).unwrap();
        assert_eq!(actual.data_as_u8_slice(), expected.data_as_u8_slice());
        assert_eq!(cpu.stats().hydrated_images, before_images.hydrated_images);
        assert_eq!(cpu.stats().cached_images, 1);
        write_png(&out.join(format!("composed-{label}.png")), &expanded, actual.data_as_u8_slice());
        std::fs::write(out.join(format!("composed-{label}.scene")), expanded.snapshot_postcard()).unwrap();
        receipt.push(json!({"label":format!("composed-{label}"),"size":[size.0,size.1*2],"retained_pixel_identity":true,
            "image_source":"explicit opaque CPU snapshot of production stylesheet preview; no GPU readback",
            "images":expanded.image_sources.len(),"fonts":expanded.fonts.len()-1,"ops":expanded.ops.len(),
            "image_generation":ledger.generation(image_id),"cached_images":cpu.stats().cached_images,
            "hybrid_source_admission":true,"gpu_initialized":false}));
        let authority_after = format!("{:?}|{}|{}|{}|{}", state.draft_theme(), state.mode_key(), state.status(), state.is_dirty(), state.has_pending_fields());
        assert_eq!(authority_before, authority_after, "rendering must preserve authored state");
    }
    let output = json!({"scope":"production Tabard reader and stylesheet preview components; whole workshop external-texture capture remains refused", "gpu_initialized":false,
        "application_authority_preserved":true,"retention":"source expansion and re-lowering, no packet cache","frames":receipt});
    std::fs::write(out.join("receipt.json"), serde_json::to_vec_pretty(&output).unwrap()).unwrap();
    println!("{}", output);
}
