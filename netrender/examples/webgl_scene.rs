// Copyright 2026 Mark Alan Boykin
// SPDX-License-Identifier: MPL-2.0

//! Browser semantic fixtures; no Graphshell or hardware-conformance claim.
#[cfg(target_arch = "wasm32")]
mod browser {
    use netrender::{
        Scene, SceneLayer, SceneFilter, SceneOp, SceneClip, ImageData, Glyph, FontBlob,
        peniko::Blob,
    };
    use netrender::vello_backends::{WebGlSession, WebGlSessionInit, SparseResourceLimits};
    use skrifa::MetadataProvider;
    use std::sync::Arc;
    use wasm_bindgen::prelude::*;
    use web_sys::{HtmlCanvasElement, WebGl2RenderingContext as Gl};
    const FONT: &[u8] = include_bytes!("../tests/fixtures/vb3_fonts/Inter.var.subset.ttf");
    fn js(error: impl std::fmt::Display) -> JsValue {
        JsValue::from_str(&error.to_string())
    }

    fn case_scene(name: &str, w: u32, h: u32) -> Result<Scene, JsValue> {
        let mut scene = Scene::new(w, h);
        match name {
            "red" => scene.push_rect(0., 0., w as f32, h as f32, [1., 0., 0., 1.]),
            "green" => scene.push_rect(0., 0., w as f32, h as f32, [0., 1., 0., 1.]),
            "transparent" => scene.push_rect(0., 0., w as f32, h as f32, [0., 0., 0.5, 0.5]),
            "mixed" => {
                scene.push_rect(0., 0., w as f32, h as f32, [0., 0., 1., 1.]);
                scene.push_rect(0., 0., 32., 32., [1., 0., 0., 1.]);
                scene
                    .ops
                    .push(SceneOp::PushLayer(SceneLayer::clip(SceneClip::Rect {
                        rect: [32., 0., 64., 32.],
                        radii: [0.; 4],
                    })));
                scene.ops.push(SceneOp::PushLayer(SceneLayer::alpha(0.5)));
                scene.push_rect(0., 0., w as f32, h as f32, [0., 1., 0., 1.]);
                scene.ops.push(SceneOp::PopLayer);
                scene.ops.push(SceneOp::PopLayer);
                scene.image_sources.insert(
                    71,
                    ImageData::from_bytes(
                        2,
                        2,
                        vec![
                            255, 0, 0, 128, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 0,
                        ],
                    ),
                );
                scene.push_image_full(
                    8.,
                    40.,
                    40.,
                    72.,
                    [0., 0., 1., 1.],
                    [1.; 4],
                    71,
                    0,
                    netrender::NO_CLIP,
                );
                let font = scene.push_font(FontBlob {
                    data: Blob::new(Arc::new(FONT.to_vec())),
                    index: 0,
                });
                let glyph = skrifa::FontRef::new(FONT)
                    .map_err(js)?
                    .charmap()
                    .map('a' as u32)
                    .ok_or_else(|| js("trusted fixture is missing its lowercase a glyph"))?
                    .to_u32();
                scene.push_glyph_run(
                    font,
                    48.,
                    vec![Glyph {
                        id: glyph,
                        x: 70.,
                        y: 110.,
                    }],
                    [1., 1., 0., 1.],
                );
            }
            "late-filter" => {
                scene.push_rect(0., 0., w as f32, h as f32, [1., 0., 0., 1.]);
                let mut layer = SceneLayer::alpha(1.);
                layer.filters.push(SceneFilter::Blur(2.));
                scene.ops.push(SceneOp::PushLayer(layer));
                scene.push_rect(0., 0., 8., 8., [0., 1., 0., 1.]);
                scene.ops.push(SceneOp::PopLayer);
            }
            _ => return Err(js("unknown fixture case")),
        }
        Ok(scene)
    }

