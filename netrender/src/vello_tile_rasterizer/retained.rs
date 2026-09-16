// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Roadmap E4 (spike) — retained fragments.
//!
//! A consumer registers a [`SceneFragment`] once, receives a
//! [`FragmentId`], and thereafter *places* it per frame
//! (`Scene::place_fragment`) instead of re-pushing its ops. The
//! rasterizer caches the fragment's lowered `vello::Scene` across
//! frames and composes placements with `vello::Scene::append(_,
//! Some(affine))`, so a placement-only change (pan / scroll / drag)
//! costs an append rather than a re-lower. Content changes go through
//! `update_fragment`, which bumps the generation and drops the cached
//! lowering.
//!
//! Design: `netrender-notes/2026-08-10_fragment_retention_design.md`.
//! The measured motivation is the pan table in
//! `examples/e1_damage_profile.rs`: 12.9 ms vs 1.2 ms at 4096²
//! pre-spike, all CPU, for a placement-only change.
//!
//! Scenes containing `SceneOp::Fragment` take this module's master
//! path and bypass the tile cache entirely; scenes without fragments
//! keep the exact pre-E4 tile path.
//!
//! Layer scopes are hoisted onto the master: `PushLayer` / `PopLayer`
//! flush the pending run and then push / pop on the master
//! `vello::Scene` itself, so a placement inside an open layer appends
//! its cached lowering *into* that layer (genet T4, 2026-09-16).
//! Before that the layer lived in the run's sub-scene and a
//! layer-scoped placement had to inline un-retained. One spike
//! limitation survives: nested fragments (fragment content placing
//! another fragment) are skipped at lower time by `scene_to_vello`'s
//! own warn arm.

use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::Hasher;

use vello::kurbo::{Affine, Rect};
use vello::peniko::{Fill, Mix};

use crate::scene::{FragmentId, Scene, SceneFragment, SceneOp, Transform};
use crate::tile_cache::op_hash;
use crate::vello_rasterizer::{emit_push_layer, scene_to_vello_with_overrides, transform_to_affine};

use super::VelloTileRasterizer;

/// One registered fragment plus its cached lowering.
pub(super) struct RetainedFragment {
    /// Bumped by `update_fragment`; participates in the frame
    /// signature so content changes rebuild the master.
    generation: u64,
    /// The content, kept for (re-)lowering.
    fragment: SceneFragment,
    /// Cached lowering, dropped on update. Lazy: first placement
    /// lowers.
    lowered: Option<vello::Scene>,
}

/// Registry + cached master + receipt counters, one per rasterizer.
#[derive(Default)]
pub(crate) struct RetainedState {
    fragments: HashMap<FragmentId, RetainedFragment>,
    next_id: FragmentId,
    /// `(signature, master)` of the last fragment-path frame. Reused
    /// wholesale when the signature matches.
    cached_master: Option<(u64, vello::Scene)>,
    /// Times a fragment was lowered (receipt: retention means this
    /// does not grow under placement-only change).
    lower_count: u64,
    /// Times the cached master was reused wholesale (receipt for the
    /// unchanged-frame short-circuit).
    master_hits: u64,
}

impl RetainedState {
    /// Drop the whole-master shortcut when an external image receives a new
    /// Vello identity or is retired. Its frame signature contains image keys,
    /// not override identities.
    pub(crate) fn invalidate_image_override_cache(&mut self) {
        self.cached_master = None;
        for fragment in self.fragments.values_mut() {
            fragment.lowered = None;
        }
    }

    pub(crate) fn register(&mut self, fragment: SceneFragment) -> FragmentId {
        let id = self.next_id;
        self.next_id += 1;
        self.fragments.insert(
            id,
            RetainedFragment {
                generation: 0,
                fragment,
                lowered: None,
            },
        );
        id
    }

    pub(crate) fn update(&mut self, id: FragmentId, fragment: SceneFragment) -> bool {
        match self.fragments.get_mut(&id) {
            Some(r) => {
                r.generation += 1;
                r.fragment = fragment;
                r.lowered = None;
                // Content changed; the cached master (which composed
                // the old lowering) is stale regardless of signature
                // arithmetic, and the generation bump ensures the
                // signature moves too. Drop it eagerly for clarity.
                self.cached_master = None;
                true
            }
            None => false,
        }
    }

    pub(crate) fn remove(&mut self, id: FragmentId) -> bool {
        let removed = self.fragments.remove(&id).is_some();
        if removed {
            self.cached_master = None;
        }
        removed
    }

