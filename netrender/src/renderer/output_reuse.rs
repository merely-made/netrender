// Copyright 2026 Mark Alan Boykin
// SPDX-License-Identifier: MPL-2.0

use super::Renderer;

impl Renderer {
    /// Whether Classic's current rendering policy permits reusing an already
    /// qualified final raster for identical admitted inputs.
    ///
    /// The tile-dirty diagnostic animates with tile age even when paint is
    /// unchanged, so it refuses reuse. An absent or uninspectable rasterizer
    /// also refuses. This checks policy only: callers must separately compare
    /// the complete paint/resources, mutable registry dependencies, physical
    /// target parameters and device/context lifetime. It does not qualify a
    /// raster, and dirty-tile counts are not an equality witness.
    pub fn vello_output_reuse_allowed(&self) -> bool {
        #[cfg(feature = "without-classic-vello")]
        {
            false
        }
        #[cfg(not(feature = "without-classic-vello"))]
        {
            self.tile_cache.is_some()
                && self
                    .vello_rasterizer
                    .as_ref()
                    .and_then(|rasterizer| rasterizer.try_lock().ok())
                    .is_some_and(|rasterizer| !rasterizer.dirty_overlay_enabled())
        }
    }
}

#[cfg(all(test, not(feature = "without-classic-vello")))]
mod tests {
    use crate::{NetrenderOptions, boot, create_netrender_instance};

    #[test]
    fn qualified_output_reuse_requires_classic_without_animated_diagnostics() {
        let handles = boot().expect("wgpu adapter required for reuse-policy control");
        let unavailable = create_netrender_instance(handles.clone(), NetrenderOptions::default())
            .expect("renderer without Classic");
        assert!(!unavailable.vello_output_reuse_allowed());

        let renderer = create_netrender_instance(
            handles,
            NetrenderOptions {
                tile_cache_size: Some(16),
                enable_vello: true,
                ..Default::default()
            },
        )
        .expect("Classic renderer");
        assert!(renderer.vello_output_reuse_allowed());
        {
            let _held = renderer.vello_rasterizer.as_ref().unwrap().lock().unwrap();
            assert!(
                !renderer.vello_output_reuse_allowed(),
                "eligibility never waits on the rasterizer"
            );
        }
        renderer
            .vello_rasterizer
            .as_ref()
            .unwrap()
            .lock()
            .unwrap()
            .set_dirty_overlay(true, 30);
        assert!(!renderer.vello_output_reuse_allowed());
        renderer
            .vello_rasterizer
            .as_ref()
            .unwrap()
            .lock()
            .unwrap()
            .set_dirty_overlay(false, 30);
        assert!(renderer.vello_output_reuse_allowed());
    }
}
