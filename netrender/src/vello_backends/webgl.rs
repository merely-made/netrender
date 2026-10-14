// Copyright 2026 Mark Alan Boykin
// SPDX-License-Identifier: MPL-2.0

//! Direct WebGL2 execution of the shared admitted sparse scene.
//! Image handles are created here on this context; callers supply CPU sources.
use super::*;
use vello_hybrid::{WebGlRenderer, WebGlRendererInit, WebGlRendererInitStatus, WebGlTextureBindings};
use vello_sparse_common::{
    TextureId,
    geometry::RectU16,
    peniko::color::{AlphaColor, Srgb},
};
use web_sys::{HtmlCanvasElement, WebGl2RenderingContext as Gl, WebGlTexture};

#[derive(Debug)]
pub enum WebGlSessionError {
    Sparse(SparseSessionError),
    Provider(String),
    ContextLost,
    StaleGeneration {
        expected: u64,
        actual: u64,
    },
    InvalidGeneration,
    /// This session accepts owned CPU source data, never a foreign GPU handle.
    ExternalTextureImportUnsupported,
}
impl std::fmt::Display for WebGlSessionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "WebGL session: {self:?}")
    }
}
impl std::error::Error for WebGlSessionError {}
impl From<SparseSessionError> for WebGlSessionError {
    fn from(error: SparseSessionError) -> Self {
        Self::Sparse(error)
    }
}
fn provider(error: impl std::fmt::Display) -> WebGlSessionError {
    WebGlSessionError::Provider(error.to_string())
}

pub struct WebGlSessionInit {
    init: Option<WebGlRendererInit>,
    resources: Option<vello_hybrid::Resources>,
    limits: SparseResourceLimits,
    generation: u64,
}
impl WebGlSessionInit {
    /// Poll once per animation turn; shader compilation can continue between calls.
    pub fn try_finish(&mut self) -> Result<Option<WebGlSession>, WebGlSessionError> {
        let Some(init) = self.init.take() else {
            return Ok(None);
        };
        match init.try_finish().map_err(provider)? {
            WebGlRendererInitStatus::Pending(init) => {
                self.init = Some(init);
                Ok(None)
            }
            WebGlRendererInitStatus::Complete(renderer) => {
                let dimension = renderer
                    .gl_context()
                    .get_parameter(Gl::MAX_TEXTURE_SIZE)
                    .map_err(|e| provider(format!("MAX_TEXTURE_SIZE: {e:?}")))?
                    .as_f64()
                    .ok_or_else(|| provider("MAX_TEXTURE_SIZE is not numeric"))?
                    as u32;
                Ok(Some(WebGlSession {
                    state: ImageState::new(
                        self.limits,
                        dimension.min(u16::MAX as u32).saturating_sub(1),
                    ),
                    text: text::TextState::new(SparseTextLimits::default()),
                    renderer,
                    resources: self.resources.take().expect("init owns resources"),
                    textures: HashMap::new(),
                    generation: self.generation,
                }))
            }
        }
    }
}

