// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! `paint_list_render` — `PaintList` → `netrender::Scene` translator.
//!
//! This is the impedance bridge between the engine-facing display-list
//! vocabulary ([`paint_list_api`]) and netrender's renderer primitives.
//! Producers emit a [`paint_list_api::PaintList`] (the closed-set
//! `PaintCmd` vocabulary — compositor primitives plus `Draw*` items);
//! this crate walks the command stream and produces a
//! [`netrender::Scene`] the renderer can rasterize.
//!
//! It depends only on `netrender` + `paint_list_api`, so any engine on
//! either side of the path-dep boundary (genet, nematic, inker, …) can
//! reuse it. The genet-specific glue that *consumes* the translator
//! output — building blurred box-shadow masks on the GPU, compositing
//! external textures, the painter message loop — stays in genet's
//! `components/paint` crate; see
//! `genet/docs/2026-05-20_paintlist_extraction_plan.md`.
//!
//! ## Mapping summary
//!
//! Most variants map 1:1 to a `netrender::SceneOp`. The painter-side
//! gaps flagged for follow-up (`DrawPath`, `DrawStroke`, inset
//! `DrawShadow`) emit a fallback or `warn!`-and-skip. Borders whose
//! style or geometry cannot be represented return a translation error.
//!
//! `DrawShadow` is fully wired for outset shadows: hard shadows
//! (blur 0) lower to a solid rect; blurred ones record a
//! [`BoxShadowMaskRequest`] (built GPU-side by the painter via
//! `Renderer::build_box_shadow_mask`) and push the matching mask
//! image op into the scene at the right paint position.

use std::collections::HashMap;
use std::fmt;

use log::warn;
use netrender::{
    ExternalTexturePlacement, Glyph as NrGlyph, ImageKey as NrImageKey, NO_CLIP, SHARP_CLIP, Scene,
    SceneImage, SceneOp, ScenePathStroke, ScenePattern, SceneShape, Transform, external_image_key,
};
use paint_list_api::{self as ple, FontResource, ImageKey, ImageResource, PaintCmd, PaintList};

#[cfg(feature = "resource-bundle")]
pub mod bundle;
mod composite;
mod convert;
mod emit;

#[cfg(test)]
mod tests;

use composite::{register_fonts, register_images};
use convert::*;
use emit::*;
pub use composite::{composite_paint_layers, CompositeLayer};
/// External-texture source metadata produced by the translator.
///
/// The ordinary `SceneImage` is emitted at the command's original position,
/// so transforms, clips, and layers are normal Scene state. The host uses this
/// side channel only to stage the producer texture under `image_key` before
/// rendering and to retire it with the document.
#[derive(Clone, Debug)]
pub struct ExternalTextureDraw {
    /// Producer-owned stable key, retained for source lookup and lifecycle.
    pub texture_key: u64,
    /// Producer's content stamp, preserved intact for resource admission.
    pub content_generation: Option<u64>,
    /// Renderer image namespace key used by the in-scene `SceneImage`.
    pub image_key: NrImageKey,
    pub placement: ExternalTexturePlacement,
    /// Position before the image op, retained for legacy direct-compositor
    /// consumers that accept flat placement semantics.
    pub scene_op_boundary: usize,
}

/// A blurred box-shadow mask the painter must build on the GPU
/// (`Renderer::build_box_shadow_mask`) before rasterizing the scene.
/// The translator can't reach the `Renderer`, so it records the
/// parameters here and pushes the matching image op into the scene
/// (at the right paint position); the painter materializes the mask
/// texture under `key` ahead of the render.
#[derive(Clone, Debug)]
pub struct BoxShadowMaskRequest {
    /// netrender image key the scene's shadow image op references.
    pub key: NrImageKey,
    /// Square mask texture side length (covers the scene; the shadow
    /// box is drawn at `bounds` within it, in absolute scene coords).
    pub dim: u32,
    /// Shadow box in absolute scene coords `[x0, y0, x1, y1]`. For an outset
    /// shadow this is the offset/spread box; for an inset shadow it is the
    /// unshadowed inner "hole".
    pub bounds: [f32; 4],
    pub corner_radius: f32,
    pub blur_radius_px: f32,
    /// Produce a `1 - coverage` mask: the inset-box-shadow primitive (shadow
    /// everywhere except `bounds`). `false` for the normal outset mask.
    pub invert: bool,
}

/// The full translator output: a [`netrender::Scene`] plus the side
/// channels the painter needs to finish the frame (external-texture
/// composites and blurred-shadow mask requests).
#[derive(Debug)]
pub struct TranslatedDisplayList {
    pub scene: Scene,
    pub external_textures: Vec<ExternalTextureDraw>,
    /// Blurred box-shadow masks to build before rendering this scene.
    pub box_shadow_masks: Vec<BoxShadowMaskRequest>,
}

/// A paint command that cannot be translated without changing its border
/// appearance. The command index refers to the original command stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TranslationError {
    pub command: usize,
    pub reason: &'static str,
}

impl fmt::Display for TranslationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "paint command {}: {}", self.command, self.reason)
    }
}

impl std::error::Error for TranslationError {}