    #[wasm_bindgen]
    pub struct WebGlFixture {
        canvas: HtmlCanvasElement,
        init: Option<WebGlSessionInit>,
        session: Option<WebGlSession>,
    }
    #[wasm_bindgen]
    impl WebGlFixture {
        #[wasm_bindgen(constructor)]
        pub fn new(canvas: HtmlCanvasElement, generation: u64) -> Result<WebGlFixture, JsValue> {
            let init = WebGlSession::start_with_generation(
                &canvas,
                SparseResourceLimits::default(),
                generation,
            )
            .map_err(js)?;
            Ok(Self {
                canvas,
                init: Some(init),
                session: None,
            })
        }
        pub fn poll(&mut self) -> Result<bool, JsValue> {
            if self.session.is_some() {
                return Ok(true);
            }
            self.session = self
                .init
                .as_mut()
                .ok_or_else(|| js("initialization consumed"))?
                .try_finish()
                .map_err(js)?;
            if self.session.is_some() {
                self.init = None;
            }
            Ok(self.session.is_some())
        }
        pub fn render_case(&mut self, name: &str, generation: u64) -> Result<(), JsValue> {
            let scene = case_scene(name, self.canvas.width(), self.canvas.height())?;
            self.session
                .as_mut()
                .ok_or_else(|| js("fixture not ready"))?
                .render(&scene, generation)
                .map_err(js)
        }
        // The host resizes only after complete recording succeeds.
        pub fn render_resized_case(
            &mut self,
            name: &str,
            w: u32,
            h: u32,
            generation: u64,
        ) -> Result<(), JsValue> {
            let scene = case_scene(name, w, h)?;
            let prepared = self
                .session
                .as_mut()
                .ok_or_else(|| js("fixture not ready"))?
                .prepare(&scene, generation, [0.; 4])
                .map_err(js)?;
            self.canvas.set_width(w);
            self.canvas.set_height(h);
            prepared.present().map_err(js)
        }
        pub fn set_solid_image(&mut self, key: u64, rgba: &[u8]) -> Result<(), JsValue> {
            if rgba.len() != 4 {
                return Err(js("solid image requires four RGBA bytes"));
            }
            self.session
                .as_mut()
                .ok_or_else(|| js("fixture not ready"))?
                .set_image_source(key, ImageData::from_bytes(1, 1, rgba.to_vec()))
                .map_err(js)
        }
        pub fn render_images(
            &mut self,
            include_first: bool,
            generation: u64,
        ) -> Result<(), JsValue> {
            let mut scene = Scene::new(self.canvas.width(), self.canvas.height());
            scene.push_rect(
                0.,
                0.,
                self.canvas.width() as f32,
                self.canvas.height() as f32,
                [0., 0., 0., 1.],
            );
            if include_first {
                scene.push_image_full(
                    0.,
                    0.,
                    16.,
                    16.,
                    [0., 0., 1., 1.],
                    [1.; 4],
                    71,
                    0,
                    netrender::NO_CLIP,
                );
            }
            scene.push_image_full(
                16.,
                0.,
                32.,
                16.,
                [0., 0., 1., 1.],
                [1.; 4],
                72,
                0,
                netrender::NO_CLIP,
            );
            self.session
                .as_mut()
                .ok_or_else(|| js("fixture not ready"))?
                .render(&scene, generation)
                .map_err(js)
        }
        pub fn uploaded_images(&self) -> Result<u64, JsValue> {
            Ok(self
                .session
                .as_ref()
                .ok_or_else(|| js("fixture not ready"))?
                .stats()
                .uploaded_images)
        }
        pub fn stats(&self) -> Result<String, JsValue> {
            Ok(format!(
                "{:?}",
                self.session
                    .as_ref()
                    .ok_or_else(|| js("fixture not ready"))?
                    .stats()
            ))
        }
        pub fn pixels(&self) -> Result<Vec<u8>, JsValue> {
            let session = self
                .session
                .as_ref()
                .ok_or_else(|| js("fixture not ready"))?;
            let gl = session.gl_context();
            gl.bind_framebuffer(Gl::READ_FRAMEBUFFER, None);
            let mut rgba =
                vec![0; self.canvas.width() as usize * self.canvas.height() as usize * 4];
            gl.read_pixels_with_opt_u8_array(
                0,
                0,
                self.canvas.width() as i32,
                self.canvas.height() as i32,
                Gl::RGBA,
                Gl::UNSIGNED_BYTE,
                Some(&mut rgba),
            )?;
            Ok(rgba)
        }
        pub fn renderer(&self) -> Result<String, JsValue> {
            let gl = self
                .session
                .as_ref()
                .ok_or_else(|| js("fixture not ready"))?
                .gl_context();
            let _ = gl.get_extension("WEBGL_debug_renderer_info");
            Ok(gl.get_parameter(0x9246)?.as_string().unwrap_or_default())
        }
        pub fn remove_image(&mut self, key: u64) -> bool {
            self.session
                .as_mut()
                .is_some_and(|session| session.remove_image_source(key))
        }
    }
}
