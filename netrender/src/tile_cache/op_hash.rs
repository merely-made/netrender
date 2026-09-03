// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Crate-facing per-op hashing and transform-id access, dispatching to
//! [`super::hash`]'s field hashers.
//!
//! Exists for roadmap E4: the retained-fragment frame signature needs
//! the same "what bytes identify this op" answer the tile cache uses,
//! and duplicating the field walks would let the two drift. The E3
//! differential tests pin the hashers' completeness; everything here is
//! a thin dispatcher over them.

use std::collections::hash_map::DefaultHasher;
use std::hash::Hasher;

use crate::scene::SceneOp;

use super::hash;

/// Hash `op`'s full visible identity (discriminant + fields) into `h`.
/// Does NOT hash the resolved transform matrix — callers that need
/// positional identity mix that in themselves via
/// [`op_transform_id`], because the field hash carries only the id.
pub(crate) fn hash_op_fields(h: &mut DefaultHasher, op: &SceneOp) {
    match op {
        SceneOp::Rect(r) => {
            h.write_u8(0);
            hash::hash_rect(h, r);
        }
        SceneOp::Stroke(s) => {
            h.write_u8(1);
            hash::hash_stroke(h, s);
        }
        SceneOp::Gradient(g) => {
            h.write_u8(2);
            hash::hash_gradient(h, g);
        }
        SceneOp::Image(i) => {
            h.write_u8(3);
            hash::hash_image(h, i);
        }
        SceneOp::Pattern(p) => {
            h.write_u8(4);
            hash::hash_pattern(h, p);
        }
        SceneOp::Shape(s) => {
            h.write_u8(5);
            hash::hash_shape(h, s);
        }
        SceneOp::GlyphRun(r) => {
            h.write_u8(6);
            hash::hash_glyph_run(h, r);
        }
        SceneOp::PushLayer(l) => {
            h.write_u8(7);
            hash::hash_push_layer(h, l);
        }
        SceneOp::PopLayer => h.write_u8(0xFF),
        SceneOp::Fragment(f) => {
            h.write_u8(0xF0);
            h.write_u64(f.id);
            h.write_u32(f.transform_id);
        }
    }
}

/// The transform id `op` resolves through, or `None` for ops that
/// carry none (`PopLayer`).
pub(crate) fn op_transform_id(op: &SceneOp) -> Option<u32> {
    match op {
        SceneOp::Rect(r) => Some(r.transform_id),
        SceneOp::Stroke(s) => Some(s.transform_id),
        SceneOp::Gradient(g) => Some(g.transform_id),
        SceneOp::Image(i) => Some(i.transform_id),
        SceneOp::Pattern(p) => Some(p.transform_id),
        SceneOp::Shape(s) => Some(s.transform_id),
        SceneOp::GlyphRun(r) => Some(r.transform_id),
        SceneOp::PushLayer(l) => Some(l.transform_id),
        SceneOp::PopLayer => None,
        SceneOp::Fragment(f) => Some(f.transform_id),
    }
}

/// Set `op`'s transform id. No-op for ops that carry none.
pub(crate) fn set_op_transform_id(op: &mut SceneOp, new_id: u32) {
    match op {
        SceneOp::Rect(r) => r.transform_id = new_id,
        SceneOp::Stroke(s) => s.transform_id = new_id,
        SceneOp::Gradient(g) => g.transform_id = new_id,
        SceneOp::Image(i) => i.transform_id = new_id,
        SceneOp::Pattern(p) => p.transform_id = new_id,
        SceneOp::Shape(s) => s.transform_id = new_id,
        SceneOp::GlyphRun(r) => r.transform_id = new_id,
        SceneOp::PushLayer(l) => l.transform_id = new_id,
        SceneOp::PopLayer => {}
        SceneOp::Fragment(f) => f.transform_id = new_id,
    }
}
