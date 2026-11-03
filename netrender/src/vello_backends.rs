// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Experimental admission boundary for Vello Classic, Hybrid, and CPU.
//!
//! [`crate::scene::Scene`] remains the authoritative display-list model. Classic
//! continues to use [`crate::vello_rasterizer`]; the opt-in sparse backends use
//! one shared lowerer so their common behavior cannot drift silently.

/// A named realization of Netrender's scene contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VelloBackend {
    /// Vello's compute-heavy GPU renderer. This is Netrender's shipping path.
    Classic,
    /// CPU path processing with GPU rasterization and compositing.
    Hybrid,
    /// Pure CPU sparse-strips rasterization.
    Cpu,
    /// Browser WebGL2 rasterization of the shared sparse scene.
    WebGl2,
}

/// Operations admitted by Netrender's current adapter, rather than every
/// operation the upstream renderer may eventually support.
/// Sparse capabilities describe the resource-free lowerers; owned sessions
/// additionally admit their documented image/pattern/outline-text subsets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BackendCapabilities {
    pub solid_geometry: bool,
    pub gradients: bool,
    pub layers: bool,
    pub transforms: bool,
    pub clips: bool,
    pub nested_layers: bool,
    pub images: bool,
    pub patterns: bool,
    pub text: bool,
    pub element_filters: bool,
    pub backdrop_blur: bool,
    pub backdrop_color_filters: bool,
    /// The backend packet has a native composition operation.
    pub native_scene_append: bool,
    /// Netrender's registry-bearing renderer is connected to that operation.
    /// The free Classic `scene_to_vello` translator cannot resolve fragments.
    pub retained_fragments: bool,
    pub gpu_output: bool,
    pub cpu_output: bool,
}

impl VelloBackend {
    /// Whether this backend's adapter was included in the current build.
    pub const fn is_compiled(self) -> bool {
        match self {
            Self::Classic => true,
            Self::Hybrid => cfg!(feature = "vello-hybrid"),
            Self::Cpu => cfg!(feature = "vello-cpu"),
            Self::WebGl2 => cfg!(all(target_arch = "wasm32", feature = "vello-webgl")),
        }
    }

    pub const fn capabilities(self) -> BackendCapabilities {
        match self {
            Self::Classic => BackendCapabilities {
                solid_geometry: true,
                gradients: true,
                layers: true,
                transforms: true,
                clips: true,
                nested_layers: true,
                images: true,
                patterns: true,
                text: true,
                element_filters: true,
                backdrop_blur: true,
                backdrop_color_filters: false,
                native_scene_append: true,
                retained_fragments: true,
                gpu_output: true,
                cpu_output: false,
            },
            Self::Hybrid | Self::WebGl2 => BackendCapabilities {
                solid_geometry: true,
                gradients: true,
                layers: true,
                transforms: true,
                clips: true,
                nested_layers: true,
                images: false,
                patterns: false,
                text: false,
                element_filters: false,
                backdrop_blur: false,
                backdrop_color_filters: false,
                native_scene_append: true,
                retained_fragments: false,
                gpu_output: true,
                cpu_output: false,
            },
            Self::Cpu => BackendCapabilities {
                solid_geometry: true,
                gradients: true,
                layers: true,
                transforms: true,
                clips: true,
                nested_layers: true,
                images: false,
                patterns: false,
                text: false,
                element_filters: false,
                backdrop_blur: false,
                backdrop_color_filters: false,
                native_scene_append: false,
                retained_fragments: false,
                gpu_output: false,
                cpu_output: true,
            },
        }
    }
}

/// A scene cannot be lowered faithfully by the selected backend adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackendAdmissionError {
    /// Sparse scenes must have a nonempty viewport.
    EmptyViewport {
        backend: VelloBackend,
        width: u32,
        height: u32,
    },
    /// Sparse dimensions exceed the selected provider's internal rounding limit.
    ViewportTooLarge {
        backend: VelloBackend,
        width: u32,
        height: u32,
    },
    /// An operation is outside the adapter's currently verified subset.
    UnsupportedOperation {
        backend: VelloBackend,
        op_index: usize,
        operation: &'static str,
        reason: &'static str,
    },
    /// A primitive references a transform absent from the scene palette.
    InvalidTransform {
        backend: VelloBackend,
        op_index: usize,
        transform_id: u32,
    },
    /// Layer pushes and pops are not balanced.
    UnbalancedLayers {
        backend: VelloBackend,
        op_index: Option<usize>,
    },
}

