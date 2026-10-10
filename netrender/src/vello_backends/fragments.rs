// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Source retention, shared by the owned CPU and Hybrid sessions. Resolution
//! expands source ops each frame; it does not cache backend packets or pixels.

use super::{sessions::validate, SparseResourceLimits, SparseSessionError, VelloBackend};
use crate::scene::{
    FragmentId, Scene, SceneClip, SceneFragment, SceneLayer, SceneOp, Transform, NO_CLIP,
};
use std::{
    collections::HashMap,
    sync::atomic::{AtomicU64, Ordering},
};

// Separate from Classic's renderer-local allocation. Never recycle keys across
// sparse registries in this process, including after removal, so a foreign/stale
// placement fails. IDs are not a persistent or cross-process resource protocol.
static NEXT_ID: AtomicU64 = AtomicU64::new(1 << 63);

/// Logical source residency limits, not total allocator/backend memory limits.
#[derive(Debug, Clone, Copy)]
pub struct SparseFragmentLimits {
    pub max_fragments: usize,
    /// Ops, palette entries, glyphs, axes, path segments, stops, dash entries and image declarations.
    pub max_complexity: usize,
    pub max_font_bytes: usize,
    pub max_image_bytes: usize,
}

impl Default for SparseFragmentLimits {
    fn default() -> Self {
        Self {
            max_fragments: 256,
            max_complexity: 100_000,
            max_font_bytes: 64 * 1024 * 1024,
            max_image_bytes: 64 * 1024 * 1024,
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SparseFragmentStats {
    pub fragments: usize,
    pub complexity: usize,
    /// Conservative sums: shared blobs referenced twice are counted twice.
    pub font_bytes: usize,
    pub image_bytes: usize,
}

struct Entry {
    fragment: SceneFragment,
    generation: u64,
}

/// An application/session-owned source ledger that can resolve the same frame
/// for either optional adapter. IDs are private to this registry, never Classic
/// renderer IDs. Migrating Classic placements requires an explicit ID mapping.
/// Text remains caller-shaped; external GPU images still require CPU sources.
/// Image keys retain Scene's shared keyspace, but conflicting definitions refuse
/// instead of overwriting. Update the owning fragment to change embedded assets,
/// or leave its image_sources empty and use the session's image-source API.
pub struct SparseFragmentRegistry {
    limits: SparseFragmentLimits,
    entries: HashMap<FragmentId, Entry>,
}

impl SparseFragmentRegistry {
    pub fn new(limits: SparseFragmentLimits) -> Self {
        Self {
            limits,
            entries: HashMap::new(),
        }
    }

    /// Retain source content without allocating any backend resource. Parameter,
    /// font and image admission still occurs when resolving/rendering a frame.
    pub fn register_fragment(
        &mut self,
        fragment: SceneFragment,
    ) -> Result<FragmentId, SparseSessionError> {
        self.check_update(None, &fragment)?;
        let id = NEXT_ID
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
            .map_err(|_| invalid("sparse fragment ID space exhausted"))?;
        self.entries.insert(
            id,
            Entry {
                fragment,
                generation: 0,
            },
        );
        Ok(id)
    }

    /// Atomic source replacement. The next resolution sees the new content;
    /// there is no lowered fragment cache to invalidate.
    pub fn update_fragment(
        &mut self,
        id: FragmentId,
        fragment: SceneFragment,
    ) -> Result<(), SparseSessionError> {
        let generation = self
            .entries
            .get(&id)
            .ok_or(SparseSessionError::MissingFragment { id })?
            .generation
            .checked_add(1)
            .ok_or_else(|| invalid("fragment generation exhausted"))?;
        self.check_update(Some(id), &fragment)?;
        self.entries.insert(
            id,
            Entry {
                fragment,
                generation,
            },
        );
        Ok(())
    }

    pub fn generation(&self, id: FragmentId) -> Option<u64> {
        self.entries.get(&id).map(|entry| entry.generation)
    }

    /// Release retained source content. Already hydrated session images/fonts
    /// have their own lifetime; use the session removal/clear APIs for those.
    pub fn remove_fragment(&mut self, id: FragmentId) -> bool {
        self.entries.remove(&id).is_some()
    }
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    pub fn stats(&self) -> SparseFragmentStats {
        self.entries
            .values()
            .fold(SparseFragmentStats::default(), |mut stats, entry| {
                add_stats(&mut stats, &entry.fragment);
                stats
            })
    }

    fn check_update(
        &self,
        id: Option<FragmentId>,
        fragment: &SceneFragment,
    ) -> Result<(), SparseSessionError> {
        if fragment
            .transforms
            .first()
            .is_none_or(|t| t.m != Transform::IDENTITY.m)
            || fragment
                .fonts
                .first()
                .is_none_or(|font| !font.data.as_ref().is_empty())
        {
            return Err(invalid(
                "fragment requires identity transform and empty font sentinel at index 0",
            ));
        }
        if fragment
            .ops
            .iter()
            .any(|op| matches!(op, SceneOp::Fragment(_)))
        {
            return Err(invalid("nested retained fragments are not supported"));
        }
        // Check references and balance before ID remapping; invalid local IDs
        // must never accidentally resolve into the parent's palette.
        let mut depth = 0usize;
        for op in &fragment.ops {
            if transform_id(op).is_some_and(|id| id as usize >= fragment.transforms.len()) {
                return Err(invalid("fragment references a missing local transform"));
            }
            match op {
                SceneOp::GlyphRun(run)
                    if run.font_id == 0 || run.font_id as usize >= fragment.fonts.len() =>
                {
                    return Err(invalid("fragment references a missing local font"))
                }
                SceneOp::PushLayer(_) => depth += 1,
                SceneOp::PopLayer if depth == 0 => {
                    return Err(invalid("fragment layers are unbalanced"))
                }
                SceneOp::PopLayer => depth -= 1,
                _ => {}
            }
        }
        if depth != 0 {
            return Err(invalid("fragment layers are unbalanced"));
        }
        let mut stats = SparseFragmentStats::default();
        for (key, entry) in &self.entries {
            if Some(*key) != id {
                add_stats(&mut stats, &entry.fragment);
            }
        }
        add_stats(&mut stats, fragment);
        check_budget(
            "retained fragment count",
            stats.fragments,
            self.limits.max_fragments,
        )?;
        check_budget(
            "retained fragment complexity",
            stats.complexity,
            self.limits.max_complexity,
        )?;
        check_budget(
            "retained fragment font bytes",
            stats.font_bytes,
            self.limits.max_font_bytes,
        )?;
        check_budget(
            "retained fragment image bytes",
            stats.image_bytes,
            self.limits.max_image_bytes,
        )
    }

    /// Expand placements in painter order and remap fonts/transforms. Primitive
    /// device clips belong to fragment coordinates and receive placement through
    /// a clip layer; local geometry receives local.then(placement). Enclosing
    /// parent layers, root state and compositor declarations are preserved.
    /// The complete expanded scene must pass the selected adapter's operation
    /// and parameter admission. Resource hydration stays in Cpu/HybridSession.
    /// Failure mutates neither the ledger, session caches nor a visible target.
    pub fn resolve(
        &self,
        scene: &Scene,
        backend: VelloBackend,
        limits: SparseResourceLimits,
    ) -> Result<Scene, SparseSessionError> {
        if backend == VelloBackend::Classic {
            return Err(invalid("sparse fragment resolution requires CPU or Hybrid"));
        }
        // Validate the parent without interpreting Fragment as an unsupported
        // sparse op. No expansion is allowed to hide an invalid parent palette.
        if scene
            .transforms
            .first()
            .is_none_or(|t| t.m != Transform::IDENTITY.m)
            || scene
                .fonts
                .first()
                .is_none_or(|font| !font.data.as_ref().is_empty())
        {
            return Err(invalid(
                "parent requires identity transform and empty font sentinel at index 0",
            ));
        }
        check_budget(
            "expanded scene complexity",
            complexity(scene),
            limits.max_ops,
        )?;
        for op in &scene.ops {
            if matches!(op, SceneOp::GlyphRun(run) if run.font_id == 0 || run.font_id as usize >= scene.fonts.len())
            {
                return Err(invalid("parent references a missing font"));
            }
            if transform_id(op).is_some_and(|id| id as usize >= scene.transforms.len()) {
                return Err(invalid("parent references a missing transform"));
            }
        }
        let mut result = scene.clone();
        result.ops.retain(|op| !matches!(op, SceneOp::Fragment(_)));
        super::validate_scene_operations(backend, &result, true)?;
        validate::scene(&result, limits)?;
        result.ops.clear();
        for op in &scene.ops {
            if let SceneOp::Fragment(placed) = op {
                let entry = self
                    .entries
                    .get(&placed.id)
                    .ok_or(SparseSessionError::MissingFragment { id: placed.id })?;
                let mut cost = SparseFragmentStats::default();
                add_stats(&mut cost, &entry.fragment);
                // Include two clip wrapper ops per primitive before cloning.
                check_budget(
                    "expanded fragment complexity",
                    complexity(&result)
                        .saturating_add(cost.complexity)
                        .saturating_add(entry.fragment.ops.len().saturating_mul(2)),
                    limits.max_ops,
                )?;
                let mut local = Scene::new(scene.viewport_width, scene.viewport_height);
                local.append_fragment(entry.fragment.clone());
                super::validate_scene_operations(backend, &local, true)?;
                validate::scene(&local, limits)?;
                let mut fragment = entry.fragment.clone();
                for (key, source) in &fragment.image_sources {
                    if let Some(old) = result.image_sources.get(key) {
                        if old.width != source.width
                            || old.height != source.height
                            || old.data.id() != source.data.id()
                            || old.data.as_ref() != source.data.as_ref()
                        {
                            return Err(SparseSessionError::InvalidImage {
                                key: *key,
                                reason: "conflicting parent/fragment image definitions",
                            });
                        }
                    }
                }
                let placement = scene.transforms[placed.transform_id as usize];
                fragment.transforms = std::iter::once(Transform::IDENTITY)
                    .chain(
                        fragment
                            .transforms
                            .iter()
                            .map(|local| local.then(&placement)),
                    )
                    .collect();
                let mut ops = Vec::new();
                for mut inner in fragment.ops {
                    if let Some(id) = transform_id_mut(&mut inner) {
                        *id += 1;
                    }
                    let clip = take_clip(&mut inner);
                    if let Some(clip) = &clip {
                        let mut layer = SceneLayer::clip(clip.clone());
                        layer.transform_id = 1;
                        ops.push(SceneOp::PushLayer(layer));
                    }
                    ops.push(inner);
                    if clip.is_some() {
                        ops.push(SceneOp::PopLayer);
                    }
                }
                fragment.ops = ops;
                result.append_fragment(fragment);
            } else {
                result.ops.push(op.clone());
            }
        }
        super::validate_scene_operations(backend, &result, true)?;
        validate::scene(&result, limits)?;
        Ok(result)
    }
}

fn invalid(reason: &'static str) -> SparseSessionError {
    SparseSessionError::InvalidScene {
        op_index: None,
        reason,
    }
}
fn check_budget(
    resource: &'static str,
    requested: usize,
    limit: usize,
) -> Result<(), SparseSessionError> {
    if requested > limit {
        Err(SparseSessionError::ResourceBudgetExceeded {
            resource,
            requested,
            limit,
        })
    } else {
        Ok(())
    }
}

fn add_stats(stats: &mut SparseFragmentStats, fragment: &SceneFragment) {
    stats.fragments = stats.fragments.saturating_add(1);
    stats.complexity = stats
        .complexity
        .saturating_add(op_cost(&fragment.ops))
        .saturating_add(fragment.transforms.len())
        .saturating_add(fragment.fonts.len())
        .saturating_add(fragment.image_sources.len());
    for font in &fragment.fonts {
        stats.font_bytes = stats.font_bytes.saturating_add(font.data.as_ref().len());
    }
    for image in fragment.image_sources.values() {
        stats.image_bytes = stats.image_bytes.saturating_add(image.data.as_ref().len());
    }
}
fn complexity(scene: &Scene) -> usize {
    op_cost(&scene.ops)
        .saturating_add(scene.transforms.len())
        .saturating_add(scene.fonts.len())
}
fn op_cost(ops: &[SceneOp]) -> usize {
    ops.iter().fold(ops.len(), |cost, op| {
        cost.saturating_add(match op {
            SceneOp::GlyphRun(v) => v.glyphs.len().saturating_add(v.font_axis_values.len()),
            SceneOp::Gradient(v) => v.stops.len(),
            SceneOp::Stroke(v) => v.dash_pattern.len(),
            SceneOp::Shape(v) => v
                .path
                .ops
                .len()
                .saturating_add(v.stroke.as_ref().map_or(0, |v| v.dash_pattern.len())),
            SceneOp::PushLayer(v) => v.filters.len().saturating_add(match &v.clip {
                SceneClip::Path(path) => path.ops.len(),
                _ => 0,
            }),
            _ => 0,
        })
    })
}
fn transform_id(op: &SceneOp) -> Option<u32> {
    match op {
        SceneOp::Rect(v) => Some(v.transform_id),
        SceneOp::Stroke(v) => Some(v.transform_id),
        SceneOp::Gradient(v) => Some(v.transform_id),
        SceneOp::Image(v) => Some(v.transform_id),
        SceneOp::Pattern(v) => Some(v.transform_id),
        SceneOp::Shape(v) => Some(v.transform_id),
        SceneOp::GlyphRun(v) => Some(v.transform_id),
        SceneOp::PushLayer(v) => Some(v.transform_id),
        SceneOp::Fragment(v) => Some(v.transform_id),
        SceneOp::PopLayer => None,
    }
}
fn transform_id_mut(op: &mut SceneOp) -> Option<&mut u32> {
    match op {
        SceneOp::Rect(v) => Some(&mut v.transform_id),
        SceneOp::Stroke(v) => Some(&mut v.transform_id),
        SceneOp::Gradient(v) => Some(&mut v.transform_id),
        SceneOp::Image(v) => Some(&mut v.transform_id),
        SceneOp::Pattern(v) => Some(&mut v.transform_id),
        SceneOp::Shape(v) => Some(&mut v.transform_id),
        SceneOp::GlyphRun(v) => Some(&mut v.transform_id),
        SceneOp::PushLayer(v) => Some(&mut v.transform_id),
        SceneOp::Fragment(v) => Some(&mut v.transform_id),
        SceneOp::PopLayer => None,
    }
}
fn take_clip(op: &mut SceneOp) -> Option<SceneClip> {
    let (rect, radii) = match op {
        SceneOp::Rect(v) => (&mut v.clip_rect, v.clip_corner_radii),
        SceneOp::Stroke(v) => (&mut v.clip_rect, v.clip_corner_radii),
        SceneOp::Gradient(v) => (&mut v.clip_rect, v.clip_corner_radii),
        SceneOp::Image(v) => (&mut v.clip_rect, v.clip_corner_radii),
        SceneOp::Pattern(v) => (&mut v.clip_rect, v.clip_corner_radii),
        SceneOp::Shape(v) => (&mut v.clip_rect, v.clip_corner_radii),
        SceneOp::GlyphRun(v) => (&mut v.clip_rect, v.clip_corner_radii),
        _ => return None,
    };
    if *rect == NO_CLIP {
        None
    } else {
        Some(SceneClip::Rect {
            rect: std::mem::replace(rect, NO_CLIP),
            radii,
        })
    }
}