pub struct WebGlSession {
    state: ImageState,
    text: text::TextState,
    renderer: WebGlRenderer,
    resources: vello_hybrid::Resources,
    textures: HashMap<VariantKey, WebGlTexture>,
    generation: u64,
}
impl WebGlSession {
    pub fn start(
        canvas: &HtmlCanvasElement,
        limits: SparseResourceLimits,
    ) -> Result<WebGlSessionInit, WebGlSessionError> {
        Self::start_with_generation(canvas, limits, 1)
    }
    pub fn start_with_generation(
        canvas: &HtmlCanvasElement,
        limits: SparseResourceLimits,
        generation: u64,
    ) -> Result<WebGlSessionInit, WebGlSessionError> {
        if generation == 0 {
            return Err(WebGlSessionError::InvalidGeneration);
        }
        let (init, resources) = WebGlRenderer::begin(canvas).map_err(provider)?;
        Ok(WebGlSessionInit {
            init: Some(init),
            resources: Some(resources),
            limits,
            generation,
        })
    }
    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub fn gl_context(&self) -> &Gl {
        self.renderer.gl_context()
    }
    fn check_generation(&self, generation: u64) -> Result<(), WebGlSessionError> {
        if generation != self.generation {
            return Err(WebGlSessionError::StaleGeneration {
                expected: self.generation,
                actual: generation,
            });
        }
        if self.gl_context().is_context_lost() {
            return Err(WebGlSessionError::ContextLost);
        }
        Ok(())
    }
    pub fn set_image_source(
        &mut self,
        key: ImageKey,
        data: ImageData,
    ) -> Result<(), WebGlSessionError> {
        self.state.set_image_source(key, data)?;
        self.retire_unused_textures();
        Ok(())
    }
    pub fn remove_image_source(&mut self, key: ImageKey) -> bool {
        let removed = self.state.remove_image_source(key);
        self.retire_unused_textures();
        removed
    }
    pub fn clear_images(&mut self) {
        self.state.clear_images();
        self.retire_unused_textures();
    }
    pub fn stats(&self) -> SparseResourceStats {
        let mut stats = self.state.stats();
        stats.gpu_texture_bytes = self
            .textures
            .keys()
            .map(|key| padded_bytes(*key).expect("admitted upload size"))
            .sum();
        stats
    }
    pub fn text_stats(&self) -> SparseTextStats {
        self.text.stats()
    }
    pub fn clear_text_cache(&mut self) {
        self.text.clear()
    }
    fn retire_unused_textures(&mut self) {
        let gl = self.renderer.gl_context();
        self.textures.retain(|key, texture| {
            if self.state.cache.contains_key(key) {
                true
            } else {
                gl.delete_texture(Some(texture));
                false
            }
        });
    }
    /// Full resource/parameter admission without target mutation. Hosts can use
    /// it before changing canvas dimensions; render repeats the same checks.
    pub fn validate_scene(&self, scene: &Scene, generation: u64) -> Result<(), WebGlSessionError> {
        self.check_generation(generation)?;
        let plan = self.state.plan(scene, VelloBackend::WebGl2)?;
        self.admit_image_uploads(&plan)?;
        self.text.plan(scene)?;
        Ok(())
    }
    fn admit_image_uploads(&self, plan: &FramePlan) -> Result<(), SparseSessionError> {
        let bytes = plan
            .variants
            .keys()
            .try_fold(0usize, |sum, key| sum.checked_add(padded_bytes(*key)?))
            .ok_or(SparseSessionError::InvalidScene {
                op_index: None,
                reason: "padded image byte accounting overflow",
            })?;
        budget(
            "WebGL padded image bytes",
            bytes,
            self.state.limits.max_cached_bytes,
        )
    }
    pub fn render(&mut self, scene: &Scene, generation: u64) -> Result<(), WebGlSessionError> {
        self.render_with_clear(scene, generation, [0.; 4])
    }
    pub fn render_with_clear(
        &mut self,
        scene: &Scene,
        generation: u64,
        clear: [f32; 4],
    ) -> Result<(), WebGlSessionError> {
        self.check_generation(generation)?;
        if clear
            .iter()
            .any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
        {
            return Err(SparseSessionError::InvalidScene {
                op_index: None,
                reason: "invalid clear color",
            }
            .into());
        }
        if self.gl_context().drawing_buffer_width() != scene.viewport_width as i32
            || self.gl_context().drawing_buffer_height() != scene.viewport_height as i32
        {
            return Err(SparseSessionError::InvalidTarget {
                reason: "drawing buffer must match scene viewport",
            }
            .into());
        }
        // Late unsupported operations fail here, before CPU/GPU cache mutation.
        let plan = self.state.plan(scene, VelloBackend::WebGl2)?;
        self.admit_image_uploads(&plan)?;
        let text_plan = self.text.plan(scene)?;
        self.state.hydrate(&plan);
        self.text.commit(&text_plan);
        let mut ids = HashMap::new();
        for (index, key) in self.state.cache.keys().enumerate() {
            let id = TextureId(index as u64);
            ids.insert(*key, id);
        }
        let paints = plan
            .ops
            .iter()
            .map(|(index, key)| {
                (*index, {
                    // The pinned GPU shader clamps Pad coordinates before
                    // subtracting the bilinear half-pixel offset. Extend
                    // only that source region by one duplicated edge texel;
                    // retain original mapping and repeat/nearest domains.
                    let padded =
                        matches!(&scene.ops[*index], SceneOp::Image(image) if !image.nearest);
                    let extra = u16::from(padded);
                    let source = ImageSource::external_texture(
                        ids[key],
                        RectU16::new(0, 0, key.width() + extra, key.height() + extra),
                        self.state.cache[key].pixmap.may_have_transparency(),
                    );
                    paint(&scene.ops[*index], *key, source)
                })
            })
            .collect();
        let mut packet =
            vello_hybrid::Scene::new(scene.viewport_width as u16, scene.viewport_height as u16);
        let mut glyph_error = None;
        sparse::lower_admitted(&mut packet, scene, &paints, |context, index, run| {
            if glyph_error.is_none() {
                if let Err(error) = self.text.draw(context, scene, run, &text_plan.runs[&index]) {
                    glyph_error = Some(SparseSessionError::GlyphRender {
                        op_index: index,
                        reason: error.to_string(),
                    });
                }
            }
        });
        if let Some(error) = glyph_error {
            return Err(error.into());
        }
        if let Some(error) = packet.flattening_error() {
            return Err(SparseSessionError::CurveFlattening {
                reason: error.to_string(),
            }
            .into());
        }
        self.retire_unused_textures();
        let gl = self.renderer.gl_context();
        let mut bindings = WebGlTextureBindings::new();
        for (key, cached) in &self.state.cache {
            if !self.textures.contains_key(key) {
                let texture = gl
                    .create_texture()
                    .ok_or_else(|| provider("image texture allocation failed"))?;
                gl.bind_texture(Gl::TEXTURE_2D, Some(&texture));
                gl.tex_parameteri(Gl::TEXTURE_2D, Gl::TEXTURE_MIN_FILTER, Gl::LINEAR as i32);
                gl.tex_parameteri(Gl::TEXTURE_2D, Gl::TEXTURE_MAG_FILTER, Gl::LINEAR as i32);
                gl.tex_parameteri(Gl::TEXTURE_2D, Gl::TEXTURE_WRAP_S, Gl::CLAMP_TO_EDGE as i32);
                gl.tex_parameteri(Gl::TEXTURE_2D, Gl::TEXTURE_WRAP_T, Gl::CLAMP_TO_EDGE as i32);
                gl.pixel_storei(Gl::UNPACK_ALIGNMENT, 1);
                gl.pixel_storei(Gl::UNPACK_PREMULTIPLY_ALPHA_WEBGL, 0);
                gl.pixel_storei(Gl::UNPACK_FLIP_Y_WEBGL, 0);
                gl.tex_image_2d_with_i32_and_i32_and_i32_and_format_and_type_and_opt_u8_array(
                    Gl::TEXTURE_2D,
                    0,
                    Gl::RGBA8 as i32,
                    i32::from(key.width()) + 1,
                    i32::from(key.height()) + 1,
                    0,
                    Gl::RGBA,
                    Gl::UNSIGNED_BYTE,
                    Some(&pad_edges(cached.pixmap.data_as_u8_slice(), *key)),
                )
                .map_err(|error| provider(format!("image upload: {error:?}")))?;
                self.textures.insert(*key, texture);
                self.state.uploaded = self.state.uploaded.saturating_add(1);
            }
            bindings.insert(ids[key], self.textures[key].clone());
        }
        self.renderer
            .render(
                &packet,
                &mut self.resources,
                &vello_hybrid::RenderSize {
                    width: scene.viewport_width as u16,
                    height: scene.viewport_height as u16,
                },
                &bindings,
                AlphaColor::<Srgb>::new(clear),
            )
            .map_err(provider)?;
        if self.gl_context().is_context_lost() {
            return Err(WebGlSessionError::ContextLost);
        }
        self.text.maintain();
        Ok(())
    }
}
impl Drop for WebGlSession {
    fn drop(&mut self) {
        for texture in self.textures.values() {
            self.renderer.gl_context().delete_texture(Some(texture));
        }
    }
}

// Upload a duplicated final row/column. The CPU cache and paint transform
// remain in the original source domain; only GPU residency includes padding.
fn padded_bytes(key: VariantKey) -> Option<usize> {
    (usize::from(key.width()) + 1)
        .checked_mul(usize::from(key.height()) + 1)?
        .checked_mul(4)
}
fn pad_edges(pixels: &[u8], key: VariantKey) -> Vec<u8> {
    let width = usize::from(key.width());
    let height = usize::from(key.height());
    let mut padded = Vec::with_capacity(padded_bytes(key).expect("admitted upload size"));
    for y in 0..=height {
        let row = &pixels[y.min(height - 1) * width * 4..][..width * 4];
        padded.extend_from_slice(row);
        padded.extend_from_slice(&row[(width - 1) * 4..]);
    }
    padded
}