impl std::fmt::Display for BackendAdmissionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyViewport {
                backend,
                width,
                height,
            } => write!(
                f,
                "{backend:?} sparse Vello viewport {width}x{height} is empty"
            ),
            Self::ViewportTooLarge {
                backend,
                width,
                height,
            } => {
                let (max_width, max_height) = sparse_dimension_limits(*backend);
                write!(
                    f,
                    "{backend:?} sparse Vello viewport {width}x{height} exceeds the width/height limits {max_width}x{max_height}"
                )
            }
            Self::UnsupportedOperation {
                backend,
                op_index,
                operation,
                reason,
            } => write!(
                f,
                "{backend:?} cannot admit scene op {op_index} ({operation}): {reason}"
            ),
            Self::InvalidTransform {
                backend,
                op_index,
                transform_id,
            } => write!(
                f,
                "{backend:?} scene op {op_index} references missing transform {transform_id}"
            ),
            Self::UnbalancedLayers { backend, op_index } => match op_index {
                Some(op_index) => write!(
                    f,
                    "{backend:?} scene layers are unbalanced at op {op_index}"
                ),
                None => write!(f, "{backend:?} scene layers are unbalanced at end of scene"),
            },
        }
    }
}

impl std::error::Error for BackendAdmissionError {}

// Vello Common 0.3 rounds each nonempty viewport up to Tile::WIDTH/HEIGHT
// (both four) in CommandRecorder::new. The rounded size must still fit u16:
// 65533..=65535 fit u16 themselves but panic during the provider's tile snap.
const SPARSE_MAX_VIEWPORT_DIMENSION: u32 = (u16::MAX as u32 / 4) * 4;
// CPU 0.3's depth buffer constructs complete 128-pixel bucket spans before
// clipping to the viewport. Its last bucket must also fit u16; otherwise
// BucketRange::span overflows even for a one-row viewport.
const SPARSE_MAX_CPU_WIDTH: u32 = (u16::MAX as u32 / 128) * 128;

const fn sparse_dimension_limits(backend: VelloBackend) -> (u32, u32) {
    let width = match backend {
        VelloBackend::Cpu => SPARSE_MAX_CPU_WIDTH,
        _ => SPARSE_MAX_VIEWPORT_DIMENSION,
    };
    (width, SPARSE_MAX_VIEWPORT_DIMENSION)
}

fn sparse_dimensions(
    backend: VelloBackend,
    scene: &crate::scene::Scene,
) -> Result<(u16, u16), BackendAdmissionError> {
    let (width, height) = (scene.viewport_width, scene.viewport_height);
    if width == 0 || height == 0 {
        return Err(BackendAdmissionError::EmptyViewport {
            backend,
            width,
            height,
        });
    }
    let (max_width, max_height) = sparse_dimension_limits(backend);
    if width > max_width || height > max_height {
        return Err(BackendAdmissionError::ViewportTooLarge {
            backend,
            width,
            height,
        });
    }
    Ok((width as u16, height as u16))
}

/// Validate the operation-level scene semantics supported by `backend`.
///
/// This checks viewport shape, operation kinds, transform references, and
/// layer balance. It deliberately cannot prove Classic registry state for
/// external images or retained fragments; resource-bearing Classic scenes
/// still require the registry-bearing [`crate::Renderer`] path. Callers choose
/// the backend-specific lowering and execution path after this preflight.
pub fn validate_scene_for_backend(
    backend: VelloBackend,
    scene: &crate::scene::Scene,
) -> Result<(), BackendAdmissionError> {
    validate_scene_operations(backend, scene, false)
}