    pub(crate) fn lower_count(&self) -> u64 {
        self.lower_count
    }

    pub(crate) fn master_hits(&self) -> u64 {
        self.master_hits
    }
}

/// Does this scene take the fragment path?
pub(super) fn has_fragments(scene: &Scene) -> bool {
    scene
        .ops
        .iter()
        .any(|op| matches!(op, SceneOp::Fragment(_)))
}

/// Frame signature. Two frames with equal signatures compose
/// byte-identical masters, so the cached one can be reused.
///
/// Direct ops hash their full field bytes via the tile cache's op
/// hashers (the same functions whose completeness the E3 differential
/// tests pin), plus the *resolved* transform matrix — the field hash
/// carries only the transform id, and ids are stable across frames
/// while their matrices move. Fragments hash identity + generation +
/// placement matrix. Image sources participate by key, matching the
/// tile path's bytes-under-a-stable-key-are-trusted semantics.
fn frame_signature(scene: &Scene, state: &RetainedState) -> u64 {
    let mut h = DefaultHasher::new();
    h.write_u32(scene.viewport_width);
    h.write_u32(scene.viewport_height);
    h.write_u32(scene.root_alpha.to_bits());
    h.write_u8(scene.root_blend_mode as u8);

    for op in &scene.ops {
        match op {
            SceneOp::Fragment(f) => {
                h.write_u8(0xF0);
                h.write_u64(f.id);
                let generation = state
                    .fragments
                    .get(&f.id)
                    .map_or(u64::MAX, |r| r.generation);
                h.write_u64(generation);
                hash_matrix(&mut h, &scene.transforms[f.transform_id as usize]);
            }
            other => {
                h.write_u8(0x0D);
                op_hash::hash_op_fields(&mut h, other);
                if let Some(tid) = op_hash::op_transform_id(other) {
                    hash_matrix(&mut h, &scene.transforms[tid as usize]);
                }
            }
        }
    }

    let mut keys: Vec<_> = scene.image_sources.keys().copied().collect();
    keys.sort_unstable();
    for k in keys {
        h.write_u64(k);
    }
    h.finish()
}

fn hash_matrix(h: &mut DefaultHasher, t: &Transform) {
    for v in t.m {
        h.write_u32(v.to_bits());
    }
}

/// Build a standalone `Scene` from a fragment so the ordinary lowering
/// path can translate it. Viewport is nominal (lowering never reads
/// it); tables arrive via E2's `append_fragment` id rewriting.
fn fragment_to_scene(fragment: &SceneFragment) -> Scene {
    let mut s = Scene::new(1, 1);
    s.append_fragment(fragment.clone());
    s
}

/// A painter-order run of leaf ops, accumulated into a scratch `Scene`
/// carrying the parent's tables. Layers and fragments are master-level,
/// so a run never holds `PushLayer` / `PopLayer` / `Fragment` and its
/// tables never change after construction.
struct Run {
    scene: Scene,
    is_empty: bool,
}

impl Run {
    fn new(parent: &Scene) -> Self {
        let mut scene = Scene::new(parent.viewport_width, parent.viewport_height);
        scene.transforms = parent.transforms.clone();
        scene.fonts = parent.fonts.clone();
        Self {
            scene,
            is_empty: true,
        }
    }

    fn push(&mut self, op: SceneOp) {
        self.scene.ops.push(op);
        self.is_empty = false;
    }

    /// Lower what has accumulated and append it at the master's
    /// current layer depth. Appending into an open master layer is the
    /// move `compose_master` already makes per tile.
    fn flush_into(
        &mut self,
        master: &mut vello::Scene,
        merged_images: &HashMap<u64, vello::peniko::ImageData>,
    ) {
        if self.is_empty {
            return;
        }
        let sub = scene_to_vello_with_overrides(&self.scene, merged_images);
        master.append(&sub, None);
        self.scene.ops.clear();
        self.is_empty = true;
    }
}

