// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Renderer-owned execution participation labels for the RG2b proof.
//!
//! These labels describe the boundary between a selected Vello realization
//! and the image graph. They are deliberately crate-private: they explain an
//! execution shape without creating a second public rendering API.

use crate::vello_backends::VelloBackend;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub(crate) enum RasterExecutionBoundary {
    /// Classic has already submitted its opaque raster work before graph work.
    OpaqueSubmission,
    /// Hybrid records its raster work into the graph-owned encoder.
    EncoderParticipation,
    /// CPU work has produced pixels which enter through a ready upload/import.
    ReadyUploadImport,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub(crate) struct RasterExecution {
    pub(crate) backend: VelloBackend,
    pub(crate) boundary: RasterExecutionBoundary,
}

#[allow(dead_code)]
impl RasterExecution {
    pub(crate) const fn classic() -> Self {
        Self {
            backend: VelloBackend::Classic,
            boundary: RasterExecutionBoundary::OpaqueSubmission,
        }
    }

    pub(crate) const fn hybrid() -> Self {
        Self {
            backend: VelloBackend::Hybrid,
            boundary: RasterExecutionBoundary::EncoderParticipation,
        }
    }

    pub(crate) const fn cpu() -> Self {
        Self {
            backend: VelloBackend::Cpu,
            boundary: RasterExecutionBoundary::ReadyUploadImport,
        }
    }

    pub(crate) const fn boundary_name(self) -> &'static str {
        match self.boundary {
            RasterExecutionBoundary::OpaqueSubmission => "opaque_submission",
            RasterExecutionBoundary::EncoderParticipation => "encoder_batch",
            RasterExecutionBoundary::ReadyUploadImport => "ready_upload_import",
        }
    }

    pub(crate) fn dump(self) -> String {
        format!(
            "rasterizer={:?} execution_boundary={}",
            self.backend,
            self.boundary_name()
        )
    }
}