fn validate_scene_operations(
    backend: VelloBackend,
    scene: &crate::scene::Scene,
    owned_resources: bool,
) -> Result<(), BackendAdmissionError> {
    use crate::scene::{SceneFilter, SceneOp};

    if backend != VelloBackend::Classic {
        sparse_dimensions(backend, scene)?;
    }

    let mut depth = 0usize;
    for (op_index, op) in scene.ops.iter().enumerate() {
        let transform_id = match op {
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
        };
        if transform_id.is_some_and(|id| id as usize >= scene.transforms.len()) {
            return Err(BackendAdmissionError::InvalidTransform {
                backend,
                op_index,
                transform_id: transform_id.expect("checked Some transform"),
            });
        }

        match op {
            SceneOp::Image(_) if backend != VelloBackend::Classic && !owned_resources => {
                return Err(unsupported(
                    backend,
                    op_index,
                    "Image",
                    "image resources require an owned CpuSession or HybridSession",
                ));
            }
            SceneOp::Pattern(_) if backend != VelloBackend::Classic && !owned_resources => {
                return Err(unsupported(
                    backend,
                    op_index,
                    "Pattern",
                    "pattern resources require an owned CpuSession or HybridSession",
                ));
            }
            SceneOp::GlyphRun(_) if backend != VelloBackend::Classic && !owned_resources => {
                return Err(unsupported(
                    backend,
                    op_index,
                    "GlyphRun",
                    "text resources require an owned CpuSession or HybridSession",
                ));
            }
            SceneOp::Fragment(_) if backend != VelloBackend::Classic => {
                return Err(unsupported(
                    backend,
                    op_index,
                    "Fragment",
                    "fragment registries are backend-owned; only Hybrid append has been proven upstream",
                ));
            }
            SceneOp::PushLayer(layer)
                if backend != VelloBackend::Classic
                    && (layer.backdrop_filter.is_some() || !layer.filters.is_empty()) =>
            {
                return Err(unsupported(
                    backend,
                    op_index,
                    "PushLayer",
                    "Netrender filter graphs have not been mapped to sparse filters",
                ));
            }
            SceneOp::PushLayer(layer)
                if backend == VelloBackend::Classic
                    && layer
                        .backdrop_filter
                        .is_some_and(|filter| !matches!(filter, SceneFilter::Blur(_))) =>
            {
                return Err(unsupported(
                    backend,
                    op_index,
                    "PushLayer",
                    "Classic backdrop preprocessing currently supports Blur only",
                ));
            }
            SceneOp::PushLayer(_) => depth += 1,
            SceneOp::PopLayer if depth == 0 => {
                return Err(BackendAdmissionError::UnbalancedLayers {
                    backend,
                    op_index: Some(op_index),
                });
            }
            SceneOp::PopLayer => depth -= 1,
            _ => {}
        }
    }
    if depth != 0 {
        return Err(BackendAdmissionError::UnbalancedLayers {
            backend,
            op_index: None,
        });
    }
    Ok(())
}

fn unsupported(
    backend: VelloBackend,
    op_index: usize,
    operation: &'static str,
    reason: &'static str,
) -> BackendAdmissionError {
    BackendAdmissionError::UnsupportedOperation {
        backend,
        op_index,
        operation,
        reason,
    }
}

#[cfg(any(
    feature = "vello-cpu",
    feature = "vello-hybrid",
    feature = "vello-webgl"
))]
mod sparse;

#[cfg(any(
    feature = "vello-cpu",
    feature = "vello-hybrid",
    feature = "vello-webgl"
))]
mod fragments;
#[cfg(any(
    feature = "vello-cpu",
    feature = "vello-hybrid",
    feature = "vello-webgl"
))]
mod sessions;
#[cfg(any(
    feature = "vello-cpu",
    feature = "vello-hybrid",
    feature = "vello-webgl"
))]
pub use fragments::{SparseFragmentLimits, SparseFragmentRegistry, SparseFragmentStats};
#[cfg(any(
    feature = "vello-cpu",
    feature = "vello-hybrid",
    feature = "vello-webgl"
))]
pub use sessions::{
    SparseResourceLimits, SparseResourceStats, SparseSessionError, SparseTextLimits,
    SparseTextStats,
};
#[cfg(feature = "vello-cpu")]
pub use sessions::CpuSession;
#[cfg(feature = "vello-hybrid")]
pub use sessions::HybridSession;

/// Lower a Netrender scene into Vello CPU's stateful render context.
#[cfg(feature = "vello-cpu")]
pub fn scene_to_vello_cpu(
    scene: &crate::scene::Scene,
) -> Result<vello_cpu::RenderContext, BackendAdmissionError> {
    let (width, height) = sparse::dimensions(scene, VelloBackend::Cpu)?;
    let mut context = vello_cpu::RenderContext::new(width, height);
    sparse::lower(&mut context, scene, VelloBackend::Cpu)?;
    context.flush();
    Ok(context)
}

/// Lower a Netrender scene into Vello Hybrid's sparse scene packet.
#[cfg(feature = "vello-hybrid")]
pub fn scene_to_vello_hybrid(
    scene: &crate::scene::Scene,
) -> Result<vello_hybrid::Scene, BackendAdmissionError> {
    let (width, height) = sparse::dimensions(scene, VelloBackend::Hybrid)?;
    let mut context = vello_hybrid::Scene::new(width, height);
    sparse::lower(&mut context, scene, VelloBackend::Hybrid)?;
    Ok(context)
}

#[cfg(test)]
mod tests;

#[cfg(all(target_arch = "wasm32", feature = "vello-webgl"))]
pub use sessions::{WebGlSession, WebGlSessionInit, WebGlSessionError, WebGlPreparedFrame};