impl VelloTileRasterizer {
    /// Roadmap E4 — build the master scene for a fragment-bearing
    /// scene. Bypasses the tile cache: direct ops lower fresh in
    /// painter-order runs (expected few — the retained content is the
    /// bulk), fragment placements append cached lowerings.
    ///
    /// Returns a clone of the cached master on signature match. The
    /// clone is an encoding memcpy, paid so callers keep ownership
    /// semantics; if profiles show it mattering, the fix is an
    /// append-from-cache path in `compose_into`, not avoiding the
    /// cache.
    pub(super) fn build_master_scene_fragments(
        &mut self,
        scene: &Scene,
        timings: &mut crate::profiling::FrameTimings,
    ) -> vello::Scene {
        use crate::profiling::Span;

        self.refresh_image_data(scene);

        let sig_span = Span::start("fragment_signature");
        let signature = frame_signature(scene, &self.retained);
        sig_span.stop_recording(timings);

        if let Some((cached_sig, cached)) = &self.retained.cached_master {
            if *cached_sig == signature {
                self.retained.master_hits += 1;
                self.last_dirty_count = 0;
                self.last_dirty_tiles.clear();
                let hit_span = Span::start("master_compose");
                let master = cached.clone();
                hit_span.stop_recording(timings);
                return master;
            }
        }

        let compose_span = Span::start("master_compose");

        // Merged Path A + Path B image map, same as the tile path.
        let mut merged_images = self.image_data.clone();
        for (key, image) in &self.image_overrides {
            merged_images.insert(*key, image.clone());
        }

        let mut master = vello::Scene::new();

        // Phase 12a' scene-level alpha + blend wrap, mirroring
        // `compose_master`.
        let scene_alpha = scene.root_alpha.clamp(0.0, 1.0);
        let scene_blend = super::master::map_blend_mode(scene.root_blend_mode);
        let needs_root_layer = scene_alpha < 1.0 || scene_blend.mix != Mix::Normal;
        if needs_root_layer {
            let viewport = Rect::new(
                0.0,
                0.0,
                scene.viewport_width as f64,
                scene.viewport_height as f64,
            );
            master.push_layer(
                Fill::NonZero,
                scene_blend,
                scene_alpha,
                Affine::IDENTITY,
                &viewport,
            );
        }

        let mut run = Run::new(scene);
        let mut layer_depth: u32 = 0;
        let mut relowered: u64 = 0;

        for op in &scene.ops {
            match op {
                SceneOp::Fragment(f) => {
                    let Some(retained) = self.retained.fragments.get(&f.id) else {
                        log::warn!(
                            "fragment path: placed FragmentId {} is not registered; skipped",
                            f.id
                        );
                        continue;
                    };
                    run.flush_into(&mut master, &merged_images);
                    if retained.lowered.is_none() {
                        let tmp = fragment_to_scene(&retained.fragment);
                        let lowered = scene_to_vello_with_overrides(&tmp, &merged_images);
                        let r = self.retained.fragments.get_mut(&f.id).unwrap();
                        r.lowered = Some(lowered);
                        self.retained.lower_count += 1;
                        relowered += 1;
                    }
                    let r = &self.retained.fragments[&f.id];
                    let affine = transform_to_affine(&scene.transforms[f.transform_id as usize]);
                    // Open master layers, if any, contain this append
                    // — which is the point: retention survives a layer
                    // scope.
                    master.append(r.lowered.as_ref().unwrap(), Some(affine));
                }
                SceneOp::PushLayer(layer) => {
                    // Hoisted to the master so the scope is open around
                    // fragment appends, not buried in the run sub-scene.
                    run.flush_into(&mut master, &merged_images);
                    emit_push_layer(&mut master, layer, scene);
                    layer_depth += 1;
                }
                SceneOp::PopLayer => {
                    debug_assert!(
                        layer_depth > 0,
                        "SceneOp::PopLayer with no matching PushLayer"
                    );
                    if layer_depth > 0 {
                        run.flush_into(&mut master, &merged_images);
                        master.pop_layer();
                        layer_depth -= 1;
                    }
                }
                other => run.push(other.clone()),
            }
        }
        run.flush_into(&mut master, &merged_images);

        // An unbalanced scene would otherwise trap the root wrap inside
        // a stray layer.
        debug_assert_eq!(layer_depth, 0, "Scene ended with unclosed PushLayer(s)");
        for _ in 0..layer_depth {
            master.pop_layer();
        }

        if needs_root_layer {
            master.pop_layer();
        }

        compose_span.stop_recording(timings);

        // Honest dirty accounting for this path: no tiles exist, so
        // report the number of fragments re-lowered this frame.
        self.last_dirty_count = relowered as usize;
        self.last_dirty_tiles.clear();

        self.retained.cached_master = Some((signature, master.clone()));
        master
    }
}
