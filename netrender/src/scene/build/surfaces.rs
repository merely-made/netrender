// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

use super::*;

impl Scene {
    /// Declare or update a native-compositor surface. If the key was
    /// not present, append to `compositor_surfaces` (z-order = vec
    /// position). If the key was present, update fields in place
    /// without reordering.
    ///
    /// Surfaces and `SceneOp::PushLayer` are independent: a surface
    /// may contain layers, a layer may span surfaces. Surfaces are
    /// about *cross-frame OS handoff regions*; layers are about
    /// *within-frame compositing groups*.
    pub fn declare_compositor_surface(&mut self, surface: CompositorSurface) {
        if let Some(existing) = self
            .compositor_surfaces
            .iter_mut()
            .find(|s| s.key == surface.key)
        {
            *existing = surface;
        } else {
            self.compositor_surfaces.push(surface);
        }
    }

    /// Drop a previously-declared compositor surface. No-op if the
    /// key is not present.
    pub fn undeclare_compositor_surface(&mut self, key: SurfaceKey) {
        self.compositor_surfaces.retain(|s| s.key != key);
    }

    /// Update one surface's transform without changing bounds. The
    /// transform is applied by the OS compositor at present time,
    /// not by netrender's master render — calling this does not
    /// force a content repaint.
    ///
    /// No-op if `key` is not declared.
    pub fn set_surface_transform(&mut self, key: SurfaceKey, transform: [f32; 6]) {
        if let Some(s) = self.compositor_surfaces.iter_mut().find(|s| s.key == key) {
            s.transform = transform;
        }
    }

    /// Update one surface's clip. OS-compositor metadata; does not
    /// force a content repaint. No-op if `key` is not declared.
    pub fn set_surface_clip(&mut self, key: SurfaceKey, clip: Option<[f32; 4]>) {
        if let Some(s) = self.compositor_surfaces.iter_mut().find(|s| s.key == key) {
            s.clip = clip;
        }
    }

    /// Update one surface's opacity. OS-compositor metadata; does not
    /// force a content repaint. No-op if `key` is not declared.
    pub fn set_surface_opacity(&mut self, key: SurfaceKey, opacity: f32) {
        if let Some(s) = self.compositor_surfaces.iter_mut().find(|s| s.key == key) {
            s.opacity = opacity;
        }
    }
}