fn preflight_borders(
    commands: &[PaintCmd],
    images: &[ImageResource],
) -> Result<(), TranslationError> {
    for (command, cmd) in commands.iter().enumerate() {
        let PaintCmd::DrawBorder(border) = cmd else {
            continue;
        };
        let reason = match &border.details {
            ple::BorderDetails::Normal(sides) => admit_normal_border(border, sides).err(),
            ple::BorderDetails::NinePatch(np) => {
                let bounds = &border.placement.bounds;
                let widths = &border.widths;
                if [bounds.min.x, bounds.min.y, bounds.max.x, bounds.max.y]
                    .iter()
                    .any(|value| !value.is_finite())
                {
                    Some("non-finite border bounds")
                } else if [widths.top, widths.right, widths.bottom, widths.left]
                    .iter()
                    .any(|width| !width.is_finite() || *width < 0.0)
                {
                    Some("invalid nine-patch border width")
                } else {
                    match &np.source {
                        ple::NinePatchSource::Image(key, _) => {
                            if np.width <= 0 || np.height <= 0 {
                                Some("nine-patch source extent is not positive")
                            } else if np.slice.top < 0
                                || np.slice.bottom < 0
                                || np.slice.left < 0
                                || np.slice.right < 0
                            {
                                Some("negative nine-patch source slice")
                            } else if np.slice.top > np.height
                                || np.slice.bottom > np.height
                                || np.slice.left > np.width
                                || np.slice.right > np.width
                            {
                                Some("nine-patch source slice exceeds its axis extent")
                            } else if !images.iter().any(|image| image.key == *key) {
                                Some("nine-patch source image is not registered")
                            } else {
                                None
                            }
                        }
                        _ => Some("nine-patch gradient source is not lowered"),
                    }
                }
            }
        };
        if let Some(reason) = reason {
            return Err(TranslationError { command, reason });
        }
    }
    Ok(())
}
// =============================================================================
// PaintList → Scene entry points
// =============================================================================

/// Translate a [`PaintList`] into a [`netrender::Scene`]. External textures
/// become normal in-order scene images; hosts needing their source lifecycle
/// use [`translate_envelope_with_external_textures`] or
/// [`translate_paint_cmd_stream`] for the associated staging metadata.
pub fn translate_paint_list<L: PaintList>(list: &L) -> Result<Scene, TranslationError> {
    translate_paint_cmd_stream(
        list.viewport(),
        list.commands(),
        list.fonts(),
        list.images(),
    )
    .map(|translated| translated.scene)
}

/// Receive-side companion: translate a wire envelope. Thin wrapper
/// since `PaintEnvelope` itself impls `PaintList`.
pub fn translate_envelope(
    envelope: &paint_list_api::PaintEnvelope,
) -> Result<Scene, TranslationError> {
    translate_paint_list(envelope)
}

/// Roadmap E4 — translate a `PaintCmd` stream into a retained
/// [`netrender::scene::SceneFragment`], for consumers whose paint
/// output is already cached per retained unit.
///
/// The sprigging shape this exists for: a `Leaf` repaints only when
/// `paint_dirty`, `RenderedLeaves` caches each leaf's command splice
/// with an epoch, and the host today re-splices every leaf's commands
/// into every frame's envelope anyway. With this entry point the host
/// instead translates a leaf's splice once per epoch, registers it
/// (`Renderer::register_fragment` / `update_fragment` on epoch change),
/// and places it per frame at the leaf's layout box
/// (`Scene::place_fragment`) — extending sprigging's retention chain
/// through translation and lowering instead of stopping it at the
/// command cache.
///
/// Commands are leaf-local: coordinates are relative to the fragment's
/// own origin, and the placement transform supplies the box position.
/// External-texture draws cannot be retained (they are per-frame
/// composite state) and are skipped with a warning; Path-B leaves keep
/// the envelope path.
pub fn translate_paint_cmds_to_fragment(
    commands: &[PaintCmd],
    fonts: &[FontResource],
    images: &[ImageResource],
) -> Result<netrender::scene::SceneFragment, TranslationError> {
    // Viewport only sizes the translator's box-shadow mask scratch;
    // fragment content never reads it. Use a nominal square.
    let translated = translate_paint_cmd_stream(
        paint_list_api::DeviceIntSize::new(1, 1),
        commands,
        fonts,
        images,
    )?;
    if !translated.external_textures.is_empty() {
        log::warn!(
            "translate_paint_cmds_to_fragment: {} DrawExternalTexture command(s) skipped — \
             external textures are per-frame composite state and cannot be retained in a \
             fragment (Path-B leaves keep the envelope path)",
            translated.external_textures.len()
        );
    }
    if !translated.box_shadow_masks.is_empty() {
        log::warn!(
            "translate_paint_cmds_to_fragment: {} blurred box-shadow mask request(s) dropped — \
             mask textures are built per frame by the host painter; draw blurred shadows \
             outside the fragment, or accept the sharp-shadow fallback inside it",
            translated.box_shadow_masks.len()
        );
    }
    Ok(netrender::scene::SceneFragment::from_scene(
        translated.scene,
    ))
}

/// Variant that also returns external-source staging metadata and box-shadow
/// mask requests. The external image itself is already in the returned Scene.
pub fn translate_envelope_with_external_textures(
    envelope: &paint_list_api::PaintEnvelope,
) -> Result<TranslatedDisplayList, TranslationError> {
    translate_paint_cmd_stream(
        envelope.viewport(),
        envelope.commands(),
        envelope.fonts(),
        envelope.images(),
    )
}

mod stream;

pub use stream::translate_paint_cmd_stream;
